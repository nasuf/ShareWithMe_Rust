# Platform support contract

ShareWithMe stores a durable content snapshot, not only a bookmark. Every item
keeps the source URL, canonical platform ID, share-sheet text, semantic original
content blocks, media URLs and cached images, source facts, extraction state,
AI output, and the next refresh time.

## Extraction matrix

| Platform | Primary public source | Persisted original content | Important fallback | Refresh |
| --- | --- | --- | --- | --- |
| 小红书 | Embedded note state | Caption, author, note images, source ID | Browser render, then share-sheet text | 12 h |
| YouTube | `yt-dlp` metadata and captions | Description or full caption track, channel, cover, duration and counts | Optional ASR when no caption exists | 24 h |
| 今日头条 | Public HTML and browser render | Article paragraphs, headings, lists and images | Share-sheet text when anti-bot verification wins | 12 h |
| 哔哩哔哩 | Public view API plus `yt-dlp` | Description or caption, UP owner, cover, duration and counts | Optional ASR when no caption exists | 24 h |
| 知乎 | Public answer/article state and answer API | Answer/article body with semantic blocks and author | Share-sheet text if access verification blocks both | 24 h |
| 微博 | Public status API and embedded state | Post text, author, images and source facts | Share-sheet text if visitor verification blocks the API | 6 h |
| 抖音 | Public SEO preview plus `yt-dlp` | Caption or subtitle/transcript, creator, cover and duration | Optional ASR; share-sheet caption remains available | 12 h |
| X | Official public oEmbed plus `yt-dlp` for video | Post text, author and media metadata | Share-sheet text for unavailable/private posts | 6 h |
| Instagram | Public OG/embedded state and optional oEmbed token | Caption, author and media preview | `yt-dlp`, then share-sheet text for permitted posts | 12 h |
| 豆瓣 | Public subject API and structured data | Subject/review/book/movie text, rating and cover | Public HTML | 72 h |
| 高德地图 | Public place preview and optional Web Service API | Name, address, coordinates, rating and image | Share-sheet place text | 24 h |
| 百度地图 | Canonical UID and optional Place API | Name/address/coordinates/phone when public or API-backed | Share-sheet place name/address is required for gated short links | 24 h |
| 京东 | Product HTML, JSON-LD and page state | Product title, description, category, image and price facts when exposed | Share-sheet product text | 12 h |
| 淘宝 | Product structured data when public | Product title/specification/media when exposed | System share-sheet title/specification is the reliable path under login/risk control | 12 h |
| 天猫 | Product structured data when public | Product title/specification/media when exposed | System share-sheet title/specification is the reliable path under login/risk control | 12 h |
| 微信读书 | Public book detail HTML and structured data | Book title, author, introduction and cover | Share-sheet book text | 72 h |
| 微信公众号 | Server-rendered `#js_content` or inline `cgiData` | Article/picture-message body blocks, account, author, publish time and images | Browser render, then share-sheet text | 24 h |

## Video speech

Video adapters use source captions first. If no caption track is available, an
OpenAI-compatible speech-to-text endpoint can be enabled with
`SHARE_WITH_ME_ASR_URL`; the downloaded audio is bounded by duration and size,
stored only in a temporary directory, uploaded for transcription, and deleted.
Without ASR credentials, ShareWithMe truthfully analyzes the public caption,
title and description instead of labeling them as a full spoken transcript.

Private, deleted, DRM-protected, region-restricted, or login-only media is not
bypassed. A Netscape cookie file may only be supplied for content the user is
authorized to access.

## Refresh and preservation rules

1. Flutter asks the backend to refresh stale cards when they become visible and
   gently refreshes at most two stale cards after a collection load.
2. The backend resolves the canonical source again and reruns the platform
   adapter. AI analysis runs only when the content fingerprint changes.
3. A partial or blocked refresh never replaces a more complete prior snapshot.
4. User-edited category, keywords, and notes survive automatic refresh.
5. Up to 40 source images are converted to bounded JPEG snapshots under
   `/media`; the remote URL is retained so a later refresh can recover it.
   Cached files are keyed by the URL without expiring signature params
   (`x-expires`, `x-signature`, ...), so a re-signed CDN URL reuses the cache.
   Each download sends the platform's own Referer (WeChat `mmbiz.qpic.cn`
   needs `https://mp.weixin.qq.com/`); a hotlink placeholder answered with
   `x-errinfo: banned` is rejected instead of cached. Cached file names end in
   a hash of the image bytes, because `/media` is served as immutable.
6. Browser-rendered pages (Toutiao, WeChat fallback, ...) return the article
   container HTML as well as its text; it is parsed into the same semantic
   blocks as plain HTTP pages (inline images, bold-paragraph subheadings,
   captions) when it covers at least 70% of the rendered text.
7. Only incomplete snapshots (partial, blocked, legacy) are refreshed
   automatically. A complete one (completeness >= 80, including one kept as
   `stale`) gets a `refresh_after` ten years out, so the saved version is not
   replaced by a later edit, deletion, block page or redesign of the source.
   The per-platform interval in the table applies to incomplete items only;
   the detail page's refresh button (`?force=true`) still re-fetches anything.

## Viewing modes

The app offers three views of a saved item:

| Mode | Source | When it works |
| --- | --- | --- |
| Reader (default) | `original_content` blocks + cached media | Always, offline, after the link dies |
| Original page | Live `final_url` in an in-app WebView | While the source is online |
| Archive | `archive`: JPEG screenshot of the article container | After the first successful capture |

The archive is captured in the background after save (and on the next refresh
for older items) by `tools/render_extract.mjs --snapshot`, capped at 16000px
tall. Blocked/captcha pages are never archived. `POST /api/items/{id}/archive`
re-captures on demand; each capture gets a new file name because `/media` is
served as immutable.

## Production boundary

Set `SHARE_WITH_ME_API_TOKEN` on any internet-facing backend and build/run the
Flutter client with the same value:

```bash
flutter run --dart-define=SHARE_WITH_ME_API_TOKEN='<random-token>'
```

All `/api` routes then require `Authorization: Bearer ...`; `/health` and
unguessable cached media paths remain public so app startup and image rendering
work normally. Submitted links and imported backups reject localhost, private,
link-local, multicast, and non-HTTP destinations, and redirects to those literal
addresses are not followed.

## Verification

Run deterministic adapter and persistence tests:

```bash
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Run live public-source contracts for all 17 target platforms:

```bash
scripts/live_platform_probe.sh
```

The live script requires complete snapshots for public-readable sources. It
also exercises URL-only diagnostics and strict share-sheet fallbacks for Baidu
Maps, Taobao, and Tmall, whose public pages commonly return login or risk-control
shells. Live URLs can change or disappear; replace a dead fixture with another
public item from the same platform rather than weakening the assertions.
