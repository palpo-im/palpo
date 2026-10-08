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
PDUs. New-invite authorization reads permission, current joined memberships and
room-state frame IDs in a short, read-only, repeatable-read transaction. Sync uses
the coordinated read phase described below. Rule checks use immutable frames,
so they cannot combine old memberships with newer rules.

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
Both sync versions capture their existing response boundary before invitation reads.
V3 passes the same cursor already used to validate the client's `since` token; sliding
sync captures its presence/profile/inbox window once. Invitation handling never reads
the global sequence again or advances this boundary. The existing stream locks fence
their respective streams; they do not fence every writer of `occur_sn_seq`. In
particular, an ordinary event may reserve its position before publishing its timeline
row on another process, whose in-memory sequence queue is invisible to this one.
An invitation read must not move the token beyond such a later reservation.

With MSC4494 enabled, one connection holds shared user/room advisory locks while
reading the current invitation decision inputs.
It discovers unadmitted inviters, then locks all relevant users in PostgreSQL lock-key order,
including the recipient. This freezes policy, ignored senders, invitations, admissions
and memberships. If a new sender appeared before the recipient lock was acquired,
the transaction releases its locks and retries without publishing a decision.
It then locks the shared rooms' current frames in lock-key order. A common order also
prevents shared readers from forming a wait cycle behind queued writers.
Shared rooms without a frame are included, so their first qualifying rule is covered.
Policy and ignore-list changes, membership replacements, invitation-state updates and
admissions take an exclusive lock for their user; frame publication takes one for its
room. All builds participate in these writes, including feature-disabled instances.
Writers hold one scope per transaction and never acquire stream locks within it.
Invitation readers take user, then room locks, after the cursor transaction has ended.
The transaction uses READ COMMITTED so a writer that commits while a lock is awaited
is visible to subsequent reads; the scopes then keep the decision inputs stable until
the inventory has been captured. Only database reads run in this phase. Immutable rule
decoding, response construction and delivery recording run after it releases the
locks. Shared readers can overlap, and unrelated users and rooms keep writing.
Invitation events are restricted to the original response window, while permission,
ignore lists, shared memberships and join rules are evaluated at this consistent
current-state read point. Current membership rows replace earlier joins, so filtering
their sequence positions cannot reconstruct historical relationships. A join-to-join
profile update after the cursor must not hide an otherwise current qualifying join;
a leave or rule change committed before the decision must affect eligibility.
This distinction also lets a newly qualifying relationship expose an older retained
invite without moving the cursor over unrelated event publications. Changes after
the captured decision do not revoke an invitation selected from that decision.
Stream reads, next tokens and first-delivery records all retain the original boundary.
With MSC4494 enabled, v3 still evaluates invitations for the normal `C + 1` next
token while the event stream is idle. Current state or a delayed admission can change
without a new sequence allocation. The response may retain that same token while
delivering an invitation once; this device's delivery record then prevents repeats.
More distant future tokens keep the existing early-return behavior.
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
