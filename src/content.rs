use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub(crate) struct OriginalContent {
    #[serde(default)]
    pub(crate) format: String,
    #[serde(default)]
    pub(crate) plain_text: String,
    #[serde(default)]
    pub(crate) blocks: Vec<ContentBlock>,
}

impl OriginalContent {
    pub(crate) fn from_plain_text(text: impl Into<String>) -> Self {
        let plain_text = text.into().trim().to_string();
        if plain_text.is_empty() {
            return Self::default();
        }
        let blocks = plain_text
            .split("\n\n")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|text| ContentBlock {
                kind: "paragraph".to_string(),
                text: text.to_string(),
                ..ContentBlock::default()
            })
            .collect();
        Self {
            format: "blocks-v1".to_string(),
            plain_text,
            blocks,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.plain_text.trim().is_empty() && self.blocks.is_empty()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub(crate) struct ContentBlock {
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) level: Option<u8>,
    #[serde(default)]
    pub(crate) url: Option<String>,
    #[serde(default)]
    pub(crate) alt: Option<String>,
    #[serde(default)]
    pub(crate) items: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub(crate) struct MediaAsset {
    pub(crate) kind: String,
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) cached_url: Option<String>,
    #[serde(default)]
    pub(crate) thumbnail_url: Option<String>,
    #[serde(default)]
    pub(crate) alt: Option<String>,
    #[serde(default)]
    pub(crate) width: Option<u32>,
    #[serde(default)]
    pub(crate) height: Option<u32>,
    #[serde(default)]
    pub(crate) duration_seconds: Option<f64>,
}

// A visual archive of the source as it looked when saved: a screenshot of the
// article container stored under `/media`, kept so the item stays viewable in
// its original layout after the link dies.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub(crate) struct ArchiveSnapshot {
    pub(crate) kind: String,
    pub(crate) url: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) source_url: String,
    pub(crate) captured_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub(crate) struct SourceFact {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) value: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub(crate) struct ExtractionState {
    pub(crate) status: String,
    pub(crate) adapter: String,
    pub(crate) method: String,
    pub(crate) completeness: u8,
    #[serde(default)]
    pub(crate) fetched_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) refresh_after: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) content_fingerprint: Option<String>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
}

impl Default for ExtractionState {
    fn default() -> Self {
        Self {
            status: "legacy".to_string(),
            adapter: "legacy".to_string(),
            method: "legacy".to_string(),
            completeness: 0,
            fetched_at: None,
            refresh_after: None,
            content_fingerprint: None,
            last_error: None,
        }
    }
}

// Completeness at which a snapshot counts as "complete" (see finalize_metadata).
pub(crate) const SETTLED_COMPLETENESS: u8 = 80;
// A complete snapshot is not refreshed automatically: re-fetching it can only
// replace it with a source that was since edited, deleted, blocked or
// re-laid-out. Users can still force a refresh from the detail page.
const SETTLED_REFRESH_DAYS: i64 = 3650;

pub(crate) fn settled_refresh_after(now: DateTime<Utc>) -> DateTime<Utc> {
    now + chrono::Duration::days(SETTLED_REFRESH_DAYS)
}

impl ExtractionState {
    /// A complete snapshot (also one kept as `stale` after a failed refresh).
    /// Legacy items are excluded so they still get upgraded.
    pub(crate) fn is_settled(&self) -> bool {
        self.status != "legacy" && self.completeness >= SETTLED_COMPLETENESS
    }

    pub(crate) fn is_stale_at(&self, now: DateTime<Utc>) -> bool {
        !self.is_settled()
            && self
                .refresh_after
                .is_none_or(|refresh_after| refresh_after <= now)
    }
}

// A media URL with expiring signature / tracking query params removed. CDNs
// such as Toutiao re-sign the same image on every fetch, so the raw URL cannot
// be used to recognise an image that is already cached.
pub(crate) fn media_identity(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return url.to_string();
    };
    let kept = parsed
        .query_pairs()
        .filter(|(key, _)| !is_volatile_query_key(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed.set_fragment(None);
    if kept.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.query_pairs_mut().clear().extend_pairs(kept);
    }
    parsed.into()
}

