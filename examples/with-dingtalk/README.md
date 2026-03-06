# Palpo + matrix-bridge-dingtalk Bridge Example

This example shows how to run [matrix-bridge-dingtalk](https://github.com/palpo-im/matrix-bridge-dingtalk) with Palpo for bridging DingTalk chats to Matrix rooms.

## Features

- **Bidirectional messaging** between Matrix and DingTalk
- **Rich text and card message** support with proper conversion
- **File/media sharing** including images, videos, and documents
- **User and room synchronization** with proper mapping
- **Webhook-based integration** with DingTalk APIs
- **High performance** built with Rust
- **Dead-letter support** for failed message delivery with replay capability

## Prerequisites

- Docker and Docker Compose
- A running PostgreSQL instance (or use the commented-out one in `compose.yml`)
- A DingTalk custom app (Bot capability enabled)
- Palpo built and ready to run

## Setup Steps

### 1. Generate Matrix appservice tokens

Replace the placeholder tokens in `appservices/dingtalk-registration.yaml` and `data/config.yaml`:

```bash
openssl rand -hex 32  # as_token
openssl rand -hex 32  # hs_token
```

Both files must use the same token values.

### 2. Create and configure a DingTalk app

1. Go to https://open.dingtalk.cn/app and create a custom app.
2. Enable **Bot** capability.
3. Add required permissions:
   - Message sending and receiving
   - User information access
   - File upload permissions
   - Rich text and card permissions
4. Configure event subscriptions:
   - Subscription mode: **Send notifications to developer's server**
   - Request URL: `http://<YOUR_PUBLIC_HOST>:8081/webhook`
   - Subscribe to events:
     - `im.message.receive_v1`
     - `im.message.recalled_v1`
     - `im.chat.member.user.added_v1`
     - `im.chat.member.user.deleted_v1`
     - `im.chat.updated_v1`
     - `im.chat.disbanded_v1`
5. Configure callback security:
   - Set a **Encrypt Key** and **Verification Token** (recommended)
   - Set the same values in `data/config.yaml` (`bridge.encrypt_key` and `bridge.verification_token`)

### 3. Configure the DingTalk bridge

Edit `data/config.yaml`:

- `homeserver.address` and `homeserver.domain` (should match your Palpo server)
- `appservice.as_token` and `appservice.hs_token`
- `bridge.app_id` and `bridge.app_secret` (from DingTalk app)
- `bridge.listen_address` (default is `http://0.0.0.0:8081`)
- `bridge.listen_secret` (webhook validation secret)
- `bridge.encrypt_key` / `bridge.verification_token` if enabled in DingTalk

Optional environment variables (override config values):
```bash
export MATRIX_BRIDGE_DINGTALK_DB_TYPE="sqlite"
export MATRIX_BRIDGE_DINGTALK_DB_URI="sqlite:/data/matrix-dingtalk.db"
export MATRIX_BRIDGE_DINGTALK_AS_TOKEN="your_as_token"
export MATRIX_BRIDGE_DINGTALK_HS_TOKEN="your_hs_token"
export MATRIX_BRIDGE_DINGTALK_BRIDGE_APP_ID="your_app_id"
export MATRIX_BRIDGE_DINGTALK_BRIDGE_APP_SECRET="your_app_secret"
```

### 4. Start the DingTalk bridge with Docker Compose

```bash
docker compose up -d
```

The bridge exposes:
- Port 8080: Matrix appservice API (for Palpo)
- Port 8081: DingTalk webhook endpoint (public)

### 5. Start Palpo

From the project root:

```bash
cargo run
```

Palpo will auto-load the registration from `appservices/dingtalk-registration.yaml`.

### 6. Bridge a Matrix room to a DingTalk chat

In a Matrix room, use the bridge command:

```
!dingtalk bridge <dingtalk_chat_id>
```

Then messages in that Matrix room will forward to the linked DingTalk chat, and DingTalk bot messages in that chat will forward back.

## Monitoring and Operations

### Health Check

Check bridge status:
```bash
curl http://localhost:8080/health
```

### Provisioning API

The bridge supports provisioning endpoints for operations (requires bearer token):

```bash
# Check runtime status
curl -H "Authorization: Bearer <token>" http://localhost:8080/admin/status

# List active mappings
curl -H "Authorization: Bearer <token>" http://localhost:8080/admin/mappings

# Replay dead-letters
curl -X POST -H "Authorization: Bearer <token>" \
  http://localhost:8080/admin/dead-letters/replay

# Cleanup dead-letters
curl -X POST -H "Authorization: Bearer <token>" \
  http://localhost:8080/admin/dead-letters/cleanup
```

### Environment Variables for Provisioning Tokens

```bash
export MATRIX_BRIDGE_DINGTALK_PROVISIONING_READ_TOKEN="read_token"
export MATRIX_BRIDGE_DINGTALK_PROVISIONING_WRITE_TOKEN="write_token"
export MATRIX_BRIDGE_DINGTALK_PROVISIONING_DELETE_TOKEN="delete_token"
```

### Metrics

Prometheus metrics are available at:
```bash
curl http://localhost:8080/metrics
```

## Networking Notes

- DingTalk webhook callbacks must reach `http://<YOUR_PUBLIC_HOST>:8081/webhook`
- Palpo must reach the bridge URL configured in `appservices/dingtalk-registration.yaml` (`http://localhost:8080` in this example)
- If running on different hosts/networks, adjust both the registration URL and `listen_address` accordingly
- The bridge uses `network_mode: host` to simplify networking

## Troubleshooting

### Enable Debug Logging

```bash
docker compose down
docker compose up -d
docker compose logs -f matrix-bridge-dingtalk
```

Or set `RUST_LOG=debug` in the environment.

### Common Issues

1. **Webhook signature failed**: Verify `bridge.listen_secret` matches your DingTalk app configuration
2. **Permission denied**: Check DingTalk app scopes for message send/read and file/image APIs
3. **Messages not bridging**: Verify event subscriptions are enabled in DingTalk app
4. **Database errors**: Check SQLite file permissions and disk space

### 10-Min Triage Flow

1. Check health: `curl http://localhost:8080/health`
2. Check status: `curl http://localhost:8080/admin/status`
3. Check metrics for errors: `curl http://localhost:8080/metrics | grep bridge_outbound_failures`
4. Review logs: `docker compose logs matrix-bridge-dingtalk | tail -100`

## File Structure

```
with-dingtalk/
├── README.md
├── compose.yml                              # Docker Compose for the bridge
├── palpo.toml                               # Palpo server configuration
├── appservices/
│   └── dingtalk-registration.yaml             # Appservice registration for Palpo
└── data/
    └── config.yaml                          # DingTalk bridge configuration
```
