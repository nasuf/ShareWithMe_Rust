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
    // Tests serve fixtures from 127.0.0.1; production never allows loopback.
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

#[cfg(test)]
mod tests {
    use super::{extract_first_url, extract_urls, is_safe_remote_url};

    #[test]
    fn extracts_http_and_bare_urls_from_shared_text() {
        assert_eq!(
            extract_first_url("复制这段内容后打开 https://xhslink.com/a1b2c3。"),
            Some("https://xhslink.com/a1b2c3".to_string())
        );
        assert_eq!(
            extract_first_url("看这个 www.example.com/path?x=1，挺有用"),
            Some("https://www.example.com/path?x=1".to_string())
        );
        assert_eq!(
            extract_first_url("98 小红书 xhslink.com/a1B2c3，复制本条信息"),
            Some("https://xhslink.com/a1B2c3".to_string())
        );
        assert_eq!(
            extract_first_url("2.33 v.douyin.com/iLxyz9/ 复制此链接"),
            Some("https://v.douyin.com/iLxyz9/".to_string())
        );
        assert_eq!(
            extract_first_url("这个视频不错 b23.tv/BV1234。"),
            Some("https://b23.tv/BV1234".to_string())
        );
        assert_eq!(
            extract_first_url("淘宝口令 m.tb.cn/h.abc123?tk=abcd CZ0001"),
            Some("https://m.tb.cn/h.abc123?tk=abcd".to_string())
        );
        assert_eq!(
            extract_first_url("京东分享 3.cn/abc123 复制打开"),
            Some("https://3.cn/abc123".to_string())
        );
        assert_eq!(
            extract_first_url("X 帖子 t.co/abc123"),
            Some("https://t.co/abc123".to_string())
        );
        assert_eq!(
            extract_first_url("高德位置 uri.amap.com/marker?position=116,39"),
            Some("https://uri.amap.com/marker?position=116,39".to_string())
        );
        assert_eq!(
            extract_first_url("资料在 pan.baidu.com/s/abc123 提取码 test"),
            Some("https://pan.baidu.com/s/abc123".to_string())
        );
        assert_eq!(extract_first_url("版本号 abc.def/ghi 不是链接"), None);
        assert_eq!(
            extract_urls("合集 xhslink.com/a1 b23.tv/BV1234 再看 www.example.com/a xhslink.com/a1"),
            [
                "https://xhslink.com/a1",
                "https://b23.tv/BV1234",
                "https://www.example.com/a"
            ]
        );
        assert_eq!(
            extract_urls("视频 youtu.be/abc123 问答 zhihu.com/question/1 微博 weibo.com/123/abc"),
            [
                "https://youtu.be/abc123",
                "https://zhihu.com/question/1",
                "https://weibo.com/123/abc"
            ]
        );
    }

    #[test]
    fn rejects_private_and_local_source_urls() {
        for url in [
            "http://localhost:8080/private",
            "http://192.168.1.1/admin",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/private",
            "file:///etc/passwd",
        ] {
            assert!(!is_safe_remote_url(&url::Url::parse(url).unwrap()), "{url}");
        }
        assert!(is_safe_remote_url(
            &url::Url::parse("https://www.xiaohongshu.com/explore/abc").unwrap()
        ));
    }
}