fn is_volatile_query_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("x-")
        || key.starts_with('_')
        || key.starts_with("utm_")
        || matches!(
            key.as_str(),
            "expires"
                | "signature"
                | "sig"
                | "sign"
                | "token"
                | "auth_key"
                | "t"
                | "ts"
                | "timestamp"
                | "from"
                | "lk3s"
        )
}

pub(crate) fn content_fingerprint(text: &str) -> String {
    // Stable FNV-1a is sufficient for change detection and avoids a crypto dependency.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(crate) fn snapshot_fingerprint(
    title: &str,
    author: Option<&str>,
    text: &str,
    facts: &[SourceFact],
) -> Option<String> {
    if title.trim().is_empty() && text.trim().is_empty() {
        return None;
    }
    let mut stable_facts = facts
        .iter()
        .filter(|fact| {
            !matches!(
                fact.key.as_str(),
                "view_count" | "like" | "favorite" | "danmaku" | "comment_count"
            )
        })
        .map(|fact| format!("{}={}", fact.key.trim(), fact.value.trim()))
        .collect::<Vec<_>>();
    stable_facts.sort();
    Some(content_fingerprint(&format!(
        "title={}\nauthor={}\ntext={}\nfacts={}",
        title.trim(),
        author.unwrap_or_default().trim(),
        text.trim(),
        stable_facts.join("\n")
    )))
}

#[cfg(test)]
mod tests {
    use super::{SourceFact, snapshot_fingerprint};

    #[test]
    fn snapshot_fingerprint_tracks_semantic_facts_but_not_view_counts() {
        let base = vec![SourceFact {
            key: "price".to_string(),
            label: "价格".to_string(),
            value: "99".to_string(),
        }];
        let first = snapshot_fingerprint("商品", None, "商品正文", &base);
        let mut changed_price = base.clone();
        changed_price[0].value = "109".to_string();
        assert_ne!(
            first,
            snapshot_fingerprint("商品", None, "商品正文", &changed_price)
        );

        let mut changed_views = base.clone();
        changed_views.push(SourceFact {
            key: "view_count".to_string(),
            label: "播放量".to_string(),
            value: "1000".to_string(),
        });
        assert_eq!(
            first,
            snapshot_fingerprint("商品", None, "商品正文", &changed_views)
        );
    }

    #[test]
    fn media_identity_ignores_expiring_signatures_but_keeps_real_params() {
        let first = "https://p3-sign.toutiaoimg.com/tos-cn-i-6w9my0ksvp/abc~tplv-tt.jpeg?_iz=58558&from=article.pc_detail&lk3s=953192f4&x-expires=1784516347&x-signature=AAA%3D";
        let resigned = "https://p3-sign.toutiaoimg.com/tos-cn-i-6w9my0ksvp/abc~tplv-tt.jpeg?_iz=58558&from=article.pc_detail&lk3s=953192f4&x-expires=1794634071&x-signature=BBB%3D";
        assert_eq!(
            super::media_identity(first),
            super::media_identity(resigned)
        );
        assert_eq!(
            super::media_identity(first),
            "https://p3-sign.toutiaoimg.com/tos-cn-i-6w9my0ksvp/abc~tplv-tt.jpeg"
        );
        assert_ne!(
            super::media_identity("https://img.example.com/view.php?id=1"),
            super::media_identity("https://img.example.com/view.php?id=2")
        );
    }
}

#[cfg(test)]
mod settled_tests {
    use chrono::{Duration, Utc};

    use super::ExtractionState;

    #[test]
    fn only_incomplete_snapshots_go_stale() {
        let now = Utc::now();
        let past = Some(now - Duration::hours(1));
        let state = |status: &str, completeness: u8| ExtractionState {
            status: status.to_string(),
            completeness,
            refresh_after: past,
            ..ExtractionState::default()
        };
        assert!(!state("complete", 85).is_stale_at(now));
        // A complete snapshot kept after a failed refresh.
        assert!(!state("stale", 90).is_stale_at(now));
        assert!(state("partial", 50).is_stale_at(now));
        assert!(state("blocked", 40).is_stale_at(now));
        assert!(state("legacy", 90).is_stale_at(now));
    }
}
