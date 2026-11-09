use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::Serialize;

use crate::{extraction::extract_metadata, network::safe_redirect_policy};

#[derive(Serialize)]
struct ProbeResult {
    input_url: String,
    final_url: String,
    platform: String,
    title: String,
    author: Option<String>,
    image_url: Option<String>,
    original_preview: String,
    original_chars: usize,
    block_count: usize,
    media_count: usize,
    fact_count: usize,
    status: String,
    completeness: u8,
    method: String,
    transcript_source: Option<String>,
    error: Option<String>,
}

pub(crate) async fn run(urls: &[String], strict: bool) -> Result<()> {
    if urls.is_empty() {
        return Err(anyhow!("usage: backend --probe <url> [url ...]"));
    }
    let http = probe_client()?;

    let mut failures = Vec::new();
    for url in urls {
        let metadata = extract_metadata(&http, url, None).await;
        if strict && !is_complete_probe(&metadata) {
            failures.push(format!(
                "{}: status={}, chars={}, title={}",
                metadata.platform,
                metadata.extraction.status,
                metadata.original_content.plain_text.chars().count(),
                metadata.title
            ));
        }
        print_result(metadata)?;
    }
    if !failures.is_empty() {
        return Err(anyhow!(
            "platform probes did not meet the complete snapshot contract:\n{}",
            failures.join("\n")
        ));
    }
    Ok(())
}

pub(crate) async fn run_shared(url: &str, shared_text: &str, strict: bool) -> Result<()> {
    let http = probe_client()?;
    let metadata = extract_metadata(&http, url, Some(shared_text)).await;
    let complete = is_complete_probe(&metadata);
    print_result(metadata)?;
    if strict && !complete {
        return Err(anyhow!(
            "shared-text probe did not meet the complete snapshot contract"
        ));
    }
    Ok(())
}

fn is_complete_probe(metadata: &crate::models::LinkMetadata) -> bool {
    metadata.extraction.status == "complete"
        && metadata.title != "未命名链接"
        && metadata.original_content.plain_text.chars().count() >= 20
        && metadata.extraction.content_fingerprint.is_some()
}

fn print_result(metadata: crate::models::LinkMetadata) -> Result<()> {
    let result = ProbeResult {
        input_url: metadata.source_url,
        final_url: metadata.final_url,
        platform: metadata.platform,
        title: metadata.title,
        author: metadata.author,
        image_url: metadata.image_url,
        original_preview: metadata
            .original_content
            .plain_text
            .chars()
            .take(240)
            .collect(),
        original_chars: metadata.original_content.plain_text.chars().count(),
        block_count: metadata.original_content.blocks.len(),
        media_count: metadata.media.len(),
        fact_count: metadata.source_facts.len(),
        status: metadata.extraction.status,
        completeness: metadata.extraction.completeness,
        method: metadata.extraction.method,
        transcript_source: metadata
            .source_facts
            .iter()
            .find(|fact| fact.key == "transcript_source")
            .map(|fact| fact.value.clone()),
        error: metadata.extraction.last_error,
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn probe_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(55))
        .redirect(safe_redirect_policy(8))
        .user_agent("ShareWithMe/0.2 (+personal link organizer; platform probe)")
        .build()
        .context("build probe HTTP client")
}

#[cfg(test)]
mod tests {
    use crate::{content::OriginalContent, models::LinkMetadata};

    use super::is_complete_probe;

    #[test]
    fn strict_probe_requires_a_meaningful_persistable_snapshot() {
        let mut metadata = LinkMetadata {
            title: "真实标题".to_string(),
            original_content: OriginalContent::from_plain_text(
                "这是一段足够长、可以保存并用于后续分析的原始正文。",
            ),
            ..LinkMetadata::default()
        };
        metadata.extraction.status = "complete".to_string();
        metadata.extraction.content_fingerprint = Some("fingerprint".to_string());
        assert!(is_complete_probe(&metadata));

        metadata.extraction.status = "partial".to_string();
        assert!(!is_complete_probe(&metadata));
    }
}
