# Audited Complement fixture compatibility

These patches apply only to the pinned Complement revision in
`.github/workflows/complement.yml`. CI checks patch applicability before applying
them, then runs the runner unit tests. Do not silently rebase patches when
updating Complement: review the upstream behavior first.

## device-list-query-keys.patch

The three `TestDeviceListsUpdateOverFederation` connectivity cases already
require exact device-list notifications after a new device uploads its keys.
They now also query Alice's new device from Bob's remote homeserver after normal
connectivity, reconnection and a server stop/restart. The query must have no
server failures, return the correct user/device IDs, and contain both uploaded
public keys with their exact values. Map lookups preserve IDs containing dots
without treating them as JSON path expressions.

This completes the remote-key-content assertion requested by
[#573](https://github.com/palpo-im/palpo/issues/573). Existing strict notification
assertions and all three entries in `required_pass/device-list-versions.json`
remain mandatory. The patch applies to the pinned revision; it strengthens the
fixture and does not mask a server failure.

## public-rooms-pagination.patch

`TestRoomState/Parallel/GET_/publicRooms_lists_newly-created_room` creates a
public room alongside thirteen other public-room fixtures, then searches only
the default first directory page. Palpo defaults to ten results and orders ties
by room ID, so a correctly published room can remain on a later page for the
entire three-second retry window. PR #684's first Complement attempt failed
this assertion while its other cases passed
([run](https://github.com/palpo-im/palpo/actions/runs/43825493919)).

The patch follows opaque `next_batch` tokens within each publication retry. It
still fails when the room is absent from the directory, and rejects malformed
pages, HTTP errors, repeated tokens and traversals exceeding 100 pages. It keeps
the existing publication timeout and the server's production page size.

CI copies `overlays/public_rooms_pagination_test.go` into the pinned checkout.
Its standalone HTTP fixture places the target after a full ten-room first page
and verifies preservation of an opaque token containing reserved characters.
Additional cases cover missing rooms and invalid/non-progressing pagination.
These tests run before the image build and again in the full suite. The
required-case manifest enforces both the helper tests and the original room
publication assertion.

## registration-uiaa.patch

- Complete an advertised dummy-only UIAA registration flow with the issued
  session in the legacy blueprint runner. Retry once, preserve credentials and
  response storage, and fail on token/email flows or a repeated challenge.
  Palpo's session validation and application-service authentication are unchanged.
- Unit tests cover session propagation, retry failure, missing sessions,
  non-dummy flows, malformed responses and unrelated endpoints.

The full suite uses an exact, expiring failure/skip baseline instead of a global
percentage threshold; see [CI gates](../../../docs/development/ci-gates.md).
The required-case manifests in
`tests/complement/required_pass` already require avatar propagation, bridge joins
and jump-to-date to run and pass. Reuse that gate rather than running those cases
a second time.

After merging main on 2026-11-01, the original avatar fixture is retained: Palpo
now preserves opaque avatar values, and changing the fixture to an MXC URI would
stop exercising that compatibility behavior. Main also supports the legacy
dummy-only blueprint request. This patch still exercises the explicit UIAA
session handshake when a server advertises it, without weakening registration
policy or changing the original avatar assertions.

## Baseline evidence (2026-10-28)

All ten terminal failures on PR #571 head
`35a67929bfee65d2bd5189190d6064f1ab2a19cd` also fail on its main baseline
`ccc82d0efe530c80fc0b235b7dd56d905a525dc2`:

- [Main run](https://github.com/palpo-im/palpo/actions/runs/40247331000)
- [PR #571 run](https://github.com/palpo-im/palpo/actions/runs/40652227484)

| Test | Observed failure | Assessment / scope |
| --- | --- | --- |
| `TestAvatarUrlUpdate` | PUT of `LemurLover` as an avatar URL returns 400 | Historical failure; fixed on current main without changing the fixture |
| `TestJoinFederatedRoomFromApplicationServiceBridgeUser` | Blueprint user registration returns 401 | Missing UIAA session handshake; corrected here, feature assertions still need CI |
| `TestJumpToDateEndpoint` | Same blueprint registration returns 401 | Not evidence of a timestamp lookup defect; corrected setup here |
| `TestDeviceListsUpdateOverFederation/good_connectivity` | `changed` contains Alice and Bob instead of only Alice | Extra remote-user notification; not proof that Alice's update was lost |
| `TestDeviceListsUpdateOverFederation/interrupted_connectivity` | Same extra Bob notification | Requires separate device-list ordering investigation |
| `TestDeviceListsUpdateOverFederation/stopped_server` | Same extra Bob notification | Requires separate device-list ordering investigation |
| `TestInboundFederationRejectsEventsWithRejectedAuthEvents` | Mock reports unexpected GET `/backfill` | Rejection assertions do not fail; history-recovery interaction needs separate coverage |
| `TestOutboundFederationEventSizeGetMissingEvents` | Mock reports unexpected GET `/backfill` | Oversized-event validation is not the reported failure |
| `TestOutboundFederationSend` | Mock reports unexpected GET `/backfill` | Message-send assertion is not the reported failure |
| `TestOutboundFederationIgnoresMissingEventWithBadJSONForRoomVersion6` | Mock reports two unexpected GET `/state` requests | Missing-state recovery interaction; do not weaken bad-JSON rejection assertions |

Those seven historical failures were addressed separately on main. This patch
does not skip tests, relax exact device-list assertions, or disable mock
unexpected-request checking; the current required-case manifests still enforce
those regressions independently of any suite baseline exception.
