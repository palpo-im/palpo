# Hagency application ownership and migration

[中文](HAGENCY_MIGRATION.zh-CN.md)

Palpo implements the Matrix Client-Server and federation protocols, rooms,
events, App Services and homeserver management APIs. Hagency-specific fleet
onboarding, project requests, resource/coordinator authorization, Agent budgets,
Inbox and runtime command delivery belong to
[hagency-server](https://github.com/chrislearn/hagency-server).
The separate Node `web-admin` application and its CI job are removed here.
Generic App Service registration, pause/revoke, URL compare-and-set, identity
retirement, and Matrix administration APIs remain in Palpo.

The `palpo-hagency-contract` and `palpo-operations` packages have since landed
in Palpo main and remain in this branch, including their migration and writer
ownership checks. Their removal or relocation requires a separate accepted
cross-project migration; resolving this cleanup does not retire those consumers.
The Hagency ownership change remains a separate proposal until the replacement
and runtime/client consumers are accepted. Retiring the Node source does not
depend on that relocation: the Rust Operations service already remains here.

The Node application is removed, but its entry point and SQLite migration dependencies
are retained under `crates/operations/tests/fixtures/legacy_node/`. The Rust
Operations CI still exercises legacy state preservation and writer fencing.
These fixtures are not a deployable Node service.

## Existing installations

This removal changes source ownership; it does not delete any running service,
SQLite database, Matrix account, App Service credential, room or queued command.
Existing deployed images remain available at their pinned revisions.

1. Keep the working `web-admin` image pinned while preparing the replacement.
   Back up its private SQLite database and the separate Palpo/Pasion databases.
2. For Node-to-Rust adoption, select a reviewed Palpo Rust Operations revision
   and follow its SQLite handoff and acceptance guide. A later move to
   hagency-server additionally requires a reviewed replacement revision and
   consumer checks; this source cleanup does not perform that move.
3. Keep the Operations SQLite database separate from Palpo's Matrix database.
   Under the proposed hagency-server deployment, configure Hagency, Palpo and
   Pasion databases separately; business state would move to Hagency PostgreSQL.
   Palpo's Matrix database is not an import destination for the old application's
   SQLite state.
4. Plan a reviewed state migration for existing fleet IDs, credentials, pending
   requests and leased deliveries. Moving to hagency-server does not provide an automatic SQLite-to-PostgreSQL
   migration. Palpo Rust Operations separately supports the documented Node
   SQLite adoption path; that does not automatically turn old Inbox records
   into coordinator authority. See [the Operations migration guide](../crates/operations/README.md). Do not reconnect by blindly creating duplicate
   fleets, discard pending operations, or run both application writers together.
5. Verify Pasion sign-in, Padmin management, fleet delivery and the new Inbox
   against the chosen runtime/client versions before routing traffic to the
   replacement. Preserve backups and the pinned old deployment for rollback.

Hagency's canonical native endpoint is `/_hagency/miniapp/v1/`; its host retains
the `/_palpo/miniapp/v1/` compatibility alias during client migration.
Those routes are application endpoints and are not mounted by standalone Palpo.
Pasion remains the identity/OAuth component; hagency-rs remains responsible for
runtime quota reservation, Agent execution and usage receipts. A Matrix server
administrator role is not a Hagency resource delegation.

Online server association and delegation issuance, real hagency-rs provisioning,
chat, and Codex/Claude quota execution require separate cross-project acceptance.
The upstream draft and migrated fixture tests do not establish those workflows
as production-ready. For the current implementation and release boundaries, see
the bilingual Operations guide in the selected hagency-server revision.
