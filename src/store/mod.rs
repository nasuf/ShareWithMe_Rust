mod local;
mod supabase;

use std::sync::Arc;

#[cfg(test)]
use std::path::PathBuf;

use anyhow::Result;
use chrono::Utc;

use crate::{
    config::{AppConfig, StoreConfig},
    models::{LinkItem, UpdateItemRequest},
};

use local::LocalStore;
use supabase::SupabaseStore;

pub(crate) struct Store {
    backend: StoreBackend,
}

enum StoreBackend {
    Local(LocalStore),
    Supabase(SupabaseStore),
}

impl Store {
    pub(crate) async fn load(config: &AppConfig) -> Result<Self> {
        let backend = match &config.store {
            StoreConfig::Local { path } => StoreBackend::Local(LocalStore::load(path.clone())?),
            StoreConfig::Supabase { database_url } => {
                StoreBackend::Supabase(SupabaseStore::connect(database_url).await?)
            }
        };
        Ok(Self { backend })
    }

    #[cfg(test)]
    pub(crate) fn local_for_tests(path: PathBuf, items: Vec<LinkItem>) -> Self {
        Self {
            backend: StoreBackend::Local(LocalStore::from_items(path, items)),
        }
    }

    pub(crate) fn backend_name(&self) -> &'static str {
        match self.backend {
            StoreBackend::Local(_) => "local",
            StoreBackend::Supabase(_) => "supabase",
        }
    }

    async fn save_item(&self, item: LinkItem) -> Result<LinkItem> {
        match &self.backend {
            StoreBackend::Local(store) => store.save_item(item),
            StoreBackend::Supabase(store) => store.save_item(item).await,
        }
    }

    async fn all_items(&self) -> Result<Vec<LinkItem>> {
        match &self.backend {
            StoreBackend::Local(store) => store.all_items(),
            StoreBackend::Supabase(store) => store.all_items().await,
        }
    }

    async fn import_items(&self, incoming: Vec<LinkItem>) -> Result<(usize, usize)> {
        match &self.backend {
            StoreBackend::Local(store) => store.import_items(incoming),
            StoreBackend::Supabase(store) => store.import_items(incoming).await,
        }
    }

    async fn find_item(&self, id: &str) -> Result<Option<LinkItem>> {
        match &self.backend {
            StoreBackend::Local(store) => store.find_item(id),
            StoreBackend::Supabase(store) => store.find_item(id).await,
        }
    }

    async fn delete_item_by_id(&self, id: &str) -> Result<bool> {
        match &self.backend {
            StoreBackend::Local(store) => store.delete_item_by_id(id),
            StoreBackend::Supabase(store) => store.delete_item_by_id(id).await,
        }
    }

    async fn update_item_status(&self, id: &str, status: &str) -> Result<()> {
        let mut item = self
            .find_item(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("item not found"))?;
        item.status = status.to_string();
        item.updated_at = Utc::now();
        self.save_item(item).await?;
        Ok(())
    }

    async fn update_item_metadata(&self, id: &str, request: UpdateItemRequest) -> Result<LinkItem> {
        let mut item = self
            .find_item(id)
            .await?
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
        let now = Utc::now();
        item.user_edited_at = Some(now);
        item.updated_at = now;
        self.save_item(item.clone()).await?;
        Ok(item)
    }

    async fn update_item_cover(
        &self,
        id: &str,
        image_url: Option<String>,
        remote_image_url: Option<String>,
        cover_cached_at: Option<chrono::DateTime<Utc>>,
        cover_checked_at: chrono::DateTime<Utc>,
    ) -> Result<LinkItem> {
        let mut item = self
            .find_item(id)
            .await?
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
        self.save_item(item.clone()).await?;
        Ok(item)
    }
}

pub(crate) async fn save_item(store: &Arc<Store>, item: LinkItem) -> Result<LinkItem> {
    store.save_item(item).await
}

pub(crate) async fn all_items(store: &Arc<Store>) -> Result<Vec<LinkItem>> {
    store.all_items().await
}

