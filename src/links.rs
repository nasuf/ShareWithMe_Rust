//! Links in API requests: pull URLs out of share-sheet text and reject
//! destinations the server must not fetch (private, local, non-HTTP).

use regex::Regex;
use url::{Host, Url};

use crate::{error::ApiError, models::AnalyzeLinkRequest};

pub(crate) fn resolve_request_url(request: &AnalyzeLinkRequest) -> Result<String, ApiError> {
    let joined = [request.url.as_deref(), request.shared_text.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    let Some(url) = extract_first_url(&joined) else {
        return Err(ApiError::bad_request(
            "request must include an http, https, or supported app short URL",
        ));
    };
    let parsed = Url::parse(&url).map_err(|_| ApiError::bad_request("invalid URL"))?;
    validate_remote_url(&parsed)?;
    Ok(url)
}

pub(crate) fn resolve_request_urls(request: &AnalyzeLinkRequest) -> Result<Vec<String>, ApiError> {
    let joined = [request.url.as_deref(), request.shared_text.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    let urls = extract_urls(&joined);
    if urls.is_empty() {
        return Err(ApiError::bad_request(
            "request must include an http, https, or supported app short URL",
        ));
    }
    for url in &urls {
        let parsed = Url::parse(url).map_err(|_| ApiError::bad_request("invalid URL"))?;
        validate_remote_url(&parsed)?;
    }
    Ok(urls)
}

pub(crate) fn is_safe_remote_url(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    match url.host() {
        Some(Host::Ipv4(ip)) => is_public_ipv4(ip),
        Some(Host::Ipv6(ip)) => {
            if let Some(ipv4) = ip.to_ipv4() {
                return is_public_ipv4(ipv4);
            }
            let first = ip.segments()[0];
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || first & 0xfe00 == 0xfc00
                || first & 0xffc0 == 0xfe80)
        }
        Some(Host::Domain(host)) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            !(host == "localhost"
                || host.ends_with(".localhost")
                || host.ends_with(".local")
                || host.ends_with(".internal"))
        }
        None => false,
    }
}

fn validate_remote_url(url: &Url) -> Result<(), ApiError> {
    if is_safe_remote_url(url) {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "URL must use http/https and point to a public network host",
        ))
    }
}

fn is_public_ipv4(ip: std::net::Ipv4Addr) -> bool {
    if cfg!(test) && ip.is_loopback() {
        return true;
    }
    let octets = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || octets[0] == 0
        || octets[0] >= 224
        || (octets[0] == 100 && (64..=127).contains(&octets[1])))
}

pub(crate) fn extract_first_url(input: &str) -> Option<String> {
    extract_urls(input).into_iter().next()
}

pub(crate) fn extract_urls(input: &str) -> Vec<String> {
    let mut candidates = Vec::<(usize, String)>::new();

    if let Ok(full_url) = Regex::new(r#"https?://[^\s<>"'，。；：）】》]+"#) {
        for matched in full_url.find_iter(input) {
            candidates.push((matched.start(), clean_shared_url(matched.as_str())));
        }
    }

    if let Ok(bare_url) = Regex::new(r#"\bwww\.[^\s<>"'，。；：）】》]+"#) {
        for matched in bare_url.find_iter(input) {
            candidates.push((
                matched.start(),
                format!("https://{}", clean_shared_url(matched.as_str())),
            ));
        }
    }

    if let Ok(bare_app_url) =
        Regex::new(r#"(?i)\b[a-z0-9-]+(?:\.[a-z0-9-]+)+/[^\s<>"'，。；：）】》]+"#)
    {
        for matched in bare_app_url.find_iter(input) {
            let cleaned = clean_shared_url(matched.as_str());
            let host = cleaned.split('/').next().unwrap_or_default();
            if is_supported_bare_url_host(host) {
                candidates.push((matched.start(), format!("https://{cleaned}")));
            }
        }
    }

    candidates.sort_by_key(|(start, _)| *start);
    let mut urls = Vec::new();
    for (_, url) in candidates {
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    urls
}

fn is_supported_bare_url_host(host: &str) -> bool {
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    const HOSTS: &[&str] = &[
        "xhslink.com",
        "xhslink.cn",
        "xiaohongshu.com",
        "rednote.com",
        "v.douyin.com",
        "douyin.com",
        "iesdouyin.com",
        "b23.tv",
        "bilibili.com",
        "v.kuaishou.com",
        "kuaishou.com",
        "gifshow.com",
        "t.cn",
        "t.co",
        "weibo.com",
        "weibo.cn",
        "mp.weixin.qq.com",
        "toutiao.com",
        "snssdk.com",
        "m.tb.cn",
        "e.tb.cn",
        "tb.cn",
        "taobao.com",
        "tmall.com",
        "u.jd.com",
        "3.cn",
        "jd.com",
        "yangkeduo.com",
        "pinduoduo.com",
        "smzdm.com",
        "zhihu.com",
        "zhuanlan.zhihu.com",
        "twitter.com",
        "x.com",
        "sspai.com",
        "juejin.cn",
        "csdn.net",
        "36kr.com",
        "ithome.com",
        "douban.com",
        "weread.qq.com",
        "pan.baidu.com",
        "music.163.com",
        "y.qq.com",
        "ximalaya.com",
        "xiaoyuzhoufm.com",
        "notion.so",
        "notion.site",
        "yuque.com",
        "feishu.cn",
        "larksuite.com",
        "amap.com",
        "gaode.com",
        "surl.amap.com",
        "uri.amap.com",
        "map.baidu.com",
        "j.map.baidu.com",
        "maps.apple.com",
        "dianping.com",
        "meituan.com",
        "ctrip.com",
        "trip.com",
        "airbnb.com",
        "booking.com",
        "youtube.com",
        "youtu.be",
        "tiktok.com",
        "instagram.com",
        "instagr.am",
        "ig.me",
        "facebook.com",
        "fb.watch",
        "reddit.com",
        "linkedin.com",
        "pinterest.com",
        "github.com",
        "gitlab.com",
        "substack.com",
    ];

    HOSTS
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

fn clean_shared_url(input: &str) -> String {
    input
        .trim()
        .trim_matches(|ch: char| matches!(ch, '<' | '>' | '"' | '\'' | '“' | '”' | '‘' | '’'))
        .trim_end_matches(|ch: char| {
            matches!(
                ch,
                ',' | '.'
                    | ';'
                    | ':'
                    | ')'
                    | ']'
                    | '}'
                    | '，'
                    | '。'
                    | '；'
                    | '：'
                    | '）'
                    | '】'
                    | '》'
            )
        })
        .to_string()
}
