//! The initial page request.

use reqwest::Client;

use crate::platforms::{Platform, platform_for_url};

/// GET `url` with the user agent its platform needs (see
/// [`Platform::fetch_user_agent`]); redirects follow the client's safe policy.
pub(super) async fn fetch_page(http: &Client, url: &str) -> reqwest::Result<reqwest::Response> {
    let mut request = http.get(url);
    if let Some(user_agent) = Platform::from_label(&platform_for_url(url)).fetch_user_agent() {
        request = request.header("user-agent", user_agent);
    }
    request.send().await
}
