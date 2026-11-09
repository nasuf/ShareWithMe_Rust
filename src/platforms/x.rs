//! X (Twitter): post text from the official oEmbed endpoint, media via yt-dlp.

use reqwest::Client;

use crate::{
    extraction::{oembed, video},
    models::LinkMetadata,
};

use super::identity::id_after;

const OEMBED_ENDPOINT: &str = "https://publish.x.com/oembed";

pub(super) fn identity(segments: &[&str]) -> Option<(String, String)> {
    id_after(segments, &["status"]).map(|id| {
        let author = segments.first().copied().unwrap_or("i");
        (id.clone(), format!("https://x.com/{author}/status/{id}"))
    })
}

pub(super) async fn enrich_remote(
    metadata: &mut LinkMetadata,
    http: &Client,
    allow_video_transcription: bool,
) {
    oembed::enrich(metadata, http, OEMBED_ENDPOINT, None).await;
    video::enrich_with_yt_dlp(metadata, http, allow_video_transcription, false).await;
}
