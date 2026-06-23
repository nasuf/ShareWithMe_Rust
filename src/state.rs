use std::sync::{Arc, Mutex};

use reqwest::Client;

use crate::{config::AppConfig, store::Store};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) store: Arc<Mutex<Store>>,
    pub(crate) http: Client,
    pub(crate) config: Arc<AppConfig>,
}
