# Invite blocking

The stable `m.invite_permission_config` global account data supports
`{"default_action":"block"}` in every build. Rejected invitations return HTTP 403
with `M_INVITE_BLOCKED`. Missing, invalid, deleted, and unknown actions retain the
normal invite behavior, including existing ignore-list checks.

Build with `cargo build -p palpo --features unstable-msc4494` to enable
[MSC4494](https://github.com/matrix-org/matrix-spec-proposals/pull/4494).
The server then advertises `uk.timedout.msc4494` in `/versions` and recognizes
`{"default_action":"uk.timedout.msc4494.deny_public"}`. Both inviter and recipient
must currently be joined to at least one common room with an `invite`, `knock`,
`restricted`, or `knock_restricted` join rule. Reserved `private` and custom join
rules do not establish eligibility.
Room-directory visibility and encryption do not determine eligibility. Unknown
or malformed room state does not establish a qualifying room; malformed state
in one shared room does not prevent another room from qualifying. Membership and
join rules are checked against current local state for each request, including federated inviters.

The check applies before local invite persistence (including `createRoom` and
membership state writes), to federation invite endpoints, and to incoming invite
PDUs. Retained invitations are filtered in ordinary and sliding sync, including
explicit sliding-sync room subscriptions, until they first qualify. That decision
is persisted for the current invitation membership in `room_invite_admissions`.
Once admitted, a pending invite remains visible even if the users later stop
sharing a qualifying room. New invitations still require a current relationship.
This preserves the same view for incremental clients, fresh syncs, and other devices
without treating loss of a shared relationship as an invitation withdrawal.

Incremental sync can expose a previously hidden invitation when it first qualifies.
Adding another qualifying room, qualifying-to-qualifying rule changes, and profile
updates do not replay an already admitted invite. Stable `block` and ignore lists
still suppress invitations; the existing account-data replay behavior is preserved.
The migration ties admissions to `room_users.id` with cascading deletion: replacing
or ending an invitation removes its admission, so new invitations cannot inherit
old trust. The decision survives restarts and is shared across server instances.

Each sync batches membership checks for distinct, not-yet-admitted inviters, reads
the recipient's joins once, and caches shared-room rules for that request. Sliding
sync reuses its invitation snapshot. Membership reads and appservice delivery retain
their existing behavior. Disabling the Cargo feature treats the experimental action
as unknown and omits its support flag.

Run the database regressions against an empty, dedicated PostgreSQL database:

```sh
PALPO_TEST_DATABASE_URL=postgres://... cargo test -p palpo --features unstable-msc4494 database_ -- --ignored --test-threads=1
```

The test harness applies migrations and refuses a database that already has tables.
