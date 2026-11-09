use chrono::Utc;

use crate::{
    digest::collection_digest,
    models::{JsonExport, LinkItem, ListItemsQuery},
};

pub(crate) fn matches_filter(item: &LinkItem, query: &ListItemsQuery) -> bool {
    if let Some(status) = &query.status
        && !status.is_empty()
        && item.status != *status
    {
        return false;
    }
    if let Some(category) = &query.category
        && !category.is_empty()
        && item.category != *category
    {
        return false;
    }
    if let Some(platform) = &query.platform
        && !platform.is_empty()
        && item.platform != *platform
    {
        return false;
    }
    if let Some(search) = &query.query {
        let needle = search.trim().to_lowercase();
        if !needle.is_empty() {
            let haystack = format!(
                "{} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                item.title,
                item.description,
                item.summary,
                item.category,
                item.platform,
                item.source_url,
                item.final_url,
                item.source_app.as_deref().unwrap_or_default(),
                item.author.as_deref().unwrap_or_default(),
                item.content_type,
                item.notes,
                item.keywords.join(" "),
                item.entities.join(" "),
                item.content_text
            )
            .to_lowercase();
            return haystack.contains(&needle);
        }
    }
    true
}

pub(crate) fn build_markdown_export(items: &[LinkItem]) -> String {
    let digest = collection_digest(items);
    let mut output = String::new();
    output.push_str("# ShareWithMe Export\n\n");
    output.push_str(&format!(
        "- Generated: {}\n- Total active items: {}\n- Total stored items: {}\n\n",
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
        digest.total,
        items.len()
    ));
    output.push_str("## Collection Summary\n\n");
    output.push_str(&format!("{}\n\n", digest.focus_summary));
    if !digest.category_summaries.is_empty() {
        output.push_str("### Categories\n\n");
        for bucket in &digest.category_summaries {
            output.push_str(&format!("- {}: {}\n", bucket.label, bucket.count));
        }
        output.push('\n');
    }
    output.push_str("## Items\n\n");
    for item in items {
        output.push_str(&format!("### {}\n\n", sanitize_markdown_line(&item.title)));
        output.push_str(&format!(
            "- Platform: {}\n- Category: {}\n- Type: {}\n- Status: {}\n- Importance: {}\n- Created: {}\n- URL: {}\n",
            sanitize_markdown_line(&item.platform),
            sanitize_markdown_line(&item.category),
            sanitize_markdown_line(&item.content_type),
            sanitize_markdown_line(&item.status),
            item.importance_score,
            item.created_at.format("%Y-%m-%d %H:%M:%S UTC"),
            item.final_url
        ));
        if !item.keywords.is_empty() {
            output.push_str(&format!(
                "- Keywords: {}\n",
                item.keywords
                    .iter()
                    .map(|value| sanitize_markdown_line(value))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        output.push('\n');
        if !item.summary.trim().is_empty() {
            output.push_str(&format!("{}\n\n", sanitize_markdown_line(&item.summary)));
        }
        let original_text = if item.original_text.trim().is_empty() {
            item.content_text.trim()
        } else {
            item.original_text.trim()
        };
        if !original_text.is_empty() {
            output.push_str("#### 原文\n\n");
            output.push_str(original_text);
            output.push_str("\n\n");
        }
        if !item.notes.trim().is_empty() {
            output.push_str(&format!(
                "> Notes: {}\n\n",
                sanitize_markdown_line(&item.notes)
            ));
        }
    }
    output
}

pub(crate) fn build_json_export(items: Vec<LinkItem>) -> JsonExport {
    let digest = collection_digest(&items);
    JsonExport {
        schema_version: 1,
        generated_at: Utc::now(),
        item_count: items.len(),
        digest,
        items,
    }
}

pub(crate) fn sort_items(items: &mut [LinkItem], sort: Option<&str>) {
    match sort.unwrap_or("newest") {
        "oldest" => items.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.title.cmp(&b.title))
        }),
        "importance" => items.sort_by(|a, b| {
            b.importance_score
                .cmp(&a.importance_score)
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| a.title.cmp(&b.title))
        }),
        _ => items.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.title.cmp(&b.title))
        }),
    }
}

