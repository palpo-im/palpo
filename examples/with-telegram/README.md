# Setup Steps (Docker Compose)

This example runs `postgres`, `palpo`, and `mautrix-telegram` in the same Docker Compose network.

## 1. Go to the example directory

```bash
cd examples/with-telegram
```

## 2. Generate appservice tokens

Generate two random tokens and replace placeholders consistently in:
- `appservices/telegram-registration.yaml`
- `data/mautrix-telegram/config.yaml` (after it is generated)

```bash
openssl rand -hex 32 # as_token
openssl rand -hex 32 # hs_token
```

## 3. Start Palpo + Postgres

```bash
docker compose up -d postgres palpo
```

## 4. Create mautrix-telegram database

```bash
docker compose exec postgres psql -U postgres -c "CREATE DATABASE mautrix_telegram;"
```

## 5. Start mautrix-telegram once to generate config

```bash
docker compose up -d mautrix-telegram
```

This creates `data/mautrix-telegram/config.yaml` on first run.

## 6. Configure `data/mautrix-telegram/config.yaml`

Set these key fields:

```yaml
homeserver:
  address: http://palpo:6006
  domain: "127.0.0.1:6006"

appservice:
  address: http://mautrix-telegram:29317
  hostname: 0.0.0.0
  port: 29317
  id: telegram
  bot_username: telegrambot
  as_token: <same as_token as registration.yaml>
  hs_token: <same hs_token as registration.yaml>

bridge:
  permissions:
    "*": relay
    "127.0.0.1:6006": full
    "@yourusername:127.0.0.1:6006": admin

telegram:
  api_id: <your_telegram_api_id>
  api_hash: <your_telegram_api_hash>

database: postgres://postgres:root@postgres:5432/mautrix_telegram?sslmode=disable
```

Get Telegram API credentials from https://my.telegram.org/apps.

## 7. Restart bridge after config update

```bash
docker compose restart mautrix-telegram
```

## 8. Verify services

```bash
docker compose ps
docker compose logs --tail=100 palpo mautrix-telegram
```

## 9. Use the bridge

Log in to `http://127.0.0.1:6006` with a Matrix client (Element, etc.), then message `@telegrambot:127.0.0.1:6006` and send `login` to link your Telegram account.

## Networking Notes

- Inside Compose, services talk to each other by service name (`palpo`, `postgres`, `mautrix-telegram`), not `localhost`.
- `appservices/telegram-registration.yaml` should keep:
  - `url: "http://mautrix-telegram:29317"`
