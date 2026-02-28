# Palpo + matrix-bridge-feishu Bridge Example

This example shows how to run [matrix-bridge-feishu](https://github.com/palpo-im/matrix-bridge-feishu) with Palpo for bridging Feishu chats to Matrix rooms.

## Prerequisites

- Docker and Docker Compose
- A running PostgreSQL instance (or use the commented-out one in `compose.yml`)
- A Feishu custom app (Bot capability enabled)
- Palpo built and ready to run

## Setup Steps

### 1. Generate Matrix appservice tokens

Replace the placeholder tokens in `appservices/feishu-registration.yaml` and `data/config.yaml`:

```bash
openssl rand -hex 32  # as_token
openssl rand -hex 32  # hs_token
```

Both files must use the same token values.

### 2. Create and configure a Feishu app

1. Go to https://open.feishu.cn/app and create a custom app.
2. Enable **Bot** capability.
3. Add required permissions for messaging/events (send message, receive message events).
4. Configure event subscription:
   - Subscription mode: **Send notifications to developer's server**
   - Request URL: `http://<YOUR_PUBLIC_HOST>:8081/webhook`
   - Subscribe to: `im.message.receive_v1`
5. (Optional but recommended) configure **Encrypt Key** and **Verification Token** in Feishu and set the same values in `data/config.yaml`.

### 3. Configure the Feishu bridge

Edit `data/config.yaml`:

- `appservice.as_token` and `appservice.hs_token`
- `bridge.app_id` and `bridge.app_secret`
- `bridge.listen_address` (default is `http://0.0.0.0:8081`)
- `bridge.encrypt_key` / `bridge.verification_token` if enabled in Feishu

### 4. Start the Feishu bridge with Docker Compose

```bash
docker compose up -d
```

### 5. Start Palpo

From the project root:

```bash
cargo run
```

Palpo will auto-load the registration from `appservices/feishu-registration.yaml`.

### 6. Bridge a Matrix room to a Feishu chat

In a Matrix room, use the bridge command:

```
!feishu bridge <feishu_chat_id>
```

Then messages in that Matrix room will forward to the linked Feishu chat, and Feishu bot messages in that chat will forward back.

## Networking Notes

- Feishu webhook callbacks must reach `http://<YOUR_PUBLIC_HOST>:8081/webhook`.
- Palpo must reach the bridge URL configured in `appservices/feishu-registration.yaml` (`http://localhost:8080` in this example).
- If running on different hosts/networks, adjust both the registration URL and `listen_address` accordingly.

## File Structure

```
with-feishu/
├── README.md
├── compose.yml                              # Docker Compose for the bridge
├── palpo.toml                               # Palpo server configuration
├── appservices/
│   └── feishu-registration.yaml             # Appservice registration for Palpo
└── data/
    └── config.yaml                          # Feishu bridge configuration
```