pub(crate) async fn import_items(
    store: &Arc<Store>,
    incoming: Vec<LinkItem>,
) -> Result<(usize, usize)> {
    store.import_items(incoming).await
}

pub(crate) async fn find_item(store: &Arc<Store>, id: &str) -> Result<Option<LinkItem>> {
    store.find_item(id).await
}

pub(crate) async fn delete_item_by_id(store: &Arc<Store>, id: &str) -> Result<bool> {
    store.delete_item_by_id(id).await
}

pub(crate) async fn update_item_status(store: &Arc<Store>, id: &str, status: &str) -> Result<()> {
    store.update_item_status(id, status).await
}

pub(crate) async fn update_item_metadata(
    store: &Arc<Store>,
    id: &str,
    request: UpdateItemRequest,
) -> Result<LinkItem> {
    store.update_item_metadata(id, request).await
}

pub(crate) async fn update_item_cover(
    store: &Arc<Store>,
    id: &str,
    image_url: Option<String>,
    remote_image_url: Option<String>,
    cover_cached_at: Option<chrono::DateTime<Utc>>,
    cover_checked_at: chrono::DateTime<Utc>,
) -> Result<LinkItem> {
    store
        .update_item_cover(
            id,
            image_url,
            remote_image_url,
            cover_cached_at,
            cover_checked_at,
        )
        .await
}

#[cfg(test)]
mod tests {
    use std::{env, fs, sync::Arc};

    use uuid::Uuid;

    use super::{Store, all_items, import_items, save_item, update_item_metadata};
    use crate::models::test_item;

    #[tokio::test]
    async fn store_deduplicates_by_final_url() {
        let path = env::temp_dir().join(format!("share_with_me_test_{}.json", Uuid::new_v4()));
        let store = Arc::new(Store::local_for_tests(path.clone(), Vec::new()));

        let first = test_item("first-id", "https://example.com/a", "First title");
        let second = test_item("second-id", "https://example.com/a", "Second title");

        save_item(&store, first).await.expect("save first");
        save_item(&store, second).await.expect("save second");

        let items = all_items(&store).await.expect("read items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "first-id");
        assert_eq!(items[0].title, "Second title");

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn import_items_merges_existing_final_urls() {
        let path = env::temp_dir().join(format!("share_with_me_test_{}.json", Uuid::new_v4()));
        let store = Arc::new(Store::local_for_tests(path.clone(), Vec::new()));

        let original = test_item("original-id", "https://example.com/a", "Original");
        let imported = test_item("imported-id", "https://example.com/a", "Imported");

        save_item(&store, original).await.expect("save original");
        let (created, merged) = import_items(&store, vec![imported])
            .await
            .expect("import items");

        let items = all_items(&store).await.expect("read items");
        assert_eq!((created, merged), (0, 1));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "original-id");
        assert_eq!(items[0].title, "Imported");

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn updates_user_editable_item_metadata() {
        let path = env::temp_dir().join(format!("share_with_me_test_{}.json", Uuid::new_v4()));
        let store = Arc::new(Store::local_for_tests(path.clone(), Vec::new()));
        let item = test_item("manual-edit", "https://example.com/a", "Original");
        let original_updated_at = item.updated_at;
        save_item(&store, item).await.expect("save item");

        let updated = update_item_metadata(
            &store,
            "manual-edit",
            crate::models::UpdateItemRequest {
                category: Some("旅行".to_string()),
                keywords: Some(vec!["路线".to_string(), "攻略".to_string()]),
                notes: Some("周末整理".to_string()),
            },
        )
        .await
        .expect("update item");

        assert_eq!(updated.category, "旅行");
        assert_eq!(updated.keywords, ["路线", "攻略"]);
        assert_eq!(updated.notes, "周末整理");
        assert!(updated.updated_at >= original_updated_at);

        let persisted = all_items(&store).await.expect("read items");
        assert_eq!(persisted[0].category, "旅行");

        let _ = fs::remove_file(path);
    }
}
