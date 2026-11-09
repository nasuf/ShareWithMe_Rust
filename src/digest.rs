use chrono::Utc;

use crate::{
    models::{CollectionDigest, CountBucket, DigestItem, LinkItem},
    text::{clean_text, truncate_chars},
};

pub(crate) fn count_by(values: impl Iterator<Item = String>) -> Vec<CountBucket> {
    let mut map = std::collections::BTreeMap::<String, usize>::new();
    for value in values {
        let label = value.trim();
        if label.is_empty() {
            continue;
        }
        *map.entry(label.to_string()).or_default() += 1;
    }
    let mut buckets = map
        .into_iter()
        .map(|(label, count)| CountBucket { label, count })
        .collect::<Vec<_>>();
    buckets.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
    buckets.truncate(20);
    buckets
}

pub(crate) fn collection_digest(items: &[LinkItem]) -> CollectionDigest {
    let active_items = items
        .iter()
        .filter(|item| item.status != "archived")
        .collect::<Vec<_>>();
    let category_summaries = count_by(active_items.iter().map(|item| item.category.clone()));
    let platform_summaries = count_by(active_items.iter().map(|item| item.platform.clone()));
    let keyword_summaries = count_by(active_items.iter().flat_map(|item| item.keywords.clone()));
    let recent_items = active_items
        .iter()
        .take(8)
        .map(|item| DigestItem {
            id: item.id.clone(),
            title: item.title.clone(),
            platform: item.platform.clone(),
            category: item.category.clone(),
            summary: truncate_chars(&item.summary, 140),
            created_at: item.created_at,
        })
        .collect::<Vec<_>>();
    let suggestions = digest_suggestions(
        active_items.len(),
        &category_summaries,
        &platform_summaries,
        &keyword_summaries,
    );

    CollectionDigest {
        total: active_items.len(),
        generated_at: Utc::now(),
        focus_summary: focus_summary(&category_summaries, &platform_summaries, &keyword_summaries),
        category_summaries,
        recent_items,
        suggestions,
    }
}

fn focus_summary(
    categories: &[CountBucket],
    platforms: &[CountBucket],
    keywords: &[CountBucket],
) -> String {
    if categories.is_empty() {
        return "还没有可汇总的收藏，先从系统分享面板或手动添加链接开始。".to_string();
    }

    let category_text = categories
        .iter()
        .take(3)
        .map(|bucket| format!("{} {}", bucket.label, bucket.count))
        .collect::<Vec<_>>()
        .join("、");
    let platform_text = platforms
        .iter()
        .take(2)
        .map(|bucket| bucket.label.clone())
        .collect::<Vec<_>>()
        .join("、");
    let keyword_text = keywords
        .iter()
        .take(4)
        .map(|bucket| bucket.label.clone())
        .collect::<Vec<_>>()
        .join("、");

    match (platform_text.is_empty(), keyword_text.is_empty()) {
        (false, false) => format!(
            "近期收藏主要集中在 {category_text}，来源多来自 {platform_text}，高频线索包括 {keyword_text}。"
        ),
        (false, true) => {
            format!("近期收藏主要集中在 {category_text}，来源多来自 {platform_text}。")
        }
        (true, false) => {
            format!("近期收藏主要集中在 {category_text}，高频线索包括 {keyword_text}。")
        }
        (true, true) => format!("近期收藏主要集中在 {category_text}。"),
    }
}

fn digest_suggestions(
    total: usize,
    categories: &[CountBucket],
    platforms: &[CountBucket],
    keywords: &[CountBucket],
) -> Vec<String> {
    let mut suggestions = Vec::new();
    if total == 0 {
        suggestions.push("添加第一个链接后，ShareWithMe 会自动生成主题和来源汇总。".to_string());
        return suggestions;
    }
    if categories.first().is_some_and(|bucket| bucket.count >= 5) {
        let category = &categories[0].label;
        suggestions.push(format!(
            "为「{category}」建立一个专题清单，方便后续集中复盘。"
        ));
    }
    if platforms.first().is_some_and(|bucket| bucket.count >= 4) {
        let platform = &platforms[0].label;
        suggestions.push(format!(
            "最近来自「{platform}」的内容较多，可以定期清理低价值重复收藏。"
        ));
    }
    if keywords
        .iter()
        .any(|bucket| bucket.label.contains("优惠") || bucket.label.contains("券"))
    {
        suggestions.push("含优惠/券相关内容，建议在过期前集中检查。".to_string());
    }
    if suggestions.is_empty() {
        suggestions.push("当前收藏分布较均衡，可以继续观察哪些主题反复出现。".to_string());
    }
    unique_limited(suggestions, 5)
}

fn unique_limited(values: Vec<String>, limit: usize) -> Vec<String> {
    let mut unique = Vec::new();
    for value in values {
        let cleaned = clean_text(&value);
        if !cleaned.is_empty() && !unique.contains(&cleaned) {
            unique.push(cleaned);
        }
        if unique.len() >= limit {
            break;
        }
    }
    unique
}

#[cfg(test)]
mod tests {

    use super::collection_digest;
    use crate::models::test_item;

    #[test]
    fn digest_summarizes_active_collection() {
        let mut items = vec![
            test_item("one", "https://github.com/example/repo", "Rust API 架构"),
            test_item(
                "two",
                "https://github.com/example/other",
                "Flutter API 示例",
            ),
            test_item("three", "https://item.jd.com/1.html", "优惠券 商品"),
        ];
        items[0].category = "技术".to_string();
        items[0].keywords = vec!["Rust".to_string(), "API".to_string()];
        items[1].category = "技术".to_string();
        items[1].keywords = vec!["Flutter".to_string(), "API".to_string()];
        items[2].category = "购物".to_string();
        items[2].keywords = vec!["优惠".to_string(), "券".to_string()];

        let mut archived = test_item("archived", "https://example.com/old", "Old");
        archived.status = "archived".to_string();
        items.push(archived);

        let digest = collection_digest(&items);
        assert_eq!(digest.total, 3);
        assert_eq!(digest.recent_items.len(), 3);
        assert!(digest.focus_summary.contains("技术 2"));
        assert!(digest.suggestions.iter().any(|item| item.contains("优惠")));
    }
}
