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

The `palpo-hagency-contract` and `palpo-operations` packages proposed in draft
[#508](https://github.com/palpo-im/palpo/pull/508) have not been merged into
Palpo main. Their implemented contract/workflow code has been adapted into
`hagency-contract` and `hagency-operations` in hagency-server. Runtime consumers
must switch to that repository before the draft's Palpo dependency can be retired.
Those packages must not be added to the Palpo workspace by later merges.

## Existing installations

This removal changes source ownership; it does not delete any running service,
SQLite database, Matrix account, App Service credential, room or queued command.
Existing deployed images remain available at their pinned revisions.

1. Keep the working `web-admin` image pinned while preparing the replacement.
   Back up its private SQLite database and the separate Palpo/Pasion databases.
2. Select a reviewed hagency-server revision that contains the contract and
   Operations migration. The cleanup PR is a draft until that revision is
   published and consumers have been checked.
3. Configure Hagency, Palpo and Pasion databases separately. Hagency business
   state belongs in the Hagency PostgreSQL database. Palpo's database is not an
   import destination for the old application's SQLite state.
4. Plan a reviewed state migration for existing fleet IDs, credentials, pending
   requests and leased deliveries. The Rust implementation does not provide an
   automatic Node SQLite converter or automatically convert the legacy Inbox
   into coordinator authority. Do not reconnect by blindly creating duplicate
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
