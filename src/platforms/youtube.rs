//! YouTube: player response / JSON-LD, oEmbed, then yt-dlp for captions.

use reqwest::Client;
use url::Url;

use crate::{
    extraction::{
        PlatformMetadata, json_ld_values, json_values_after_markers, oembed,
        platform_metadata_from_values, video,
    },
    models::LinkMetadata,
};

use super::identity::{id_after, query_value};

const OEMBED_ENDPOINT: &str = "https://www.youtube.com/oembed";

pub(super) fn identity(url: &Url, segments: &[&str]) -> Option<(String, String)> {
    let id = if url
        .host_str()
        .is_some_and(|host| host.ends_with("youtu.be"))
    {
        segments.first().map(|value| (*value).to_string())
    } else {
        query_value(url, &["v"]).or_else(|| id_after(segments, &["shorts", "live", "embed"]))
    }?;
    Some((id.clone(), format!("https://www.youtube.com/watch?v={id}")))
}

pub(super) fn embedded_state(html: &str) -> Option<PlatformMetadata> {
    let mut candidates = json_ld_values(html);
    candidates.extend(json_values_after_markers(
        html,
        &["var ytInitialPlayerResponse =", "ytInitialPlayerResponse ="],
    ));
    platform_metadata_from_values(
        candidates,
        &["title", "name", "headline"],
        &["shortDescription", "description"],
        &["author", "ownerChannelName", "name"],
        &["thumbnailUrl", "thumbnail", "image", "url"],
    )
}

pub(super) async fn enrich_remote(
    metadata: &mut LinkMetadata,
    http: &Client,
    allow_video_transcription: bool,
) {
    oembed::enrich(metadata, http, OEMBED_ENDPOINT, None).await;
    video::enrich_with_yt_dlp(metadata, http, allow_video_transcription, true).await;
}

#[cfg(test)]
mod tests {
    use super::embedded_state;

    #[test]
    fn parses_player_response() {
        let html = r#"<script>var ytInitialPlayerResponse = {"videoDetails":{"title":"Rust 入门","shortDescription":"第一课：所有权","author":"频道名","thumbnail":{"thumbnails":[{"url":"https://i.ytimg.com/vi/a/hq.jpg"}]}}};</script>"#;
        let metadata = embedded_state(html).expect("player response");
        assert_eq!(metadata.title.as_deref(), Some("Rust 入门"));
        assert_eq!(metadata.description.as_deref(), Some("第一课：所有权"));
        assert_eq!(metadata.author.as_deref(), Some("频道名"));
    }
}
