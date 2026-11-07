use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;
use tokio::{process::Command, time::timeout};

#[derive(Debug, Deserialize)]
pub(crate) struct RenderedPage {
    pub(crate) ok: bool,
    pub(crate) final_url: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) image_url: Option<String>,
    pub(crate) content_text: Option<String>,
    // The article container's HTML, used to rebuild headings, images and
    // captions. Older renderer builds omit it.
    #[serde(default)]
    pub(crate) content_html: Option<String>,
    pub(crate) extractor: Option<String>,
    #[serde(default)]
    pub(crate) snapshot_width: Option<u32>,
    #[serde(default)]
    pub(crate) snapshot_height: Option<u32>,
    #[serde(default)]
    pub(crate) snapshot_error: Option<String>,
}

pub(crate) async fn render_extract(url: &str) -> Option<RenderedPage> {
    run_renderer(url, None, Duration::from_secs(60)).await
}

// Render the page and also write a JPEG screenshot of the article to `output`.
// Returns the rendered page only when the screenshot was actually written.
pub(crate) async fn render_snapshot(url: &str, output: &Path) -> Option<RenderedPage> {
    let rendered = run_renderer(url, Some(output), Duration::from_secs(90)).await?;
    if let Some(error) = &rendered.snapshot_error {
        tracing::warn!(url, error, "renderer snapshot failed");
        return None;
    }
    (rendered.snapshot_width.is_some() && output.exists()).then_some(rendered)
}

async fn run_renderer(url: &str, snapshot: Option<&Path>, limit: Duration) -> Option<RenderedPage> {
    let script = renderer_script_path()?;
    let mut command = Command::new("node");
    command.arg(script).arg(url).kill_on_drop(true);
    if let Some(output) = snapshot {
        command.arg("--snapshot").arg(output);
    }

    let output = timeout(limit, command.output()).await.ok()?.ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let rendered = serde_json::from_str::<RenderedPage>(stdout.trim()).ok()?;
    if rendered.ok { Some(rendered) } else { None }
}

fn renderer_script_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("SHARE_WITH_ME_RENDERER") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    [
        PathBuf::from("tools/render_extract.mjs"),
        PathBuf::from("backend/tools/render_extract.mjs"),
    ]
    .into_iter()
    .find(|candidate| candidate.exists())
}
