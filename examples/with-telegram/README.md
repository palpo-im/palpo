# Setup Steps

## 1. Generate tokens
Replace the placeholder tokens in telegram-registration.yaml with real ones:


### Generate as_token
openssl rand -hex 32

### Generate hs_token
openssl rand -hex 32

## 2. Start mautrix-telegram with Docker Compose

cd crates/server
docker compose -f compose-telegram.yml up -d
This will create a data/mautrix-telegram/ directory with a default config.yaml on first run.

## 3. Configure mautrix-telegram
Edit crates/server/data/mautrix-telegram/config.yaml:

homeserver:
    address: http://host.docker.internal:6006   # or your host IP
    domain: "127.0.0.1:6006"                     # must match server_name

appservice:
    address: http://mautrix-telegram:29317
    hostname: 0.0.0.0
    port: 29317
    id: telegram
    bot_username: telegrambot
    as_token: <same as_token from registration.yaml>
    hs_token: <same hs_token from registration.yaml>

bridge:
    permissions:
        "*": relay
        "127.0.0.1:6006": full
        "@yourusername:127.0.0.1:6006": admin

telegram:
    api_id: <your_telegram_api_id>
    api_hash: <your_telegram_api_hash>

database: postgres://postgres:root@postgres:5432/mautrix_telegram?sslmode=disable

## 4. Create the mautrix-telegram database

docker exec -it <postgres_container> psql -U postgres -c "CREATE DATABASE mautrix_telegram;"

## 5. Get Telegram API credentials
Go to https://my.telegram.org/apps to create an app and get api_id and api_hash.

## 6. Restart everything

docker compose -f compose-telegram.yml restart mautrix-telegram

## 7. Start Palpo

cargo run
Palpo will auto-load the registration from appservices/telegram-registration.yaml on startup.

## 8. Use the bridge
Log in to your Palpo instance with a Matrix client (Element, etc.), then message @telegrambot:127.0.0.1:6006 and send login to link your Telegram account.

Key consideration: Since Palpo runs on the host and mautrix-telegram runs in Docker, the bridge needs to reach Palpo. Use host.docker.internal:6006 (Docker Desktop) or your actual host IP. Conversely, Palpo needs to reach the bridge at the URL in the registration file -- if they're on different networks, you may need to adjust the url in telegram-registration.yaml to something Palpo can reach (e.g., http://localhost:29317 if you expose port 29317 from the container).