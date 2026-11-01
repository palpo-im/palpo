# Hagency application retirement

[中文](HAGENCY_MIGRATION.zh-CN.md)

Palpo retains the Matrix client/server and federation protocols, rooms, events,
App Services and homeserver management APIs. Hagency fleet installation, pairing,
coordinator/project approvals, resource catalogs and budgets, Agent lifecycle,
Inbox notifications and runtime relay transport are no longer implemented here.
The `palpo-hagency-contract` crate, the Hagency portions of `palpo-operations`,
and legacy Node migration fixtures have been removed.

Ordinary account signup approvals remain in the independent
[account approval service](../crates/operations/README.md), together with its
request/receipt API, worker and scoped native administration sessions. Generic
App Service registration, enable/disable/delete, callback URL compare-and-set,
identity management and the mountable `MatrixServer` library remain available.
Matrix device-assertion validation, refused-transaction retries, credential
redaction and membership-profile sync fixes are retained.

## Existing deployments

Removing source does not stop deployed services or delete SQLite/PostgreSQL
data, accounts, rooms, credentials or pending requests. Keep the previous
executable/image pinned if those workflows are still needed, and back up its
private database and configuration before replacing it.

The reduced account service can retain its existing signup state, server/key
bindings and writer fences. Unknown historical JSON fields and tables remain
stored, but retired workflows are never executed. Stop the previous writer
before opening the same database. Removed Hagency CLI commands, capability
names, association endpoints and relay/fleet APIs are unavailable in this build.

Hagency business applications belong in their own project, such as
[hagency-server](https://github.com/chrislearn/hagency-server). This change does
not deploy that project, validate a replacement revision, migrate business data
or redirect clients to it. Review and validate any replacement separately before
switching existing consumers. Matrix administrator authority alone does not
authorize external resource allocations.
