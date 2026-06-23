use regex::Regex;
use url::Url;

use crate::models::LinkMetadata;

pub(crate) fn platform_for_url(raw_url: &str) -> String {
    let Ok(url) = Url::parse(raw_url) else {
        return "未知来源".to_string();
    };
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.");
    let platform = match host {
        value if value.contains("xiaohongshu.com") || value.contains("xhslink.com") => "小红书",
        value if value.contains("toutiao.com") || value.contains("snssdk.com") => "今日头条",
        value if value.contains("douyin.com") || value.contains("iesdouyin.com") => "抖音",
        value if value.contains("kuaishou.com") || value.contains("gifshow.com") => "快手",
        value if value.contains("bilibili.com") || value.contains("b23.tv") => "哔哩哔哩",
        value if value.contains("zhihu.com") => "知乎",
        value if value.contains("weibo.com") || value.contains("weibo.cn") => "微博",
        value if value.contains("mp.weixin.qq.com") => "微信公众号",
        value if value.contains("weixin.qq.com") || value.contains("qq.com") => "腾讯内容",
        value if value.contains("pan.baidu.com") => "百度网盘",
        value if value.contains("baidu.com") || value.contains("baike.baidu.com") => "百度",
        value if value.contains("youtube.com") || value.contains("youtu.be") => "YouTube",
        value if value.contains("twitter.com") || value == "x.com" => "X",
        value if value.contains("instagram.com") => "Instagram",
        value if value.contains("tiktok.com") => "TikTok",
        value if value.contains("facebook.com") || value.contains("fb.watch") => "Facebook",
        value if value.contains("reddit.com") => "Reddit",
        value if value.contains("linkedin.com") => "LinkedIn",
        value if value.contains("pinterest.com") => "Pinterest",
        value if value.contains("github.com") => "GitHub",
        value if value.contains("gitlab.com") => "GitLab",
        value if value.contains("medium.com") => "Medium",
        value if value.contains("juejin.cn") => "掘金",
        value if value.contains("csdn.net") => "CSDN",
        value if value.contains("sspai.com") => "少数派",
        value if value.contains("36kr.com") => "36氪",
        value if value.contains("ithome.com") => "IT之家",
        value if value.contains("douban.com") => "豆瓣",
        value if value.contains("mafengwo.cn") => "马蜂窝",
        value if value.contains("trip.com") || value.contains("ctrip.com") => "携程",
        value if value.contains("airbnb.") || value.contains("booking.com") => "旅行住宿",
        value if value.contains("amap.com") || value.contains("gaode.com") => "高德地图",
        value if value.contains("maps.apple.com") => "Apple Maps",
        value if value.contains("google.com/maps") || value.contains("goo.gl/maps") => {
            "Google Maps"
        }
        value if value.contains("dianping.com") => "大众点评",
        value if value.contains("meituan.com") => "美团",
        value
            if value.contains("taobao.com")
                || value.contains("tmall.com")
                || value == "m.tb.cn"
                || value == "tb.cn" =>
        {
            "淘宝/天猫"
        }
        value if value.contains("jd.com") => "京东",
        value if value.contains("yangkeduo.com") || value.contains("pinduoduo.com") => "拼多多",
        value if value.contains("smzdm.com") => "什么值得买",
        value if value.contains("amazon.") => "Amazon",
        value if value.contains("notion.site") || value.contains("notion.so") => "Notion",
        value if value.contains("yuque.com") => "语雀",
        value if value.contains("feishu.cn") || value.contains("larksuite.com") => "飞书",
        value if value.contains("docs.google.com") || value.contains("drive.google.com") => {
            "Google Docs/Drive"
        }
        value if value.contains("dropbox.com") => "Dropbox",
        value if value.contains("icloud.com") => "iCloud",
        value if value.contains("pan.baidu.com") => "百度网盘",
        value if value.contains("spotify.com") => "Spotify",
        value if value.contains("music.apple.com") => "Apple Music",
        value if value.contains("music.163.com") => "网易云音乐",
        value if value.contains("y.qq.com") => "QQ音乐",
        value if value.contains("ximalaya.com") => "喜马拉雅",
        value if value.contains("xiaoyuzhoufm.com") => "小宇宙",
        value if value.contains("substack.com") => "Substack",
        _ => host,
    };
    platform.to_string()
}

