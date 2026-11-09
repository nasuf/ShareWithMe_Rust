//! Map a URL to the display name of its source platform.
//!
//! The table is ordered: more specific hosts must come before broader ones
//! (`weread.qq.com` before `qq.com`, `map.baidu.com` before `baidu.com`).
//! Only some of these names have a dedicated adapter (see [`super::Platform`]);
//! the rest are labels for the generic HTML path and for classification.

use url::Url;

use super::xiaohongshu;

const UNKNOWN_PLATFORM: &str = "未知来源";

pub(crate) fn platform_for_url(raw_url: &str) -> String {
    let Ok(url) = Url::parse(raw_url) else {
        return UNKNOWN_PLATFORM.to_string();
    };
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.");
    platform_for_host(host).unwrap_or(host).to_string()
}

fn platform_for_host(host: &str) -> Option<&'static str> {
    let has = |domain: &str| host.contains(domain);
    let platform = match host {
        // xhslink.cn is the newer share short link; rednote.com is the
        // international domain overseas IPs (e.g. the server) are sent to.
        value if xiaohongshu::is_host(value) => "小红书",
        _ if has("toutiao.com") || has("snssdk.com") => "今日头条",
        _ if has("douyin.com") || has("iesdouyin.com") => "抖音",
        _ if has("kuaishou.com") || has("gifshow.com") => "快手",
        _ if has("bilibili.com") || has("b23.tv") => "哔哩哔哩",
        _ if has("zhihu.com") => "知乎",
        "t.cn" => "微博",
        _ if has("weibo.com") || has("weibo.cn") => "微博",
        _ if has("weread.qq.com") => "微信读书",
        _ if has("mp.weixin.qq.com") => "微信公众号",
        _ if has("weixin.qq.com") || has("qq.com") => "腾讯内容",
        _ if has("map.baidu.com") => "百度地图",
        _ if has("pan.baidu.com") => "百度网盘",
        _ if has("baidu.com") => "百度",
        _ if has("youtube.com") || has("youtu.be") => "YouTube",
        "x.com" | "t.co" => "X",
        value if has("twitter.com") || value.ends_with(".x.com") => "X",
        "instagr.am" | "ig.me" => "Instagram",
        _ if has("instagram.com") => "Instagram",
        _ if has("tiktok.com") => "TikTok",
        _ if has("facebook.com") || has("fb.watch") => "Facebook",
        _ if has("reddit.com") => "Reddit",
        _ if has("linkedin.com") => "LinkedIn",
        _ if has("pinterest.com") => "Pinterest",
        _ if has("github.com") => "GitHub",
        _ if has("gitlab.com") => "GitLab",
        _ if has("medium.com") => "Medium",
        _ if has("juejin.cn") => "掘金",
        _ if has("csdn.net") => "CSDN",
        _ if has("sspai.com") => "少数派",
        _ if has("36kr.com") => "36氪",
        _ if has("ithome.com") => "IT之家",
        _ if has("douban.com") => "豆瓣",
        _ if has("mafengwo.cn") => "马蜂窝",
        _ if has("trip.com") || has("ctrip.com") => "携程",
        _ if has("airbnb.") || has("booking.com") => "旅行住宿",
        _ if has("amap.com") || has("gaode.com") => "高德地图",
        _ if has("maps.apple.com") => "Apple Maps",
        _ if has("google.com/maps") || has("goo.gl/maps") => "Google Maps",
        _ if has("dianping.com") => "大众点评",
        _ if has("meituan.com") => "美团",
        _ if has("tmall.com") => "天猫",
        "m.tb.cn" | "e.tb.cn" | "tb.cn" => "淘宝",
        _ if has("taobao.com") => "淘宝",
        "u.jd.com" | "3.cn" => "京东",
        _ if has("jd.com") => "京东",
        _ if has("yangkeduo.com") || has("pinduoduo.com") => "拼多多",
        _ if has("smzdm.com") => "什么值得买",
        _ if has("amazon.") => "Amazon",
        _ if has("notion.site") || has("notion.so") => "Notion",
        _ if has("yuque.com") => "语雀",
        _ if has("feishu.cn") || has("larksuite.com") => "飞书",
        _ if has("docs.google.com") || has("drive.google.com") => "Google Docs/Drive",
        _ if has("dropbox.com") => "Dropbox",
        _ if has("icloud.com") => "iCloud",
        _ if has("spotify.com") => "Spotify",
        _ if has("music.apple.com") => "Apple Music",
        _ if has("music.163.com") => "网易云音乐",
        _ if has("y.qq.com") => "QQ音乐",
        _ if has("ximalaya.com") => "喜马拉雅",
        _ if has("xiaoyuzhoufm.com") => "小宇宙",
        _ if has("substack.com") => "Substack",
        _ => return None,
    };
    Some(platform)
}

#[cfg(test)]
mod tests {
    use super::platform_for_url;

    #[test]
    fn recognizes_every_supported_target_platform() {
        let cases = [
            ("https://www.xiaohongshu.com/explore/abc", "小红书"),
            ("https://xhslink.cn/m/abc", "小红书"),
            ("https://www.rednote.com/explore/abc", "小红书"),
            ("https://www.youtube.com/watch?v=abc", "YouTube"),
            ("https://youtu.be/abc", "YouTube"),
            ("https://www.toutiao.com/article/123/", "今日头条"),
            ("https://www.bilibili.com/video/BV1abc", "哔哩哔哩"),
            ("https://b23.tv/abc", "哔哩哔哩"),
            ("https://www.zhihu.com/question/1/answer/2", "知乎"),
            ("https://weibo.com/123/AbCd", "微博"),
            ("https://t.cn/abc", "微博"),
            ("https://www.douyin.com/video/123", "抖音"),
            ("https://x.com/user/status/1", "X"),
            ("https://twitter.com/user/status/1", "X"),
            ("https://www.instagram.com/p/abc/", "Instagram"),
            ("https://movie.douban.com/subject/1/", "豆瓣"),
            ("https://www.amap.com/place/B000", "高德地图"),
            ("https://map.baidu.com/?uid=abc", "百度地图"),
            ("https://item.jd.com/1.html", "京东"),
            ("https://3.cn/abc", "京东"),
            ("https://item.taobao.com/item.htm?id=1", "淘宝"),
            ("https://m.tb.cn/h.abc", "淘宝"),
            ("https://detail.tmall.com/item.htm?id=1", "天猫"),
            ("https://weread.qq.com/web/bookDetail/abc", "微信读书"),
            ("https://mp.weixin.qq.com/s/abc", "微信公众号"),
        ];
        for (url, expected) in cases {
            assert_eq!(platform_for_url(url), expected, "{url}");
        }
    }

    #[test]
    fn orders_specific_hosts_before_broad_ones() {
        assert_eq!(platform_for_url("https://news.qq.com/a/1"), "腾讯内容");
        assert_eq!(platform_for_url("https://pan.baidu.com/s/1"), "百度网盘");
        assert_eq!(platform_for_url("https://www.baidu.com/s?wd=1"), "百度");
        assert_eq!(platform_for_url("https://example.org/a"), "example.org");
        assert_eq!(platform_for_url("not a url"), "未知来源");
    }
}
