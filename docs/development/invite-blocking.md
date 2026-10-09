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
room. Resolved-state publication locks all changed users in physical-key order,
then the room, and commits the membership replacements and frame together on one
connection. A failed publication rolls back both, including admission deletions.
User/settings preparation runs before these locks; derived statistics and federation
side effects run after commit. All builds participate, including feature-disabled
instances. Writers never acquire stream locks within the publication transaction.
Invitation readers take user, then room locks, after the cursor transaction has ended.
The transaction uses READ COMMITTED so a writer that commits while a lock is awaited
is visible to subsequent reads; the scopes then keep the decision inputs stable until
the inventory has been captured. Only database reads run in this phase. Immutable rule
decoding, response construction and delivery recording run after it releases the
locks. Shared readers can overlap, and unrelated users and rooms keep writing.
Remote joins import the returned auth chain and state as checked outliers, without
publishing historical membership changes or a provisional local join. The returned
resolved state is published together before the actual join event is appended, so
invitation decisions cannot combine a premature join with the previous room rules.
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
without a new sequence allocation. The event component stays at `C + 1`; an
independent `_i<batch>` response identity distinguishes newly offered invitations.
Constructing or sending a response does not acknowledge it. Requests repeating the
old token replay unconfirmed invitations; requests echoing the offered token confirm
only that response's selected membership IDs for that user and device.
More distant future tokens keep the existing early-return behavior.
Sliding sync checks both first-delivery obligations and list positions before
returning an idle, count-only response. It compares the filtered, sorted requested
ranges against each connection's last acknowledged response, preserving exact indices
and order. An invitation becoming visible outside a range can displace rooms inside
it without allocating a sequence number. That range still receives new `SYNC` ops,
even when its newly selected rooms were already delivered to this device. The
connection's JSON cache persists these windows across instances; older caches
that tracked offered windows receive one refresh before using acknowledgements.
All builds preserve and record
the windows. With MSC4494 enabled, changed windows receive an independent `_w`
response identity and remain pending until that exact token returns. Lost responses
and retries on another instance refresh the same windows. Count-only responses do
not replace them. The handler compares returned ops with the preceding
windows so long polling cannot swallow a changed list with no room or count updates.
Unchanged `SYNC` ops remain empty for long polling. An invitation selected by a list
range or explicit subscription also bypasses the idle return if first delivery is
owed. Rendering and recording use the captured invitation identity and original
cursor. Invitations outside the selection remain unrecorded; once the selected
positions have been acknowledged, they do not cause repeated full responses.
Global admission preserves the user's visibility decision; first delivery is
recorded separately for each device. An unseen device receives an admitted invite
even if another device's delayed admission commits behind its cursor. Once that
device acknowledges its response and advances past its delivery position, the
invite is not replayed. Echoing another device's response cannot confirm delivery.
Batch IDs use a separate database sequence, so they cannot skip event publications.
Concurrent or partially selected responses acknowledge their own exact subsets,
instead of treating a higher batch ID as confirmation of every earlier response.
Expired batch records cause a safe redelivery. Existing first-delivery positions
and global admissions are preserved.
Additional qualifying rooms, rule changes and join-to-join profile updates cannot
restamp that device's delivery. Local join-to-join profile updates remove an unused
client-supplied restricted-join authorizer before hashing and signing, so the event
and subsequent leaves can be accepted by other homeservers. Federation restricted
join checks return a definitive forbidden error when all allowed rooms are known
and the user is absent; unknown room state still permits another server to assist.
Incoming federation transactions count all raw EDUs before decoding them. A
malformed ephemeral update is skipped individually, so it cannot prevent the
transaction's persistent membership events or valid EDUs from being processed.
An incapable restricted-room resident allows candidate failover before checking
its potentially delayed view of allowed-room membership. Remote invitation
rejection routes through servers in the invitation state, including domainless
room IDs. An out-of-band invitation can only be rescinded by its original
inviter while the receiving server is not participating; this is checked inside
the membership/frame publication transaction. Remote knock summaries include
the accepted local knock and cannot overwrite a replacement membership.
Membership `prev_content` uses the event's exact before-state frame, rather than
assuming consecutive frame IDs. Newly shared membership is included in device
list change notifications. Imported auth/state outliers are not timeline history:
backfill promotes them without publishing historical memberships, current room
state, notifications, commands or sticky windows. Federation backfill responses
include their requested seed events so page boundaries do not skip history.
History fetching also tries participating servers when the power-level user map
does not name an administrator, as in room v12. Rebuilding a just-joined sync
timeline after backfill retains the response's original upper boundary, so it
includes subsequent live profile updates without consuming future events.
Repeated remote knocks contact the resident again because a remote rejection
may not have been federated to the knocking server. Device-list responses omit
devices awaiting identity-key upload; persisted device-list revisions match
outgoing EDUs and resync responses.
Account-data
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