pub(crate) fn infer_category(text: &str, platform: &str) -> String {
    let lower = text.to_lowercase();
    if matches!(platform, "知乎" | "微博") {
        let explicit_technical = [
            "rust",
            "flutter",
            "api",
            "github",
            "openai",
            "deepseek",
            "大模型",
            "代码",
            "编程",
            "开源",
        ]
        .iter()
        .any(|keyword| lower.contains(keyword));
        if !explicit_technical {
            return match platform {
                "知乎" => "知识问答".to_string(),
                "微博" => "社交动态".to_string(),
                _ => unreachable!(),
            };
        }
    }
    let rules = [
        (
            "技术",
            [
                "flutter",
                "rust",
                "api",
                "github",
                "gitlab",
                "openai",
                "deepseek",
                "人工智能",
                "大模型",
                "代码",
                "编程",
                "模型",
                "开源",
                "架构",
            ]
            .as_slice(),
        ),
        (
            "地点",
            ["地图", "地址", "导航", "大众点评", "美团", "高德", "maps"].as_slice(),
        ),
        (
            "美食",
            ["餐厅", "咖啡", "菜谱", "好吃", "探店", "烘焙"].as_slice(),
        ),
        (
            "旅行",
            [
                "旅行", "酒店", "攻略", "签证", "机票", "citywalk", "景点", "路线", "民宿",
            ]
            .as_slice(),
        ),
        (
            "购物",
            [
                "价格",
                "优惠",
                "下单",
                "淘宝",
                "京东",
                "拼多多",
                "种草",
                "购物车",
                "值得买",
            ]
            .as_slice(),
        ),
        (
            "财经",
            ["股票", "基金", "投资", "财报", "美联储", "市场"].as_slice(),
        ),
        (
            "健康",
            ["健身", "睡眠", "医疗", "营养", "跑步", "心理"].as_slice(),
        ),
        (
            "生活灵感",
            ["装修", "穿搭", "收纳", "灵感", "家居", "小红书"].as_slice(),
        ),
        (
            "影音娱乐",
            [
                "电影",
                "剧集",
                "音乐",
                "播客",
                "专辑",
                "spotify",
                "网易云",
                "小宇宙",
                "抖音",
                "哔哩哔哩",
                "bilibili",
                "youtube",
                "视频",
            ]
            .as_slice(),
        ),
        (
            "知识问答",
            ["知乎", "回答", "问题", "观点", "专栏", "问答"].as_slice(),
        ),
        (
            "社交动态",
            ["微博", "热搜", "博文", "转发", "评论"].as_slice(),
        ),
        (
            "文档资料",
            [
                "notion", "语雀", "飞书", "文档", "网盘", "drive", "dropbox", "资料",
            ]
            .as_slice(),
        ),
        (
            "新闻",
            ["今日头条", "新闻", "发布", "报道", "事件"].as_slice(),
        ),
    ];
    for (category, keywords) in rules {
        if keywords
            .iter()
            .any(|keyword| lower.contains(&keyword.to_lowercase()))
        {
            return category.to_string();
        }
    }
    match platform {
        "小红书" => "生活灵感".to_string(),
        "今日头条" => "新闻".to_string(),
        "抖音" | "哔哩哔哩" | "YouTube" => "影音娱乐".to_string(),
        "知乎" => "知识问答".to_string(),
        "微博" => "社交动态".to_string(),
        "GitHub" | "GitLab" | "掘金" | "CSDN" | "少数派" | "IT之家" => "技术".to_string(),
        "淘宝/天猫" | "京东" | "拼多多" | "Amazon" | "什么值得买" => {
            "购物".to_string()
        }
        "高德地图" | "Apple Maps" | "Google Maps" | "大众点评" | "美团" => {
            "地点".to_string()
        }
        "Spotify" | "Apple Music" | "网易云音乐" | "QQ音乐" | "喜马拉雅" | "小宇宙" => {
            "影音娱乐".to_string()
        }
        "Notion" | "语雀" | "飞书" | "Google Docs/Drive" | "Dropbox" | "iCloud" | "百度网盘" => {
            "文档资料".to_string()
        }
        _ => "待整理".to_string(),
    }
}

pub(super) fn extract_keywords(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    if let Ok(regex) = Regex::new(r"[A-Za-z][A-Za-z0-9_+\-.]{2,}|[\p{Han}]{2,8}") {
        for token in regex.find_iter(text) {
            let value = token.as_str().trim().to_string();
            if !is_stop_word(&value) && !words.contains(&value) {
                words.push(value);
            }
            if words.len() >= 12 {
                break;
            }
        }
    }
    words
}

pub(super) fn extract_entities(text: &str) -> Vec<String> {
    let mut entities = Vec::new();
    for marker in [
        "北京", "上海", "深圳", "杭州", "广州", "成都", "苹果", "OpenAI", "DeepSeek",
    ] {
        if text.contains(marker) {
            entities.push(marker.to_string());
        }
    }
    entities
}

