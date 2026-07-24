use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceIdentity {
    pub(crate) canonical_url: String,
    pub(crate) source_id: Option<String>,
}

pub(crate) fn source_identity(raw_url: &str, platform: &str) -> SourceIdentity {
    let Ok(mut url) = Url::parse(raw_url) else {
        return fallback(raw_url);
    };
    url.set_fragment(None);

    let segments = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let identity = match platform {
        "小红书" => id_after(&segments, &["explore", "item"]).map(|id| {
            (
                id.clone(),
                format!("https://www.xiaohongshu.com/explore/{id}"),
            )
        }),
        "YouTube" => youtube_id(&url, &segments)
            .map(|id| (id.clone(), format!("https://www.youtube.com/watch?v={id}"))),
        "今日头条" => id_after(&segments, &["article"])
            .map(|id| (id.clone(), format!("https://www.toutiao.com/article/{id}/"))),
        "哔哩哔哩" => segments
            .iter()
            .find(|value| value.starts_with("BV") || value.starts_with("av"))
            .map(|id| {
                (
                    (*id).to_string(),
                    format!("https://www.bilibili.com/video/{id}/"),
                )
            }),
        "知乎" => zhihu_identity(&segments),
        "微博" => weibo_identity(&segments, &url),
        "抖音" => id_after(&segments, &["video"])
            .map(|id| (id.clone(), format!("https://www.douyin.com/video/{id}"))),
        "X" => id_after(&segments, &["status"]).map(|id| {
            let author = segments.first().copied().unwrap_or("i");
            (id.clone(), format!("https://x.com/{author}/status/{id}"))
        }),
        "Instagram" => instagram_identity(&segments),
        "豆瓣" => douban_identity(&url, &segments),
        "高德地图" => id_after(&segments, &["place"])
            .map(|id| (id.clone(), format!("https://www.amap.com/place/{id}")))
            .or_else(|| map_identity(&url, "https://www.amap.com/place/")),
        "百度地图" => map_identity(&url, "https://map.baidu.com/?uid="),
        "京东" => jd_identity(&url, &segments),
        "淘宝" => commerce_query_identity(&url, "https://item.taobao.com/item.htm?id="),
        "天猫" => commerce_query_identity(&url, "https://detail.tmall.com/item.htm?id="),
        "微信读书" => weread_identity(&url, &segments),
        _ => None,
    };

    if let Some((source_id, canonical_url)) = identity {
        return SourceIdentity {
            canonical_url,
            source_id: Some(source_id),
        };
    }

    strip_tracking(&mut url);
    SourceIdentity {
        canonical_url: url.to_string(),
        source_id: None,
    }
}

fn youtube_id(url: &Url, segments: &[&str]) -> Option<String> {
    if url
        .host_str()
        .is_some_and(|host| host.ends_with("youtu.be"))
    {
        return segments.first().map(|value| (*value).to_string());
    }
    query_value(url, &["v"]).or_else(|| id_after(segments, &["shorts", "live", "embed"]))
}

fn zhihu_identity(segments: &[&str]) -> Option<(String, String)> {
    if let Some(id) = id_after(segments, &["answer"]) {
        let question = id_after(segments, &["question"])?;
        return Some((
            format!("answer:{id}"),
            format!("https://www.zhihu.com/question/{question}/answer/{id}"),
        ));
    }
    id_after(segments, &["p"]).map(|id| {
        (
            format!("article:{id}"),
            format!("https://zhuanlan.zhihu.com/p/{id}"),
        )
    })
}

fn weibo_identity(segments: &[&str], url: &Url) -> Option<(String, String)> {
    let id =
        id_after(segments, &["status"]).or_else(|| segments.get(1).map(|id| (*id).to_string()))?;
    let author = segments.first().copied().unwrap_or("u");
    let host = if url
        .host_str()
        .is_some_and(|host| host.ends_with("weibo.cn"))
    {
        "weibo.cn"
    } else {
        "weibo.com"
    };
    Some((id.clone(), format!("https://{host}/{author}/{id}")))
}

fn instagram_identity(segments: &[&str]) -> Option<(String, String)> {
    for kind in ["p", "reel", "tv"] {
        if let Some(id) = id_after(segments, &[kind]) {
            return Some((
                id.clone(),
                format!("https://www.instagram.com/{kind}/{id}/"),
            ));
        }
    }
    None
}

