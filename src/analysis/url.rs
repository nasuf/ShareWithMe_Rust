use regex::Regex;
use url::Url;

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
    Url::parse(&url).map_err(|_| ApiError::bad_request("invalid URL"))?;
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
        Url::parse(url).map_err(|_| ApiError::bad_request("invalid URL"))?;
    }
    Ok(urls)
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
        "xiaohongshu.com",
        "v.douyin.com",
        "douyin.com",
        "iesdouyin.com",
        "b23.tv",
        "bilibili.com",
        "v.kuaishou.com",
        "kuaishou.com",
        "gifshow.com",
        "t.cn",
        "weibo.com",
        "weibo.cn",
        "mp.weixin.qq.com",
        "toutiao.com",
        "snssdk.com",
        "m.tb.cn",
        "tb.cn",
        "taobao.com",
        "tmall.com",
        "u.jd.com",
        "jd.com",
        "yangkeduo.com",
        "pinduoduo.com",
        "smzdm.com",
        "zhihu.com",
        "zhuanlan.zhihu.com",
        "sspai.com",
        "juejin.cn",
        "csdn.net",
        "36kr.com",
        "ithome.com",
        "douban.com",
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
