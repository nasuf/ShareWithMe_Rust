use std::{env, path::PathBuf, time::Duration};

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
    pub(crate) extractor: Option<String>,
}

pub(crate) async fn render_extract(url: &str) -> Option<RenderedPage> {
    let script = renderer_script_path()?;
    let mut command = Command::new("node");
    command.arg(script).arg(url);

    let output = timeout(Duration::from_secs(45), command.output())
        .await
        .ok()?
        .ok()?;

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
