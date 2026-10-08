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
PDUs. Both authorization and sync read permission, current joined memberships and
room-state frame IDs in short, read-only, repeatable-read transactions. Rule checks
use those immutable frames, so they cannot combine old memberships with newer rules.

Sync reads each invitation's membership row ID, event ID, sender, stream position,
stripped state, existing admission and this device's first delivery together. The same captured state is used
in both ordinary and sliding-sync responses; rendering does not reload by room ID.
Inventory reads have no side effects. Only invitation states included in a complete
response receive admission records, after list filters, ranges and subscriptions.
Count-only responses and excluded rooms do not admit unseen invitations.

The pending-invitation lifecycle is:

| State | Sync behavior |
| --- | --- |
| Unadmitted, currently ineligible under `deny_public` | Hidden; no admission is written. |
| Unadmitted, currently eligible or allowed | May be returned; admission is written only if selected in the completed response. |
| Already admitted | Remains visible while pending, even if qualification is later lost; stable `block` and ignore lists still suppress it. |
| Ended or replaced | The old admission is deleted; the new event must establish its own eligibility. |

Eligibility is a current-state predicate, not a membership/history-change position.
First delivery uses the response's captured stream position, so eligible invitations
committed after a client passed their event position can still be delivered once.
The invitation transaction reads current state without publishing a sequence value.
After these reads, each sync version captures its response boundary through its
existing PostgreSQL advisory stream locks. This includes observed changes while
waiting for earlier presence, device-inbox and applicable sticky/profile writes to
commit. PostgreSQL sequences expose uncommitted allocations, so an unlocked sequence
read, even inside repeatable read, cannot provide a safe sync cursor. Both sync
versions prepare invitations before other response data and use the protected
boundary for stream reads, the next token and first-delivery records.
Current membership rows replace earlier joins, so filtering them at an older cursor
cannot reconstruct historical membership. A leave, profile update, permission change
or join-rule change observed by the snapshot must belong to the response boundary.
Global admission preserves the user's visibility decision; first delivery is
recorded separately for each device. An unseen device receives an admitted invite
even if another device's delayed admission commits behind its cursor. Once that
device advances past its own delivery position, the invite is not replayed.
Additional qualifying rooms, rule changes and join-to-join profile updates cannot
restamp that device's delivery. Account-data
replay behavior is preserved. Invites returned while allowing all senders are also
admitted, so enabling `deny_public` later preserves invitations already shown.
New invitations still require a current relationship.

`room_invite_admissions` is tied to `room_users.id` with cascading deletion.
Before recording delivery, the server locks and verifies both the captured membership
ID and event ID. A replaced invitation cannot inherit a stale snapshot's admission.
Ordered, atomic upserts preserve global admission and each device's first delivery
across concurrent devices and instances. The JSONB device map merges with existing
keys taking precedence. Decisions survive restarts.

Checks batch distinct unadmitted inviters and cache each immutable shared-room frame
within the request. Membership reads and appservice delivery keep their existing
behavior. Disabling the Cargo feature treats the experimental action as unknown and
omits its support flag. Without admission tracking, stable sync retains its existing
event window, captured through stream locks before reading invitations.

Run the database regressions against an empty, dedicated PostgreSQL database:

```sh
PALPO_TEST_DATABASE_URL=postgres://... cargo test -p palpo --features unstable-msc4494 database_ -- --ignored --test-threads=1
```

The test harness applies migrations and refuses a database that already has tables.
