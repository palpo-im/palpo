# Audited Complement fixture compatibility

These patches apply only to the pinned Complement revision in
`.github/workflows/complement.yml`. CI checks patch applicability before applying
them, then runs the runner unit tests. Do not silently rebase patches when
updating Complement: review the upstream behavior first.

## registration-uiaa.patch

- Complete an advertised dummy-only UIAA registration flow with the issued
  session in the legacy blueprint runner. Retry once, preserve credentials and
  response storage, and fail on token/email flows or a repeated challenge.
  Palpo's session validation and application-service authentication are unchanged.
- Unit tests cover session propagation, retry failure, missing sessions,
  non-dummy flows, malformed responses and unrelated endpoints.

The full suite's existing threshold is unchanged. The required-case manifests in
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
those regressions independently of the suite's percentage allowance.