fn sanitize_markdown_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {

    use super::{build_json_export, build_markdown_export, matches_filter, sort_items};
    use crate::models::{ListItemsQuery, test_item};

    #[test]
    fn item_filter_respects_status_query() {
        let mut item = test_item("favorite", "https://example.com/favorite", "Favorite");
        item.status = "favorite".to_string();
        item.notes = "周末复盘".to_string();

        let favorite_query = ListItemsQuery {
            query: None,
            category: None,
            platform: None,
            status: Some("favorite".to_string()),
            sort: None,
            fields: None,
        };
        let active_query = ListItemsQuery {
            query: None,
            category: None,
            platform: None,
            status: Some("active".to_string()),
            sort: None,
            fields: None,
        };
        let all_query = ListItemsQuery {
            query: None,
            category: None,
            platform: None,
            status: Some(String::new()),
            sort: None,
            fields: None,
        };
        let platform_query = ListItemsQuery {
            query: None,
            category: None,
            platform: Some("example.com".to_string()),
            status: Some(String::new()),
            sort: None,
            fields: None,
        };
        let other_platform_query = ListItemsQuery {
            query: None,
            category: None,
            platform: Some("GitHub".to_string()),
            status: Some(String::new()),
            sort: None,
            fields: None,
        };

        assert!(matches_filter(&item, &favorite_query));
        assert!(!matches_filter(&item, &active_query));
        assert!(matches_filter(&item, &all_query));
        assert!(matches_filter(&item, &platform_query));
        assert!(!matches_filter(&item, &other_platform_query));

        for term in ["example.com/favorite", "example.com", "周末复盘"] {
            let query = ListItemsQuery {
                query: Some(term.to_string()),
                category: None,
                platform: None,
                status: Some(String::new()),
                sort: None,
                fields: None,
            };
            assert!(matches_filter(&item, &query), "term should match: {term}");
        }
    }

    #[test]
    fn sort_items_supports_recency_and_importance() {
        let mut low_old = test_item("low-old", "https://example.com/low", "Low");
        let mut high_new = test_item("high-new", "https://example.com/high", "High");
        let mut mid = test_item("mid", "https://example.com/mid", "Mid");
        low_old.importance_score = 20;
        high_new.importance_score = 90;
        mid.importance_score = 50;
        low_old.created_at -= chrono::Duration::minutes(2);
        mid.created_at -= chrono::Duration::minutes(1);

        let mut items = vec![mid.clone(), low_old.clone(), high_new.clone()];
        sort_items(&mut items, Some("oldest"));
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["low-old", "mid", "high-new"]
        );

        sort_items(&mut items, Some("importance"));
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["high-new", "mid", "low-old"]
        );
    }

    #[test]
    fn markdown_export_contains_summary_and_items() {
        let mut item = test_item("export", "https://example.com/export", "Export Title");
        item.category = "资料".to_string();
        item.keywords = vec!["备份".to_string(), "复盘".to_string()];
        item.notes = "手动备注".to_string();

        let markdown = build_markdown_export(&[item]);

        assert!(markdown.contains("# ShareWithMe Export"));
        assert!(markdown.contains("Export Title"));
        assert!(markdown.contains("https://example.com/export"));
        assert!(markdown.contains("Keywords: 备份, 复盘"));
        assert!(markdown.contains("Notes: 手动备注"));
    }

    #[test]
    fn json_export_preserves_items_and_schema_version() {
        let mut item = test_item("json-export", "https://example.com/json", "JSON Export");
        item.category = "资料".to_string();
        item.keywords = vec!["迁移".to_string()];

        let export = build_json_export(vec![item.clone()]);

        assert_eq!(export.schema_version, 1);
        assert_eq!(export.item_count, 1);
        assert_eq!(export.digest.total, 1);
        assert_eq!(export.items[0].id, "json-export");
        assert_eq!(export.items[0].keywords, ["迁移"]);
    }
}
