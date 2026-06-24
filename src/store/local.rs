use std::{
    fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

use anyhow::{Context, Result};

use crate::models::LinkItem;

#[derive(Debug)]
pub(super) struct LocalStore {
    path: PathBuf,
    items: Mutex<Vec<LinkItem>>,
}

impl LocalStore {
    pub(super) fn load(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::from_items(path, Vec::new()));
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read store file {}", path.display()))?;
        let items = serde_json::from_str::<Vec<LinkItem>>(&raw)
            .with_context(|| format!("parse store file {}", path.display()))?;
        Ok(Self::from_items(path, items))
    }

    pub(super) fn from_items(path: PathBuf, items: Vec<LinkItem>) -> Self {
        Self {
            path,
            items: Mutex::new(items),
        }
    }

    pub(super) fn save_item(&self, item: LinkItem) -> Result<()> {
        let mut items = self.items()?;
        save_into_vec(&mut items, item);
        self.persist(&items)
    }

    pub(super) fn all_items(&self) -> Result<Vec<LinkItem>> {
        Ok(self.items()?.clone())
    }

    pub(super) fn import_items(&self, incoming: Vec<LinkItem>) -> Result<(usize, usize)> {
        let mut items = self.items()?;
        let mut created = 0;
        let mut merged = 0;

        for item in incoming {
            if save_into_vec(&mut items, item) {
                merged += 1;
            } else {
                created += 1;
            }
        }

        self.persist(&items)?;
        Ok((created, merged))
    }

    pub(super) fn find_item(&self, id: &str) -> Result<Option<LinkItem>> {
        Ok(self.items()?.iter().find(|item| item.id == id).cloned())
    }

    pub(super) fn delete_item_by_id(&self, id: &str) -> Result<bool> {
        let mut items = self.items()?;
        let before = items.len();
        items.retain(|item| item.id != id);
        let deleted = items.len() != before;
        if deleted {
            self.persist(&items)?;
        }
        Ok(deleted)
    }

    fn items(&self) -> Result<MutexGuard<'_, Vec<LinkItem>>> {
        self.items
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
    }

    fn persist(&self, items: &[LinkItem]) -> Result<()> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("create store directory {}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(items).context("serialize store")?;
        fs::write(&self.path, raw).with_context(|| format!("write store {}", self.path.display()))
    }
}

fn save_into_vec(items: &mut Vec<LinkItem>, mut item: LinkItem) -> bool {
    let mut merged = true;
    if let Some(existing) = items.iter_mut().find(|existing| existing.id == item.id) {
        *existing = item;
    } else if let Some(existing) = items
        .iter_mut()
        .find(|existing| existing.final_url == item.final_url)
    {
        item.id = existing.id.clone();
        item.created_at = existing.created_at;
        *existing = item;
    } else {
        items.push(item);
        merged = false;
    }
    sort_items(items);
    merged
}

fn sort_items(items: &mut [LinkItem]) {
    items.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.title.cmp(&b.title))
    });
}
