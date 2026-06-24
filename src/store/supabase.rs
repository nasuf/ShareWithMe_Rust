use anyhow::{Context, Result};
use sqlx::{
    PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions, PgRow, PgSslMode},
};
use std::str::FromStr;

use crate::models::LinkItem;

const SCHEMA_SQL: &str = r#"
create schema if not exists sharewithme;

create table if not exists sharewithme.items (
    id text primary key,
    final_url text not null unique,
    item jsonb not null,
    status text not null,
    created_at timestamptz not null,
    updated_at timestamptz not null
);

create index if not exists items_created_at_idx on sharewithme.items (created_at desc);
create index if not exists items_status_idx on sharewithme.items (status);
create index if not exists items_item_gin_idx on sharewithme.items using gin (item);

alter table sharewithme.items enable row level security;
"#;

#[derive(Clone)]
pub(super) struct SupabaseStore {
    pool: PgPool,
}

impl SupabaseStore {
    pub(super) async fn connect(database_url: &str) -> Result<Self> {
        let options = PgConnectOptions::from_str(database_url)
            .context("parse Supabase database URL")?
            .ssl_mode(PgSslMode::Require);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .context("connect to Supabase Postgres")?;
        let store = Self { pool };
        store.ensure_schema().await?;
        Ok(store)
    }

    pub(super) async fn save_item(&self, mut item: LinkItem) -> Result<()> {
        if let Some(existing) = self.find_by_identity(&item.id, &item.final_url).await? {
            item.id = existing.id;
            item.created_at = existing.created_at;
        }
        self.upsert_item(&item).await
    }

    pub(super) async fn all_items(&self) -> Result<Vec<LinkItem>> {
        let rows = sqlx::query(
            "select item from sharewithme.items order by created_at desc, item->>'title' asc",
        )
        .fetch_all(&self.pool)
        .await
        .context("read Supabase items")?;
        rows.into_iter().map(item_from_row).collect()
    }

    pub(super) async fn import_items(&self, incoming: Vec<LinkItem>) -> Result<(usize, usize)> {
        let mut created = 0;
        let mut merged = 0;
        for mut item in incoming {
            if let Some(existing) = self.find_by_identity(&item.id, &item.final_url).await? {
                item.id = existing.id;
                item.created_at = existing.created_at;
                merged += 1;
            } else {
                created += 1;
            }
            self.upsert_item(&item).await?;
        }
        Ok((created, merged))
    }

    pub(super) async fn find_item(&self, id: &str) -> Result<Option<LinkItem>> {
        let row = sqlx::query("select item from sharewithme.items where id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .with_context(|| format!("read Supabase item {id}"))?;
        row.map(item_from_row).transpose()
    }

    pub(super) async fn delete_item_by_id(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("delete from sharewithme.items where id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("delete Supabase item {id}"))?;
        Ok(result.rows_affected() > 0)
    }

    async fn ensure_schema(&self) -> Result<()> {
        for statement in SCHEMA_SQL
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Err(error) = sqlx::query(statement).execute(&self.pool).await {
                self.verify_schema()
                    .await
                    .with_context(|| format!("ensure Supabase schema failed: {error}"))?;
                tracing::warn!(
                    "Supabase schema DDL was skipped because the connection role has limited privileges"
                );
                return Ok(());
            }
        }
        Ok(())
    }

    async fn verify_schema(&self) -> Result<()> {
        sqlx::query(
            "select id, final_url, item, status, created_at, updated_at from sharewithme.items limit 0",
        )
        .execute(&self.pool)
        .await
        .context("verify Supabase sharewithme.items schema")?;
        Ok(())
    }

    async fn find_by_identity(&self, id: &str, final_url: &str) -> Result<Option<ExistingItem>> {
        let row = sqlx::query(
            r#"
            select id, created_at
            from sharewithme.items
            where id = $1 or final_url = $2
            order by case when id = $1 then 0 else 1 end
            limit 1
            "#,
        )
        .bind(id)
        .bind(final_url)
        .fetch_optional(&self.pool)
        .await
        .context("find existing Supabase item")?;

        row.map(|row| {
            Ok(ExistingItem {
                id: row.try_get("id")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .transpose()
    }

    async fn upsert_item(&self, item: &LinkItem) -> Result<()> {
        let json = serde_json::to_value(item).context("serialize LinkItem for Supabase")?;
        sqlx::query(
            r#"
            insert into sharewithme.items (id, final_url, item, status, created_at, updated_at)
            values ($1, $2, $3, $4, $5, $6)
            on conflict (id) do update set
                final_url = excluded.final_url,
                item = excluded.item,
                status = excluded.status,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&item.id)
        .bind(&item.final_url)
        .bind(json)
        .bind(&item.status)
        .bind(item.created_at)
        .bind(item.updated_at)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upsert Supabase item {}", item.id))?;
        Ok(())
    }
}

struct ExistingItem {
    id: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

fn item_from_row(row: PgRow) -> Result<LinkItem> {
    let value: serde_json::Value = row.try_get("item")?;
    serde_json::from_value(value).context("parse Supabase LinkItem JSON")
}
