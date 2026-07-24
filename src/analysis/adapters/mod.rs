mod asr;
mod douban;
mod maps;
mod oembed;
mod structured;
mod video;
mod xiaohongshu;

use reqwest::Client;

use crate::models::LinkMetadata;

pub(super) use xiaohongshu::merge as merge_xiaohongshu_metadata;

pub(super) fn enrich_from_html(metadata: &mut LinkMetadata, html: &str) {
    structured::enrich(metadata, html);
}

pub(super) async fn enrich_remote(
    metadata: &mut LinkMetadata,
    http: &Client,
    allow_video_transcription: bool,
) {
    oembed::enrich(metadata, http).await;
    douban::enrich(metadata, http).await;
    maps::enrich(metadata, http).await;
    video::enrich(metadata, http, allow_video_transcription).await;
}
