//! Canonical identity of a saved link: a stable per-platform content ID plus a
//! clean canonical URL, used to merge duplicates and to refresh content.

use url::Url;

use super::Platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceIdentity {
    pub(crate) canonical_url: String,
    pub(crate) source_id: Option<String>,
}

pub(crate) fn source_identity(raw_url: &str, platform: &str) -> SourceIdentity {
    let Ok(mut url) = Url::parse(raw_url) else {
        return SourceIdentity {
            canonical_url: raw_url.to_string(),
            source_id: None,
        };
    };
    url.set_fragment(None);

    let segments = path_segments(&url);
    if let Some((source_id, canonical_url)) =
        Platform::from_label(platform).identity(&url, &segments)
    {
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

/// Non-empty path segments.
pub(super) fn path_segments(url: &Url) -> Vec<&str> {
    url.path_segments()
        .map(|segments| segments.filter(|value| !value.is_empty()).collect())
        .unwrap_or_default()
}

/// The segment right after the first segment equal to one of `markers`
/// (`/explore/<id>` -> `<id>`).
pub(super) fn id_after(segments: &[&str], markers: &[&str]) -> Option<String> {
    segments.windows(2).find_map(|pair| {
        markers
            .contains(&pair[0])
            .then(|| pair[1].trim_matches('/').to_string())
            .filter(|value| !value.is_empty())
    })
}

/// First non-empty query value for any of `keys`.
pub(super) fn query_value(url: &Url, keys: &[&str]) -> Option<String> {
    url.query_pairs()
        .find(|(key, value)| keys.contains(&key.as_ref()) && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// Like [`query_value`], but also looks inside query values that are
/// themselves `&`-joined key/value lists (`?s=inf%26uid%3D...`).
pub(super) fn nested_query_value(url: &Url, keys: &[&str]) -> Option<String> {
    url.query_pairs().find_map(|(_, value)| {
        value.split('&').find_map(|part| {
            let (key, value) = part.split_once('=')?;
            (keys.contains(&key) && !value.is_empty()).then(|| value.to_string())
        })
    })
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

#[cfg(test)]
mod tests {
    use super::source_identity;

    #[track_caller]
    fn assert_identity(raw: &str, platform: &str, expected_url: &str, expected_id: &str) {
        let identity = source_identity(raw, platform);
        assert_eq!(identity.canonical_url, expected_url, "{raw}");
        assert_eq!(identity.source_id.as_deref(), Some(expected_id), "{raw}");
    }

    #[test]
    fn canonicalizes_every_dedicated_platform() {
        let cases = [
            (
                "https://www.xiaohongshu.com/discovery/item/abc123?xsec_token=t",
                "小红书",
                "https://www.xiaohongshu.com/explore/abc123",
                "abc123",
            ),
            (
                "https://www.youtube.com/watch?v=abc123&utm_source=share",
                "YouTube",
                "https://www.youtube.com/watch?v=abc123",
                "abc123",
            ),
            (
                "https://youtu.be/abc123?si=x",
                "YouTube",
                "https://www.youtube.com/watch?v=abc123",
                "abc123",
            ),
            (
                "https://www.youtube.com/shorts/short1",
                "YouTube",
                "https://www.youtube.com/watch?v=short1",
                "short1",
            ),
            (
                "https://m.toutiao.com/article/7300/?app=news",
                "今日头条",
                "https://www.toutiao.com/article/7300/",
                "7300",
            ),
            (
                "https://m.bilibili.com/video/BV1epwvzGEKf?share=1",
                "哔哩哔哩",
                "https://www.bilibili.com/video/BV1epwvzGEKf/",
                "BV1epwvzGEKf",
            ),
            (
                "https://www.zhihu.com/question/11/answer/22?utm_medium=social",
                "知乎",
                "https://www.zhihu.com/question/11/answer/22",
                "answer:22",
            ),
            (
                "https://zhuanlan.zhihu.com/p/33",
                "知乎",
                "https://zhuanlan.zhihu.com/p/33",
                "article:33",
            ),
            (
                "https://m.weibo.cn/status/5000000000001",
                "微博",
                "https://weibo.cn/status/5000000000001",
                "5000000000001",
            ),
            (
                "https://weibo.com/123/AbCd?refer=x",
                "微博",
                "https://weibo.com/123/AbCd",
                "AbCd",
            ),
            (
                "https://www.douyin.com/video/7400?previous_page=app",
                "抖音",
                "https://www.douyin.com/video/7400",
                "7400",
            ),
            (
                "https://x.com/OpenAI/status/12345?s=20",
                "X",
                "https://x.com/OpenAI/status/12345",
                "12345",
            ),
            (
                "https://www.instagram.com/reel/Cabc/?igsh=x",
                "Instagram",
                "https://www.instagram.com/reel/Cabc/",
                "Cabc",
            ),
            (
                "https://www.amap.com/place/B000A7BD6C",
                "高德地图",
                "https://www.amap.com/place/B000A7BD6C",
                "B000A7BD6C",
            ),
            (
                "https://surl.amap.com/?id=B0FFG",
                "高德地图",
                "https://www.amap.com/place/B0FFG",
                "B0FFG",
            ),
            (
                "https://map.baidu.com/?newmap=1&s=inf%26uid%3D435d7aea036e54355abbbcc8",
                "百度地图",
                "https://map.baidu.com/?uid=435d7aea036e54355abbbcc8",
                "435d7aea036e54355abbbcc8",
            ),
            (
                "https://item.jd.com/100012345678.html?utm_source=share",
                "京东",
                "https://item.jd.com/100012345678.html",
                "100012345678",
            ),
            (
                "https://item.taobao.com/item.htm?id=987&spm=a1",
                "淘宝",
                "https://item.taobao.com/item.htm?id=987",
                "987",
            ),
            (
                "https://detail.tmall.com/item.htm?id=123&spm=a1",
                "天猫",
                "https://detail.tmall.com/item.htm?id=123",
                "123",
            ),
            (
                "https://weread.qq.com/web/bookDetail/abc?from=share",
                "微信读书",
                "https://weread.qq.com/web/bookDetail/abc",
                "abc",
            ),
            (
                "https://mp.weixin.qq.com/s/s4xrYzArVrkhEAfvvhR01Q?from=timeline",
                "微信公众号",
                "https://mp.weixin.qq.com/s/s4xrYzArVrkhEAfvvhR01Q",
                "s4xrYzArVrkhEAfvvhR01Q",
            ),
            (
                "https://mp.weixin.qq.com/s?__biz=MzA5&mid=2650&idx=1&sn=abcdef&chksm=xyz&scene=27",
                "微信公众号",
                "https://mp.weixin.qq.com/s?__biz=MzA5&mid=2650&idx=1&sn=abcdef",
                "abcdef",
            ),
            (
                "https://www.douban.com/doubanapp/dispatch/movie/35524447?dt_dapp=1",
                "豆瓣",
                "https://movie.douban.com/subject/35524447/",
                "subject:35524447",
            ),
            (
                "https://www.douban.com/doubanapp/dispatch/book/1084336?dt_dapp=1",
                "豆瓣",
                "https://book.douban.com/subject/1084336/",
                "subject:1084336",
            ),
            (
                "https://sec.douban.com/c?r=https%3A%2F%2Fmovie.douban.com%2Fsubject%2F35524447%2F%3Fdt_dapp%3D1&_s=abc&a=1",
                "豆瓣",
                "https://movie.douban.com/subject/35524447/",
                "subject:35524447",
            ),
            (
                "https://www.douban.com/review/42/",
                "豆瓣",
                "https://www.douban.com/review/42/",
                "review:42",
            ),
        ];
        for (raw, platform, expected_url, expected_id) in cases {
            assert_identity(raw, platform, expected_url, expected_id);
        }
    }

    #[test]
    fn generic_links_only_lose_tracking_params() {
        let identity = source_identity(
            "https://example.com/a?id=1&utm_source=x&spm=y#top",
            "example.com",
        );
        assert_eq!(identity.canonical_url, "https://example.com/a?id=1");
        assert_eq!(identity.source_id, None);
        let identity = source_identity("not a url", "未知来源");
        assert_eq!(identity.canonical_url, "not a url");
    }
}
