# Audited Complement fixture compatibility

These patches apply only to the pinned Complement revision in
`.github/workflows/complement.yml`. CI checks patch applicability before applying
them, then runs the runner unit tests. Do not silently rebase patches when
updating Complement: review the upstream behavior first.

## registration-and-avatar.patch

- Use an MXC URI in `TestAvatarUrlUpdate`, keeping the assertion that the exact
  value propagates to room membership. `LemurLover` remains the display-name
  fixture. Palpo's avatar validation is unchanged.
- Complete an advertised dummy-only UIAA registration flow with the issued
  session in the legacy blueprint runner. Retry once, preserve credentials and
  response storage, and fail on token/email flows or a repeated challenge.
  Palpo's session validation and application-service authentication are unchanged.
- Unit tests cover session propagation, retry failure, missing sessions,
  non-dummy flows, malformed responses and unrelated endpoints.

The full suite's existing threshold is unchanged. A separate strict step requires
all three affected tests to actually pass, including the two tests whose blueprint
registration previously failed before their feature assertions could run.

## Baseline evidence (2026-10-28)

All ten terminal failures on PR #571 head
`35a67929bfee65d2bd5189190d6064f1ab2a19cd` also fail on its main baseline
`ccc82d0efe530c80fc0b235b7dd56d905a525dc2`:

- [Main run](https://github.com/palpo-im/palpo/actions/runs/40247331000)
- [PR #571 run](https://github.com/palpo-im/palpo/actions/runs/40652227484)

| Test | Observed failure | Assessment / scope |
| --- | --- | --- |
| `TestAvatarUrlUpdate` | PUT of `LemurLover` as an avatar URL returns 400 | Invalid fixture; corrected here |
| `TestJoinFederatedRoomFromApplicationServiceBridgeUser` | Blueprint user registration returns 401 | Missing UIAA session handshake; corrected here, feature assertions still need CI |
| `TestJumpToDateEndpoint` | Same blueprint registration returns 401 | Not evidence of a timestamp lookup defect; corrected setup here |
| `TestDeviceListsUpdateOverFederation/good_connectivity` | `changed` contains Alice and Bob instead of only Alice | Extra remote-user notification; not proof that Alice's update was lost |
| `TestDeviceListsUpdateOverFederation/interrupted_connectivity` | Same extra Bob notification | Requires separate device-list ordering investigation |
| `TestDeviceListsUpdateOverFederation/stopped_server` | Same extra Bob notification | Requires separate device-list ordering investigation |
| `TestInboundFederationRejectsEventsWithRejectedAuthEvents` | Mock reports unexpected GET `/backfill` | Rejection assertions do not fail; history-recovery interaction needs separate coverage |
| `TestOutboundFederationEventSizeGetMissingEvents` | Mock reports unexpected GET `/backfill` | Oversized-event validation is not the reported failure |
| `TestOutboundFederationSend` | Mock reports unexpected GET `/backfill` | Message-send assertion is not the reported failure |
| `TestOutboundFederationIgnoresMissingEventWithBadJSONForRoomVersion6` | Mock reports two unexpected GET `/state` requests | Missing-state recovery interaction; do not weaken bad-JSON rejection assertions |

The remaining seven failures are deliberately not hidden by this patch. No test
is skipped, no exact device-list assertion is relaxed, and mock unexpected-request
checking remains enabled.
