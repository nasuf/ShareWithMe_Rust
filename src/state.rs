use std::sync::Arc;

use reqwest::Client;

use crate::{config::AppConfig, store::Store};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) store: Arc<Store>,
    pub(crate) http: Client,
    pub(crate) config: Arc<AppConfig>,
}
