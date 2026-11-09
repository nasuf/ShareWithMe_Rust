//! Platform adapters.
//!
//! Every platform with dedicated support has one module here that owns all of
//! its source-specific knowledge: canonical identity, page parsing, public
//! APIs, login/verification shells and refresh policy. The generic pipeline in
//! [`crate::extraction`] calls these hooks through [`Platform`] and never
//! matches on platform names itself.
//!
//! Adding a platform: create `platforms/<name>.rs`, add a variant below, and
//! wire the hooks it needs in the `match` arms of this file.

mod amap;
mod baidu_maps;
mod bilibili;
mod detect;
mod douban;
mod douyin;
mod identity;
mod instagram;
mod jd;
mod maps;
mod taobao;
mod toutiao;
pub(crate) mod wechat;
mod weibo;
mod weread;
mod x;
pub(crate) mod xiaohongshu;
mod youtube;
mod zhihu;

#[cfg(test)]
mod fixture_tests;

use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use url::Url;

use crate::{extraction::PlatformMetadata, models::LinkMetadata};

pub(crate) use detect::platform_for_url;
pub(crate) use identity::{SourceIdentity, source_identity};

/// A platform with dedicated extraction support. Everything else is `Generic`
/// and goes through the plain HTML/structured-data path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Xiaohongshu,
    YouTube,
    Toutiao,
    Bilibili,
    Zhihu,
    Weibo,
    Douyin,
    X,
    Instagram,
    Douban,
    Amap,
    BaiduMaps,
    Jd,
    Taobao,
    Tmall,
    WeRead,
    WeChat,
    Generic,
}

/// Static policy for one platform.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlatformProfile {
    /// Stable adapter id stored in `extraction.adapter`.
    pub(crate) adapter: &'static str,
    /// What a saved item from this platform is (post, video, article, ...).
    pub(crate) content_kind: &'static str,
    /// How long an incomplete snapshot waits before it is retried.
    refresh_hours: i64,
}

impl PlatformProfile {
    pub(crate) fn refresh_after(self, fetched_at: DateTime<Utc>) -> DateTime<Utc> {
        fetched_at + Duration::hours(self.refresh_hours)
    }
}

const fn profile(
    adapter: &'static str,
    content_kind: &'static str,
    refresh_hours: i64,
) -> PlatformProfile {
    PlatformProfile {
        adapter,
        content_kind,
        refresh_hours,
    }
}

/// Shared mobile Safari user agent. Several platforms serve their real page
/// only to a phone browser and a login or verification shell to anything else.
pub(crate) const MOBILE_SAFARI_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) \
     AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";

/// Crawler user agent some platforms answer with a server-rendered preview.
pub(crate) const SEARCH_CRAWLER_UA: &str =
    "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";

impl Platform {
    /// Resolve the display label stored on items (see [`platform_for_url`]).
    pub(crate) fn from_label(label: &str) -> Self {
        match label {
            "小红书" => Self::Xiaohongshu,
            "YouTube" => Self::YouTube,
            "今日头条" => Self::Toutiao,
            "哔哩哔哩" => Self::Bilibili,
            "知乎" => Self::Zhihu,
            "微博" => Self::Weibo,
            "抖音" => Self::Douyin,
            "X" => Self::X,
            "Instagram" => Self::Instagram,
            "豆瓣" => Self::Douban,
            "高德地图" => Self::Amap,
            "百度地图" => Self::BaiduMaps,
            "京东" => Self::Jd,
            "淘宝" => Self::Taobao,
            "天猫" => Self::Tmall,
            "微信读书" => Self::WeRead,
            "微信公众号" => Self::WeChat,
            _ => Self::Generic,
        }
    }

    pub(crate) fn of(metadata: &LinkMetadata) -> Self {
        Self::from_label(&metadata.platform)
    }

    pub(crate) fn profile(self) -> PlatformProfile {
        match self {
            Self::Xiaohongshu => profile("xiaohongshu", "post", 12),
            Self::YouTube => profile("youtube", "video", 24),
            Self::Toutiao => profile("toutiao", "article", 12),
            Self::Bilibili => profile("bilibili", "video", 24),
            Self::Zhihu => profile("zhihu", "article", 24),
            Self::Weibo => profile("weibo", "post", 6),
            Self::Douyin => profile("douyin", "video", 12),
            Self::X => profile("x", "post", 6),
            Self::Instagram => profile("instagram", "post", 12),
            Self::Douban => profile("douban", "review", 72),
            Self::Amap => profile("amap", "place", 24),
            Self::BaiduMaps => profile("baidu_maps", "place", 24),
            Self::Jd => profile("jd", "product", 12),
            Self::Taobao => profile("taobao", "product", 12),
            Self::Tmall => profile("tmall", "product", 12),
            Self::WeRead => profile("weread", "book", 72),
            Self::WeChat => profile("wechat_oa", "article", 24),
            Self::Generic => profile("generic", "link", 72),
        }
    }

