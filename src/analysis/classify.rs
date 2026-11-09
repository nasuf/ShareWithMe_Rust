//! Heuristic classification used when the AI analyzer is unavailable.

use regex::Regex;

use crate::{models::LinkMetadata, text::truncate_chars};

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
        "淘宝" | "天猫" | "京东" | "拼多多" | "Amazon" | "什么值得买" => {
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
