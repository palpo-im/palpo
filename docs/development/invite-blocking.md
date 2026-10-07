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
room state does not establish a qualifying room. Membership and join rules are
checked against current local state for each request, including federated inviters.

The check applies before local invite persistence (including `createRoom` and
membership state writes), to federation invite endpoints, and to incoming invite
PDUs. Retained invitations are filtered in ordinary and sliding sync, including
explicit sliding-sync room subscriptions. Incremental sync re-exposes retained
invitations after relevant membership or join-rule changes without repeating them
on subsequent unchanged syncs. Membership reads and appservice delivery
retain their existing behavior. Disabling the Cargo feature treats the experimental
action as unknown and omits its support flag.

Run the database regressions against an empty, dedicated PostgreSQL database:

```sh
PALPO_TEST_DATABASE_URL=postgres://... cargo test -p palpo --features unstable-msc4494 database_ -- --ignored --test-threads=1
```

The test harness applies migrations and refuses a database that already has tables.
