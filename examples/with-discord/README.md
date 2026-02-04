# Palpo + matrix-appservice-discord Bridge Example

This example shows how to run [matrix-appservice-discord](https://github.com/matrix-org/matrix-appservice-discord) with Palpo for bridging Discord channels to Matrix rooms.

## Prerequisites

- Docker and Docker Compose
- A running PostgreSQL instance (or use the commented-out one in `compose.yml`)
- A Discord bot token (see step 3)
- Palpo built and ready to run

## Setup Steps

### 1. Generate tokens

Replace the placeholder tokens in `appservices/discord-registration.yaml` with real ones:

```bash
# Generate as_token
openssl rand -hex 32

# Generate hs_token
openssl rand -hex 32
```

Update both `appservices/discord-registration.yaml` and `data/matrix-appservice-discord/config.yaml` with the same tokens.

### 2. Create a Discord bot

1. Go to https://discord.com/developers/applications
2. Click "New Application" and give it a name
3. Go to the "Bot" section and click "Add Bot"
4. Copy the **Bot Token** - you'll need this for config
5. Copy the **Application ID** (Client ID) from the "General Information" page
6. Under "Privileged Gateway Intents", enable **Server Members Intent** and **Message Content Intent** if needed
7. Use the OAuth2 URL Generator to invite the bot to your Discord server:
   - Select the `bot` scope
   - Select permissions: `Manage Webhooks`, `Send Messages`, `Read Message History`, `Manage Messages`
   - Open the generated URL and authorize the bot to your server

### 3. Configure the Discord bridge

Edit `data/matrix-appservice-discord/config.yaml`:

```yaml
bridge:
  domain: "127.0.0.1:6006"
  homeserverUrl: "http://127.0.0.1:6006"

auth:
  clientID: "YOUR_DISCORD_APPLICATION_CLIENT_ID"
  botToken: "YOUR_DISCORD_BOT_TOKEN"
```

### 4. Start the Discord bridge with Docker Compose

```bash
docker compose up -d
```

### 5. Start Palpo

From the project root:

```bash
cargo run
```

Palpo will auto-load the registration from `appservices/discord-registration.yaml` on startup.

### 6. Bridge a Discord channel to a Matrix room

Once the bridge is running, you can join a bridged room on Matrix using the alias format:

```
#_discord_<guildId>_<channelId>:127.0.0.1:6006
```

To find the Guild ID and Channel ID, enable Developer Mode in Discord (Settings > Advanced > Developer Mode), then right-click a server name for the Guild ID and right-click a channel for the Channel ID.

## Networking Notes

- If Palpo runs on the host and the Discord bridge runs in Docker, the bridge needs to reach Palpo. Use `host.docker.internal:6006` (Docker Desktop) or your actual host IP in the bridge config's `homeserverUrl`.
- Palpo needs to reach the bridge at the URL specified in `discord-registration.yaml`. If they're on different networks, adjust the `url` field accordingly (e.g., `http://localhost:9005` if you expose port 9005).

## File Structure

```
with-discord/
├── README.md
├── compose.yml                              # Docker Compose for the bridge
├── palpo.toml                               # Palpo server configuration
├── appservices/
│   └── discord-registration.yaml            # Appservice registration for Palpo
└── data/
    └── matrix-appservice-discord/
        └── config.yaml                      # Discord bridge configuration
```