pub(crate) fn infer_content_type(url: &str, text: &str) -> String {
    let lower = format!("{} {}", url, text).to_lowercase();
    if lower.contains("maps")
        || lower.contains("amap")
        || lower.contains("gaode")
        || lower.contains("dianping")
        || lower.contains("meituan")
        || lower.contains("地址")
        || lower.contains("导航")
    {
        "place".to_string()
    } else if lower.contains("video")
        || lower.contains("douyin")
        || lower.contains("kuaishou")
        || lower.contains("bilibili")
        || lower.contains("youtube")
        || lower.contains("tiktok")
        || lower.contains("视频")
    {
        "video".to_string()
    } else if lower.contains("spotify")
        || lower.contains("music.")
        || lower.contains("ximalaya")
        || lower.contains("xiaoyuzhou")
        || lower.contains("播客")
        || lower.contains("专辑")
    {
        "audio".to_string()
    } else if lower.contains("notion")
        || lower.contains("docs.google")
        || lower.contains("drive.google")
        || lower.contains("dropbox")
        || lower.contains("yuque")
        || lower.contains("feishu")
        || lower.contains("pan.baidu")
        || lower.contains("文档")
        || lower.contains("网盘")
    {
        "document".to_string()
    } else if lower.contains("zhihu.com")
        || lower.contains("zhuanlan")
        || lower.contains("question")
        || lower.contains("answer")
        || lower.contains("知乎")
    {
        "article".to_string()
    } else if lower.contains("weibo.com")
        || lower.contains("weibo.cn")
        || lower.contains("微博")
        || lower.contains("status")
    {
        "post".to_string()
    } else if lower.contains("product")
        || lower.contains("taobao")
        || lower.contains("tmall")
        || lower.contains("m.tb.cn")
        || lower.contains("tb.cn")
        || lower.contains("jd.com")
        || lower.contains("yangkeduo")
        || lower.contains("pinduoduo")
        || lower.contains("amazon.")
        || lower.contains("商品")
    {
        "product".to_string()
    } else if lower.contains("/article/")
        || lower.contains("zhuanlan")
        || lower.contains("mp.weixin.qq.com/s")
        || lower.contains("toutiao.com")
        || text.chars().count() > 600
    {
        "article".to_string()
    } else {
        "link".to_string()
    }
}

pub(super) fn fallback_summary(metadata: &LinkMetadata) -> String {
    if !metadata.description.trim().is_empty() {
        truncate_chars(&metadata.description, 160)
    } else if !metadata.content_text.trim().is_empty() {
        truncate_chars(&metadata.content_text, 160)
    } else {
        format!("来自 {} 的链接，已保存原始地址。", metadata.platform)
    }
}

pub(super) fn first_meaningful_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| line.chars().count() >= 4 && !line.starts_with("http"))
        .map(|line| truncate_chars(line, 120))
}

pub(super) fn is_url_only_text(text: &str) -> bool {
    let without_urls = Regex::new(r#"https?://\S+|www\.\S+"#)
        .map(|regex| regex.replace_all(text, " ").to_string())
        .unwrap_or_else(|_| text.to_string());
    without_urls
        .trim()
        .trim_matches(|ch: char| {
            ch.is_ascii_punctuation()
                || matches!(
                    ch,
                    '，' | '。' | '；' | '：' | '、' | '（' | '）' | '【' | '】'
                )
        })
        .trim()
        .is_empty()
}

pub(super) fn is_blocked_without_content(metadata: &LinkMetadata) -> bool {
    metadata.title.contains("网页验证拦截")
        || metadata.content_text.contains("无法从公开网页直接抓取正文")
        || metadata.content_text.contains("无法直接抓取正文")
}

pub(super) fn first_non_empty(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .map(|value| clean_text(&value))
        .find(|value| !value.is_empty())
}

pub(super) fn plain_text_preview(input: &str) -> String {
    clean_text(
        &Regex::new(r"<[^>]+>")
            .map(|regex| regex.replace_all(input, " ").to_string())
            .unwrap_or_else(|_| input.to_string()),
    )
}

pub(super) fn html_to_formatted_text(input: &str) -> String {
    let mut text = input
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");
    if let Ok(block_regex) = Regex::new(r"(?i)</?(p|div|section|article|li|h[1-6])\b[^>]*>") {
        text = block_regex.replace_all(&text, "\n").to_string();
    }
    if let Ok(tag_regex) = Regex::new(r"(?is)<[^>]+>") {
        text = tag_regex.replace_all(&text, " ").to_string();
    }
    clean_original_text(&text)
}

pub(super) fn clean_original_text(input: &str) -> String {
    let decoded = input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut lines = Vec::new();
    let mut blank_pending = false;
    for raw_line in decoded.lines() {
        let line = raw_line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank_pending = !lines.is_empty();
            continue;
        }
        if blank_pending && !lines.last().is_some_and(String::is_empty) {
            lines.push(String::new());
        }
        lines.push(line);
        blank_pending = false;
    }
    lines.join("\n").trim().to_string()
}

pub(crate) fn clean_text(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

pub(crate) fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut output = input.chars().take(max_chars).collect::<String>();
    if input.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn is_stop_word(value: &str) -> bool {
    matches!(
        value,
        "https"
            | "http"
            | "www"
            | "com"
            | "cn"
            | "net"
            | "org"
            | "这个"
            | "一个"
            | "可以"
            | "来自"
            | "分享"
            | "链接"
            | "打开"
            | "复制"
    )
}
