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
