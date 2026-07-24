# ShareWithMe Backend Deployment

The backend stores production data in Supabase Postgres and runs on RackNerd through systemd.

## Required GitHub Secrets

- `RACKNERD_HOST`: server host, for example `192.210.235.115`
- `RACKNERD_USER`: SSH user with permission to write `RACKNERD_APP_DIR` and manage systemd
- `RACKNERD_SSH_KEY`: private SSH key for that user
- `SUPABASE_DATABASE_URL`: Supabase session-pooler Postgres URL for `nasuf / ShareWithMe`
- `DEEPSEEK_API_KEY`: DeepSeek API key
- `SHARE_WITH_ME_API_TOKEN`: random bearer token shared only with the Flutter build

Build or run Flutter with the matching token:

```text
--dart-define=SHARE_WITH_ME_API_TOKEN=<same-random-token>
```

Optional enrichment secrets are `INSTAGRAM_OEMBED_TOKEN`,
`AMAP_WEB_SERVICE_KEY`, `BAIDU_MAPS_AK`, and
`SHARE_WITH_ME_ASR_API_KEY`.

## Optional GitHub Variables

- `RACKNERD_APP_DIR`: default `/app`
- `RACKNERD_SERVICE_NAME`: default `sharewithme-backend`
- `SHARE_WITH_ME_BIND`: default `0.0.0.0:8080`
- `DEEPSEEK_BASE_URL`: default `https://api.deepseek.com/chat/completions`
- `DEEPSEEK_MODEL`: default `deepseek-v4-flash`
- `SHARE_WITH_ME_ASR_URL`: OpenAI-compatible transcription endpoint
- `SHARE_WITH_ME_ASR_MODEL`: default `whisper-1`

## Supabase Schema

The runtime creates the required schema automatically. The same SQL is committed in
`schema/sharewithme.sql` for review and manual recovery.

Prefer Supabase shared pooler session mode for RackNerd IPv4 compatibility:

```text
postgres://postgres.metykjthuctqcwwutrnn:<password>@aws-1-ap-northeast-1.pooler.supabase.com:5432/postgres
```
