# Deploy the Palpo web administration service

The web application is a separate Node 24 service. Its container runs as the
unprivileged `node` user, with no production npm dependencies. A named volume
persists SQLite data, fleet credentials and optional account-approval settings.

Use a fixed source revision and set these values in the deployment environment:

```sh
export RELEASE_ID=<source-revision>
export PALPO_URL=http://palpo:8008
export PALPO_SERVER_NAME=matrix.example.org
export PALPO_DOCKER_NETWORK=palpo
export PUBLIC_ORIGIN=https://admin.example.org
export PALPO_TRANSPORT_ORIGIN=https://admin.example.org
```

`PALPO_URL` must be reachable from the shared Docker network. The homeserver
must be able to resolve the web service's `palpo-web-admin` network alias. Run
one companion instance per homeserver/network and one process per database.
The example publishes only `127.0.0.1:8090`; `PALPO_WEB_PORT` can change this
local port. Put it behind an HTTPS reverse proxy that preserves the public Host
header, including its port. Route `/api/fleet/v2/*` on the transport origin to
the same service; it may share the browser origin.

From `web-admin/`, build without changing running containers:

```sh
docker compose -p palpo-web-admin -f deploy/compose.yaml build
```

Activate the built image and wait for its HTTP health check:

```sh
docker compose -p palpo-web-admin -f deploy/compose.yaml up -d --wait
```

Sign in with a local Matrix administrator to authorize fleets. Fleet owners
and project owners use their own ordinary accounts. Outbound fleets need no
public callback address or reverse SSH tunnel. Existing callback fleets require
explicit migration; see [the outbound protocol](outbound-v2.md). Configure
`PALPO_CALLBACK_ORIGINS` only for legacy destinations approved by the operator.

For optional account signup, provision a protected configuration file in the
data volume and set `PALPO_ACCOUNT_CONFIG=/app/data/account-approval.json`.
The [account approval guide](account-approval.md) describes its credentials,
private administrator room and recovery rules. Keep credentials out of source
snapshots and container build arguments.

Use normal Compose stop/up operations so SQLite's process lock is released.
Retain the same Compose project name and data volume during upgrades. After a
hard crash, confirm the owning process has stopped before removing its stale
lock. Back up the database and protected configuration before upgrading; an
older image can reuse that volume only when its schema is compatible.