fn douban_identity(url: &Url, segments: &[&str]) -> Option<(String, String)> {
    for kind in ["subject", "review", "note"] {
        if let Some(id) = id_after(segments, &[kind]) {
            let host = url.host_str().unwrap_or("www.douban.com");
            return Some((
                format!("{kind}:{id}"),
                format!("https://{host}/{kind}/{id}/"),
            ));
        }
    }
    None
}

fn map_identity(url: &Url, prefix: &str) -> Option<(String, String)> {
    let id = query_value(url, &["id", "uid", "poiid", "poiShareUid"])
        .or_else(|| nested_query_value(url, &["uid", "poiid", "poiShareUid"]))?;
    Some((id.clone(), format!("{prefix}{id}")))
}

fn nested_query_value(url: &Url, keys: &[&str]) -> Option<String> {
    url.query_pairs().find_map(|(_, value)| {
        value.split('&').find_map(|part| {
            let (key, value) = part.split_once('=')?;
            (keys.contains(&key) && !value.is_empty()).then(|| value.to_string())
        })
    })
}

fn jd_identity(_url: &Url, segments: &[&str]) -> Option<(String, String)> {
    let id = segments
        .iter()
        .find_map(|segment| segment.strip_suffix(".html"))
        .filter(|value| value.chars().all(|ch| ch.is_ascii_digit()))?
        .to_string();
    Some((id.clone(), format!("https://item.jd.com/{id}.html")))
}

fn commerce_query_identity(url: &Url, prefix: &str) -> Option<(String, String)> {
    let id = query_value(url, &["id", "itemId"])?;
    Some((id.clone(), format!("{prefix}{id}")))
}

fn weread_identity(url: &Url, segments: &[&str]) -> Option<(String, String)> {
    let id =
        id_after(segments, &["bookDetail"]).or_else(|| query_value(url, &["bookId", "bookid"]))?;
    Some((
        id.clone(),
        format!("https://weread.qq.com/web/bookDetail/{id}"),
    ))
}

fn id_after(segments: &[&str], markers: &[&str]) -> Option<String> {
    segments.windows(2).find_map(|pair| {
        markers
            .contains(&pair[0])
            .then(|| pair[1].trim_matches('/').to_string())
            .filter(|value| !value.is_empty())
    })
}

fn query_value(url: &Url, keys: &[&str]) -> Option<String> {
    url.query_pairs()
        .find(|(key, value)| keys.contains(&key.as_ref()) && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

fn strip_tracking(url: &mut Url) {
    let kept = url
        .query_pairs()
        .filter(|(key, _)| {
            !matches!(
                key.as_ref(),
                "utm_source"
                    | "utm_medium"
                    | "utm_campaign"
                    | "utm_term"
                    | "utm_content"
                    | "share_source"
                    | "share_token"
                    | "spm"
                    | "from"
                    | "log_from"
            )
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    if !kept.is_empty() {
        url.query_pairs_mut().extend_pairs(kept);
    }
}

fn fallback(raw_url: &str) -> SourceIdentity {
    SourceIdentity {
        canonical_url: raw_url.to_string(),
        source_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::source_identity;

    #[test]
    fn canonicalizes_platform_urls() {
        let cases = [
            (
                "https://www.youtube.com/watch?v=abc123&utm_source=share",
                "YouTube",
                "https://www.youtube.com/watch?v=abc123",
                "abc123",
            ),
            (
                "https://x.com/OpenAI/status/12345?s=20",
                "X",
                "https://x.com/OpenAI/status/12345",
                "12345",
            ),
            (
                "https://item.jd.com/100012345678.html?utm_source=share",
                "京东",
                "https://item.jd.com/100012345678.html",
                "100012345678",
            ),
            (
                "https://detail.tmall.com/item.htm?id=123&spm=a1",
                "天猫",
                "https://detail.tmall.com/item.htm?id=123",
                "123",
            ),
            (
                "https://map.baidu.com/?newmap=1&s=inf%26uid%3D435d7aea036e54355abbbcc8",
                "百度地图",
                "https://map.baidu.com/?uid=435d7aea036e54355abbbcc8",
                "435d7aea036e54355abbbcc8",
            ),
        ];

        for (raw, platform, expected_url, expected_id) in cases {
            let identity = source_identity(raw, platform);
            assert_eq!(identity.canonical_url, expected_url);
            assert_eq!(identity.source_id.as_deref(), Some(expected_id));
        }
    }
}
