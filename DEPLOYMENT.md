# ShareWithMe Backend Deployment

The backend stores production data in Supabase Postgres and runs on RackNerd through systemd.

## Required GitHub Secrets

- `RACKNERD_HOST`: server host, for example `192.210.235.115`
- `RACKNERD_USER`: SSH user with permission to write `RACKNERD_APP_DIR` and manage systemd
- `RACKNERD_SSH_KEY`: private SSH key for that user
- `SUPABASE_DATABASE_URL`: Supabase session-pooler Postgres URL for `nasuf / ShareWithMe`
- `DEEPSEEK_API_KEY`: DeepSeek API key

## Optional GitHub Variables

- `RACKNERD_APP_DIR`: default `/app`
- `RACKNERD_SERVICE_NAME`: default `sharewithme-backend`
- `SHARE_WITH_ME_BIND`: default `0.0.0.0:8080`
- `DEEPSEEK_BASE_URL`: default `https://api.deepseek.com/chat/completions`
- `DEEPSEEK_MODEL`: default `deepseek-v4-flash`

## Supabase Schema

The runtime creates the required schema automatically. The same SQL is committed in
`schema/sharewithme.sql` for review and manual recovery.

Prefer Supabase shared pooler session mode for RackNerd IPv4 compatibility:

```text
postgres://postgres.metykjthuctqcwwutrnn:<password>@aws-1-ap-northeast-1.pooler.supabase.com:5432/postgres
```
