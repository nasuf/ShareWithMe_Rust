# Backend architecture

The backend turns a shared link into a durable content snapshot, organises it
(summary, category, keywords) and serves the collection to the Flutter app.

## Layers

```
src/
├── main.rs            startup: config, store, HTTP client, router (+ --probe CLI)
├── api/               HTTP layer — routing, auth, request handlers
├── extraction/        link -> LinkMetadata snapshot (platform-agnostic pipeline)
├── platforms/         one module per supported platform, behind `Platform`
├── analysis/          snapshot -> summary/category/keywords (DeepSeek or heuristics)
├── store/             persistence (local JSON file or Supabase Postgres)
├── models.rs          API and storage types (LinkItem, LinkMetadata, ...)
├── content.rs         snapshot content types, fingerprints, refresh policy
├── links.rs           URLs in requests: extraction from share text, safety checks
├── network.rs         HTTP redirect policy (never follows into private networks)
├── render.rs          headless browser bridge (tools/render_extract.mjs)
├── text.rs            text normalisation
├── digest.rs          collection digest
└── probe.rs           `--probe` CLI for live platform checks
```

Dependencies point downwards: `api` → `extraction`/`analysis`/`store` →
`platforms`/`models`. `extraction` and `platforms` call each other only through
the hooks below; nothing outside `platforms/` matches on platform names.

## Extraction pipeline

`extraction::extract_metadata` runs these steps; each platform-specific step is
a method on `platforms::Platform`:

| Step | Code | Platform hook |
| --- | --- | --- |
| 1. Seed from share-sheet text | `extraction/mod.rs` | — |
| 2. Fetch page (mobile UA where needed) | `extraction/fetch.rs` | `fetch_user_agent` |
| 3. Parse page: platform parser, else generic Open Graph/body | `extraction/html.rs` | `parse_page`, `merge_embedded_state` |
| 4. Structured data (JSON-LD, page state) | `extraction/structured_data.rs` | `enrich_from_html` |
| 5. Public platform API | — | `fetch_api` |
| 6. Headless browser render | `extraction/rendered.rs` | `needs_browser_render` |
| 7. oEmbed / platform APIs / yt-dlp / ASR | `extraction/{oembed,video,asr}.rs` | `enrich_remote` |
| 8. Replace login/visitor shells with share text | `extraction/shell.rs` | `shell_reason` |
| 9. Identity, fingerprint, completeness grade | `extraction/finalize.rs` | `identity`, `profile`, `missing_content_hint` |

Completeness (`finalize.rs`): **complete** (80 + 5 author + 5 cover) is never
refreshed automatically; **partial** (50/20) and **blocked** (40/15) are retried
after the platform's refresh interval. See `PLATFORM_SUPPORT.md` for the
per-platform contract.

## Adding a platform

1. Add a label in `platforms/detect.rs` (host → display name).
2. Create `platforms/<name>.rs` with the hooks it needs — usually `identity`,
   plus `embedded_state`, `enrich_remote` or `shell_reason`.
3. Add a `Platform` variant and wire it in the `match` arms of
   `platforms/mod.rs` (`from_label`, `profile`, and each hook you implemented).
4. Add unit tests in the platform file, a case in `platforms/identity.rs`, and
   a live URL in `scripts/live_platform_probe.sh`.

Shared helpers for adapters: `LinkMetadata::{set_fact, add_image,
set_cover_image, set_body, ...}` in `extraction/snapshot.rs`, and the JSON
helpers re-exported from `extraction`.

## HTTP API

`api/mod.rs` only builds the router. Handlers are grouped by responsibility:
`items` (read/edit/delete), `ingest` (save links), `refresh` (re-fetch and
re-analyse without losing saved content), `media` (cached images), `archive`
(page screenshots), `library` (stats, digest, export, import), `auth` (bearer
token).

## Testing

```bash
cargo test --locked                      # unit + HTTP-level tests, no network
cargo llvm-cov --summary-only            # line coverage (CI floor: 70%)
scripts/live_platform_probe.sh           # live contract against real platforms
```

Tests live next to the code they cover. Network-dependent paths are tested
against local fixture servers (`127.0.0.1` is allowed only in test builds).
Not unit-tested by design: `store/supabase.rs` (needs Postgres),
`render.rs` (needs Node + a browser) and `extraction/asr.rs` (needs an ASR API);
the live probe covers them end to end.