    /// Stable content ID and canonical URL, when the URL identifies one item.
    pub(crate) fn identity(self, url: &Url, segments: &[&str]) -> Option<(String, String)> {
        match self {
            Self::Xiaohongshu => xiaohongshu::identity(segments),
            Self::YouTube => youtube::identity(url, segments),
            Self::Toutiao => toutiao::identity(segments),
            Self::Bilibili => bilibili::identity(segments),
            Self::Zhihu => zhihu::identity(segments),
            Self::Weibo => weibo::identity(url, segments),
            Self::Douyin => douyin::identity(segments),
            Self::X => x::identity(segments),
            Self::Instagram => instagram::identity(segments),
            Self::Douban => douban::identity(url, segments),
            Self::Amap => amap::identity(url, segments),
            Self::BaiduMaps => baidu_maps::identity(url),
            Self::Jd => jd::identity(segments),
            Self::Taobao => taobao::identity(url, taobao::TAOBAO_ITEM_PREFIX),
            Self::Tmall => taobao::identity(url, taobao::TMALL_ITEM_PREFIX),
            Self::WeRead => weread::identity(url, segments),
            Self::WeChat => wechat::identity(url, segments),
            Self::Generic => None,
        }
    }

    /// User agent for the initial page request, when the default one is
    /// answered with a shell.
    pub(crate) fn fetch_user_agent(self) -> Option<&'static str> {
        matches!(self, Self::Xiaohongshu | Self::WeChat).then_some(MOBILE_SAFARI_UA)
    }

    /// Parse platform-specific page data (embedded state, article DOM). Returns
    /// the extraction method when the page was recognised, so the generic
    /// Open Graph/HTML path is skipped.
    pub(crate) fn parse_page(
        self,
        metadata: &mut LinkMetadata,
        html: &str,
    ) -> Option<&'static str> {
        let parsed = match self {
            Self::Xiaohongshu => xiaohongshu::merge(metadata, html),
            Self::WeChat => return wechat::merge(metadata, html).then_some("wechat-article"),
            _ => self.merge_embedded_state(metadata, html),
        };
        parsed.then_some("embedded-state")
    }

    /// Merge metadata from a page's embedded JSON state (also used for the
    /// public preview pages some platforms serve to crawlers).
    pub(crate) fn merge_embedded_state(self, metadata: &mut LinkMetadata, html: &str) -> bool {
        let state = match self {
            Self::Douyin => douyin::embedded_state(html),
            Self::Bilibili => bilibili::embedded_state(html),
            Self::YouTube => youtube::embedded_state(html),
            Self::Zhihu => zhihu::embedded_state(html),
            Self::Weibo => weibo::embedded_state(html),
            Self::Instagram => instagram::embedded_state(html),
            _ => None,
        };
        let Some(state) = state.filter(PlatformMetadata::is_meaningful) else {
            return false;
        };
        crate::extraction::merge_platform_metadata(metadata, state);
        true
    }

    /// Enrich from structured data (JSON-LD, page state) in the fetched HTML.
    /// Runs after page parsing, for every platform.
    pub(crate) fn enrich_from_html(self, metadata: &mut LinkMetadata, html: &str) {
        use crate::extraction::structured_data as structured;
        let values = crate::extraction::json_ld_values(html);
        match self {
            Self::Jd => jd::enrich_from_html(metadata, html, &values),
            Self::Taobao | Self::Tmall => taobao::enrich_from_html(metadata, &values),
            Self::Douban => structured::enrich_catalog(metadata, &values),
            Self::WeRead => weread::enrich_from_html(metadata, html, &values),
            Self::Amap | Self::BaiduMaps => structured::enrich_place(metadata, &values),
            _ => structured::enrich_common(metadata, &values),
        }
    }

    /// Public API lookup that replaces the page content when it succeeds.
    pub(crate) async fn fetch_api(
        self,
        metadata: &LinkMetadata,
        http: &Client,
    ) -> Option<PlatformMetadata> {
        let fetch_for = async |raw_url: &str| match self {
            Self::Zhihu => zhihu::fetch_answer(http, raw_url).await,
            Self::Weibo => weibo::fetch_status(http, raw_url).await,
            _ => None,
        };
        match fetch_for(&metadata.source_url).await {
            Some(found) => Some(found),
            None => fetch_for(&metadata.final_url).await,
        }
    }

    /// Remote enrichment after the page: oEmbed, platform APIs, yt-dlp.
    pub(crate) async fn enrich_remote(
        self,
        metadata: &mut LinkMetadata,
        http: &Client,
        allow_video_transcription: bool,
    ) {
        match self {
            Self::YouTube => {
                youtube::enrich_remote(metadata, http, allow_video_transcription).await
            }
            Self::X => x::enrich_remote(metadata, http, allow_video_transcription).await,
            Self::Instagram => {
                instagram::enrich_remote(metadata, http, allow_video_transcription).await;
            }
            Self::Bilibili => {
                bilibili::enrich_remote(metadata, http, allow_video_transcription).await
            }
            Self::Douyin => douyin::enrich_remote(metadata, http, allow_video_transcription).await,
            Self::Douban => douban::enrich_remote(metadata, http).await,
            Self::Amap => amap::enrich_remote(metadata, http).await,
            Self::BaiduMaps => baidu_maps::enrich_remote(metadata, http).await,
            _ => {}
        }
    }

    /// Why the saved page is a login/visitor/verification shell instead of the
    /// content, if it is one.
    pub(crate) fn shell_reason(self, metadata: &LinkMetadata) -> Option<&'static str> {
        let text = metadata.original_content.plain_text.as_str();
        match self {
            Self::Weibo => weibo::shell_reason(text),
            Self::Douyin => douyin::shell_reason(text),
            Self::Instagram => instagram::shell_reason(text),
            Self::Douban => douban::shell_reason(&metadata.final_url),
            _ => None,
        }
    }

    /// What the user can do when no content could be extracted at all.
    pub(crate) fn missing_content_hint(self) -> Option<&'static str> {
        match self {
            Self::Taobao | Self::Tmall => Some(taobao::MISSING_CONTENT_HINT),
            Self::BaiduMaps => Some(baidu_maps::MISSING_CONTENT_HINT),
            Self::Instagram => Some(instagram::MISSING_CONTENT_HINT),
            _ => None,
        }
    }

    /// Platforms whose articles are usually only complete in a real browser.
    pub(crate) fn needs_browser_render(self) -> bool {
        matches!(self, Self::Toutiao | Self::Xiaohongshu | Self::WeChat)
    }
}

