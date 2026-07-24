mod identity;

use chrono::{DateTime, Duration, Utc};

pub(crate) use identity::{SourceIdentity, source_identity};

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlatformProfile {
    pub(crate) adapter: &'static str,
    pub(crate) content_kind: &'static str,
    refresh_hours: i64,
}

impl PlatformProfile {
    pub(crate) fn refresh_after(self, fetched_at: DateTime<Utc>) -> DateTime<Utc> {
        fetched_at + Duration::hours(self.refresh_hours)
    }
}

pub(crate) fn profile_for(platform: &str) -> PlatformProfile {
    match platform {
        "小红书" => profile("xiaohongshu", "post", 12),
        "YouTube" => profile("youtube", "video", 24),
        "今日头条" => profile("toutiao", "article", 12),
        "哔哩哔哩" => profile("bilibili", "video", 24),
        "知乎" => profile("zhihu", "article", 24),
        "微博" => profile("weibo", "post", 6),
        "抖音" => profile("douyin", "video", 12),
        "X" => profile("x", "post", 6),
        "Instagram" => profile("instagram", "post", 12),
        "豆瓣" => profile("douban", "review", 72),
        "高德地图" => profile("amap", "place", 24),
        "百度地图" => profile("baidu_maps", "place", 24),
        "京东" => profile("jd", "product", 12),
        "淘宝" => profile("taobao", "product", 12),
        "天猫" => profile("tmall", "product", 12),
        "微信读书" => profile("weread", "book", 72),
        "微信公众号" => profile("wechat_oa", "article", 24),
        _ => profile("generic", "link", 72),
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
