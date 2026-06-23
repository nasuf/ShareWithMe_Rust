use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use chrono::Utc;

use crate::models::{LinkItem, UpdateItemRequest};

#[derive(Debug)]
pub(crate) struct Store {
    pub(crate) path: PathBuf,
    pub(crate) items: Vec<LinkItem>,
}

impl Store {
    pub(crate) fn load(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                path,
                items: Vec::new(),
            });
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read store file {}", path.display()))?;
        let items = serde_json::from_str::<Vec<LinkItem>>(&raw)
            .with_context(|| format!("parse store file {}", path.display()))?;
        Ok(Self { path, items })
    }

    fn persist(&self) -> Result<()> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("create store directory {}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(&self.items).context("serialize store")?;
        fs::write(&self.path, raw).with_context(|| format!("write store {}", self.path.display()))
    }
}

pub(crate) fn save_item(store: &Arc<Mutex<Store>>, mut item: LinkItem) -> Result<()> {
    let mut store = store
        .lock()
        .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
    if let Some(existing) = store
        .items
        .iter_mut()
        .find(|existing| existing.id == item.id)
    {
        *existing = item;
    } else if let Some(existing) = store
        .items
        .iter_mut()
        .find(|existing| existing.final_url == item.final_url)
    {
        item.id = existing.id.clone();
        item.created_at = existing.created_at;
        *existing = item;
    } else {
        store.items.push(item);
    }
    store.items.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.title.cmp(&b.title))
    });
    store.persist()
}

pub(crate) fn all_items(store: &Arc<Mutex<Store>>) -> Result<Vec<LinkItem>> {
    Ok(store
        .lock()
        .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
        .items
        .clone())
}

pub(crate) fn import_items(
    store: &Arc<Mutex<Store>>,
    incoming: Vec<LinkItem>,
) -> Result<(usize, usize)> {
    let mut store = store
        .lock()
        .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
    let mut created = 0;
    let mut merged = 0;

    for mut item in incoming {
        if let Some(existing) = store
            .items
            .iter_mut()
            .find(|existing| existing.id == item.id)
        {
            *existing = item;
            merged += 1;
        } else if let Some(existing) = store
            .items
            .iter_mut()
            .find(|existing| existing.final_url == item.final_url)
        {
            item.id = existing.id.clone();
            item.created_at = existing.created_at;
            *existing = item;
            merged += 1;
        } else {
            store.items.push(item);
            created += 1;
        }
    }

    store.items.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.title.cmp(&b.title))
    });
    store.persist()?;
    Ok((created, merged))
}

pub(crate) fn find_item(store: &Arc<Mutex<Store>>, id: &str) -> Result<Option<LinkItem>> {
    Ok(store
        .lock()
        .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
        .items
        .iter()
        .find(|item| item.id == id)
        .cloned())
}

pub(crate) fn delete_item_by_id(store: &Arc<Mutex<Store>>, id: &str) -> Result<bool> {
    let mut store = store
        .lock()
        .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
    let before = store.items.len();
    store.items.retain(|item| item.id != id);
    let deleted = store.items.len() != before;
    if deleted {
        store.persist()?;
    }
    Ok(deleted)
}

pub(crate) fn update_item_status(store: &Arc<Mutex<Store>>, id: &str, status: &str) -> Result<()> {
    let mut store = store
        .lock()
        .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
    let item = store
        .items
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| anyhow::anyhow!("item not found"))?;
    item.status = status.to_string();
    item.updated_at = Utc::now();
    store.persist()
}

pub(crate) fn update_item_metadata(
    store: &Arc<Mutex<Store>>,
    id: &str,
    request: UpdateItemRequest,
) -> Result<LinkItem> {
    let mut store = store
        .lock()
        .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
    let updated = {
        let item = store
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| anyhow::anyhow!("item not found"))?;
        if let Some(category) = request.category {
            item.category = category;
        }
        if let Some(keywords) = request.keywords {
            item.keywords = keywords;
        }
        if let Some(notes) = request.notes {
            item.notes = notes;
        }
        item.updated_at = Utc::now();
        item.clone()
    };
    store.persist()?;
    Ok(updated)
}

pub(crate) fn update_item_cover(
    store: &Arc<Mutex<Store>>,
    id: &str,
    image_url: Option<String>,
    remote_image_url: Option<String>,
    cover_cached_at: Option<chrono::DateTime<Utc>>,
    cover_checked_at: chrono::DateTime<Utc>,
) -> Result<LinkItem> {
    let mut store = store
        .lock()
        .map_err(|_| anyhow::anyhow!("store lock poisoned"))?;
    let updated = {
        let item = store
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| anyhow::anyhow!("item not found"))?;
        if let Some(image_url) = image_url {
            item.image_url = Some(image_url);
        }
        if let Some(remote_image_url) = remote_image_url {
            item.remote_image_url = Some(remote_image_url);
        }
        if let Some(cover_cached_at) = cover_cached_at {
            item.cover_cached_at = Some(cover_cached_at);
        }
        item.cover_checked_at = Some(cover_checked_at);
        item.updated_at = Utc::now();
        item.clone()
    };
    store.persist()?;
    Ok(updated)
}