/// Referer that a media CDN accepts for hotlinked images. Several CDNs answer
/// a foreign Referer with an error or a placeholder image.
pub(crate) fn media_referer(host: &str) -> &'static str {
    let has = |domain: &str| host.contains(domain);
    if xiaohongshu::is_media_host(host) {
        "https://www.xiaohongshu.com/"
    } else if has("toutiao") || has("byteimg") {
        "https://www.toutiao.com/"
    } else if has("hdslb.com") || has("bilibili.com") {
        "https://www.bilibili.com/"
    } else if has("douyinpic.com") || has("douyinvod.com") {
        "https://www.douyin.com/"
    } else if has("zhimg.com") || has("zhihu.com") {
        "https://www.zhihu.com/"
    } else if has("sinaimg.cn") || has("weibo.com") {
        "https://weibo.com/"
    } else if has("doubanio.com") || has("douban.com") {
        "https://www.douban.com/"
    } else if has("instagram.com") || has("cdninstagram.com") {
        "https://www.instagram.com/"
    } else if has("twimg.com") {
        "https://x.com/"
    } else if has("ytimg.com") || has("googleusercontent.com") {
        "https://www.youtube.com/"
    } else if has("autonavi.com") || has("amap.com") {
        "https://www.amap.com/"
    } else if has("bdimg.com") || has("baidu.com") {
        "https://map.baidu.com/"
    } else if has("360buyimg.com") || has("jd.com") {
        "https://item.jd.com/"
    } else if has("alicdn.com") || has("taobao.com") {
        "https://www.taobao.com/"
    } else if wechat::is_media_host(host) {
        // mmbiz.qpic.cn answers a foreign Referer with a 200 "此图片来自微信公众平台
        // 未经允许不可引用" placeholder instead of the image.
        "https://mp.weixin.qq.com/"
    } else if has("weread.qq.com") || has("wrimg.com") {
        "https://weread.qq.com/"
    } else {
        "https://www.google.com/"
    }
}

#[cfg(test)]
mod tests {
    use super::{Platform, media_referer};

    #[test]
    fn every_dedicated_label_round_trips() {
        for label in [
            "小红书",
            "YouTube",
            "今日头条",
            "哔哩哔哩",
            "知乎",
            "微博",
            "抖音",
            "X",
            "Instagram",
            "豆瓣",
            "高德地图",
            "百度地图",
            "京东",
            "淘宝",
            "天猫",
            "微信读书",
            "微信公众号",
        ] {
            assert_ne!(Platform::from_label(label), Platform::Generic, "{label}");
        }
        assert_eq!(Platform::from_label("GitHub"), Platform::Generic);
        assert_eq!(Platform::Generic.profile().adapter, "generic");
    }

    #[test]
    fn uses_platform_referers_for_protected_media_hosts() {
        for (host, referer) in [
            ("i0.hdslb.com", "https://www.bilibili.com/"),
            ("p3-sign.douyinpic.com", "https://www.douyin.com/"),
            ("img.alicdn.com", "https://www.taobao.com/"),
            ("mmbiz.qpic.cn", "https://mp.weixin.qq.com/"),
            ("sns-webpic-qc.xhscdn.com", "https://www.xiaohongshu.com/"),
            ("example.com", "https://www.google.com/"),
        ] {
            assert_eq!(media_referer(host), referer, "{host}");
        }
    }
}
