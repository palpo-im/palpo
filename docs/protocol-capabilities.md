# Protocol discovery and capability boundaries

This is an operational reference for discovery behavior, source gates and regression
entry points. A version or feature identifier is not evidence that every endpoint,
room topology, permission combination or peer implementation has been exercised.
The tables inventory the current declarations; unknown coverage is stated explicitly.

## Specification revisions and build policy

`routing/client.rs::SUPPORTED_MATRIX_VERSIONS` advertises `r0.5.0`, `r0.6.0`, and
`v1.1` through `v1.12`. The stable reference for the contracts below is
[Matrix v1.12](https://spec.matrix.org/v1.12/client-server-api/). The list remains
independent of the outgoing `MatrixVersion` type. Selectively implemented newer
capabilities do not add newer versions to this declaration.

No optional server features are enabled by a server default feature list.
Server Cargo features `unstable-msc4532` and `unstable-msc4559` imply the preceding
presence features; `unstable-msc4540` implies MSC4363 and MSC4484. Core event-format
features enabled through dependencies do not by themselves prove server behavior.
MSC references in the inventory identify the proposal, but this repository does
not pin each proposal's draft revision. Resolving that revision and validating
full behavior remain separate work.

## Identity and third-party contracts

| Surface | Revision and policy | HTTP regression and limit |
| --- | --- | --- |
| `/client/versions` | Explicit legacy-through-v1.12 list; optional advertisements follow compile/runtime gates below. | `review_threepid_capabilities_and_rejections_match_routes` checks the HTTP upper bound, six compile gates and disabled delayed-event/email-OTP advertisements. It does not exercise every declared stable version. |
| `/client/{v3,r0,v1}/capabilities` → `m.3pid_changes=false` | [v1.12 capability](https://spec.matrix.org/v1.12/client-server-api/#m3pid_changes-capability). Post-registration contact changes remain disabled. | Same case checks the authenticated JSON response and denial/data preservation on all three supported route prefixes. |
| `GET account/3pid` | Existing locally verified or administrator-managed contacts can be read; this is independent of changing them. | Same case seeds a contact with fixed timestamps and reads it before/after refused writes. It does not exercise a real email provider. |
| `POST account/3pid/{add,bind,unbind,delete}` | Refused with `403 M_THREEPID_DENIED`; no identity-server publication or unbinding is performed. | Same case checks all four refusals and unchanged stored contact data. |
| `POST account/3pid/{email,msisdn}/requestToken` | Public refusal with `403 M_THREEPID_DENIED`, as permitted by [v1.12 contact policy](https://spec.matrix.org/v1.12/client-server-api/#post_matrixclientv3account3pidemailrequesttoken). Previous Palpo `account/{medium}/requestToken` paths remain aliases. | Same case exercises correct paths and aliases without user credentials. The separate registration email flow is not enabled by these routes. |
| `POST federation/v1/3pid/onbind` | No server authentication; explicitly unsupported with `404 M_UNRECOGNIZED`. Federation enablement still gates the route. | Same case checks the refusal; `review_onbind_respects_disabled_federation` checks the global disable gate in a separate process. No invite/signature binding is implemented. |
| Legacy `PUT .../3pid/onbind` | v1.12 Federation text used PUT; official [matrix-spec#2452](https://github.com/matrix-org/matrix-spec/pull/2452), merged 2026-10-29, corrected this to POST. PUT remains a refusal alias. The pre-existing `v2` spelling is also a non-standard refusal alias. | Both methods and both route spellings are checked. Aliases do not advertise additional federation functionality. |
| `PUT federation/v1/exchange_third_party_invite/{roomId}` | Still in the authenticated federation router and unsupported after authentication. | Same case verifies an unsigned request is rejected with `401 M_MISSING_TOKEN`. It does not validate a signed third-party invite or the full exchange flow. |
| Six authenticated `/client/v3/thirdparty/...` GET routes | [v1.12 third-party networks](https://spec.matrix.org/v1.12/client-server-api/#third-party-networks). Only enabled database registrations participate. Empty protocol inventory is `{}`; unmatched location/user/protocol lookups return `404 M_NOT_FOUND`. Required generic lookup parameters are checked. | `review_thirdparty_queries_track_live_appservice_disable` uses PostgreSQL and a real local HTTP AS peer, toggles enabled→disabled→enabled without restart, and checks all six routes. |
| Third-party query forwarding | Protocol-specific fields are forwarded, with client `access_token` removed. A fresh outbound request carries only the AS's configured `hs_token`; client headers/body are not copied. Standard user responses use `userid`; old Palpo `user_id` is accepted on input. | Same AS case checks decoded alias/user/custom fields, exact dedicated token, no client Authorization/body, standard output and the legacy input alias. This is query interoperability, not 3PID binding support. |
| Third-party network room-directory publication | `directory/list/appservice/{network_id}/{room_id}` remains explicitly unsupported. | No publication interoperability established here. Public-room network selection has separate directory tests; it does not create a third-party directory. |

The third-party lookup handlers retain best-effort aggregation across AS peers.
This regression uses one successful peer; partial upstream failures, conflicting
protocol metadata, duplicate results, transport deadlines and many-peer admission
are outside its evidence. Protocol metadata with non-empty instance lists and
provider-specific field semantics also need separate fixtures.

## Authenticated capability inventory

| Capability | Revision / feature / runtime policy | Evidence boundary |
| --- | --- | --- |
| `m.change_password` | v1.12; all builds; disabled when delegated auth is enabled. | This PR does not add password-change or delegated-auth mode tests. |
| `m.room_versions` | v1.12; stable/unstable configured room-version lists and configured default. | No Cartesian product of room versions and event/auth rules is claimed. |
| `m.set_displayname`, `m.set_avatar_url` | v1.12; default enabled values retained for older clients. | Profile HTTP suites cover selected ownership/opaque avatar cases. |
| `m.profile_fields` | [v1.16 capability](https://spec.matrix.org/v1.16/client-server-api/#mprofile_fields-capability), explicitly emitted by older-baseline servers; enabled. | Core wire tests and selected profile HTTP tests; no v1.16-wide compatibility claim. |
| `m.account_moderation` | Existing implementation extension; per-user administrator policy; default values may be omitted. | `oauth_admin_tests` exercises selected admin discovery and authorization cases. |
| `org.continuwuity.msc4540.admin` | MSC4540 draft; compiled only with `unstable-msc4540`; allowed scopes depend on provisioned administrator policy. | `oauth_admin_tests`; see [OAuth administration](oauth-admin-scopes.md). It adds no `/versions` flag. |
| `org.matrix.msc4140.delayed_events` | MSC4140 draft; emitted when `delayed_events.enable`, with configured limits. | Delayed-event suites; this PR only checks disabled `/versions` discovery. |
| `m.forget_forced_upon_leave` | Core default is omitted by serialization. | No new discovery/forgetting contract is asserted here. |

## `/versions` feature inventory

“All builds” means this checkout's server declarations, including its fixed core
dependencies. A compiled advertisement is distinct from a user's permission or an
operator enabling local/outgoing presence. “Advertisement gate only” means the new
HTTP test checks presence/absence of a flag, not the entire proposal's behavior.

| Identifier | Specification reference | Build/runtime advertisement gate | HTTP evidence / limitation |
| --- | --- | --- | --- |
| `uk.half-shot.msc2666.query_mutual_rooms` | [MSC2666](https://github.com/matrix-org/matrix-spec-proposals/pull/2666); implemented revision unpinned | all builds; emitted true independently of runtime policy | Pure advertisement tests exist; no claim-specific HTTP conformance established here. |
| `uk.half-shot.msc2666.query_mutual_rooms.stable` | [MSC2666](https://github.com/matrix-org/matrix-spec-proposals/pull/2666); implemented revision unpinned | all builds; emitted true independently of runtime policy | Pure advertisement tests exist; no claim-specific HTTP conformance established here. |
| `org.matrix.e2e_cross_signing` | legacy discovery identifier; no separate revision pinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.msc2285.stable` | [MSC2285](https://github.com/matrix-org/matrix-spec-proposals/pull/2285); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.msc2836` | [MSC2836](https://github.com/matrix-org/matrix-spec-proposals/pull/2836); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.msc2946` | [MSC2946](https://github.com/matrix-org/matrix-spec-proposals/pull/2946); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.msc3026.busy_presence` | [MSC3026](https://github.com/matrix-org/matrix-spec-proposals/pull/3026); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.msc3827` | [MSC3827](https://github.com/matrix-org/matrix-spec-proposals/pull/3827); implemented revision unpinned | all builds; emitted true independently of runtime policy | `directory::tests::review_public_rooms_filters_networks_and_bidirectional_pages` (selected room-type filters) |
| `org.matrix.msc3952_intentional_mentions` | [MSC3952](https://github.com/matrix-org/matrix-spec-proposals/pull/3952); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.msc3575` | [MSC3575](https://github.com/matrix-org/matrix-spec-proposals/pull/3575); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.msc3916.stable` | [MSC3916](https://github.com/matrix-org/matrix-spec-proposals/pull/3916); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.msc4180` | [MSC4180](https://github.com/matrix-org/matrix-spec-proposals/pull/4180); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `uk.tcpip.msc4133` | [MSC4133](https://github.com/matrix-org/matrix-spec-proposals/pull/4133); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `uk.tcpip.msc4133.stable` | [MSC4133](https://github.com/matrix-org/matrix-spec-proposals/pull/4133); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `us.cloke.msc4175` | [MSC4175](https://github.com/matrix-org/matrix-spec-proposals/pull/4175); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.simplified_msc3575` | [MSC4186](https://github.com/matrix-org/matrix-spec-proposals/pull/4186) (legacy discovery name); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `uk.timedout.msc4323` | [MSC4323](https://github.com/matrix-org/matrix-spec-proposals/pull/4323); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.msc4354` | [MSC4354](https://github.com/matrix-org/matrix-spec-proposals/pull/4354); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `org.matrix.msc4380.stable` | [MSC4380](https://github.com/matrix-org/matrix-spec-proposals/pull/4380); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `net.zemos.msc4383` | [MSC4383](https://github.com/matrix-org/matrix-spec-proposals/pull/4383); implemented revision unpinned | all builds; emitted true independently of runtime policy | No claim-specific HTTP evidence established by this document; pure advertisement/router tests are not behavioral conformance. |
| `uk.timedout.msc4494` | [MSC4494](https://github.com/matrix-org/matrix-spec-proposals/pull/4494); implemented revision unpinned | `unstable-msc4494` | new contract: advertisement gate only; `invite-blocking.md` describes narrower evidence |
| `org.matrix.msc4140` | [MSC4140](https://github.com/matrix-org/matrix-spec-proposals/pull/4140); implemented revision unpinned | all builds; `delayed_events.enable=true` | `delayed_event` / delayed-publication HTTP suites; new contract checks disabled advertisement only |
| `org.continuwuity.msc4484.unstable` | [MSC4484](https://github.com/matrix-org/matrix-spec-proposals/pull/4484); implemented revision unpinned | `unstable-msc4484` | new contract: advertisement gate only; `oauth_admin_tests` covers selected scope routes |
| `org.continuwuity.presence_v2.msc4495` | [MSC4495](https://github.com/matrix-org/matrix-spec-proposals/pull/4495); implemented revision unpinned | `unstable-msc4495` | new contract: advertisement gate only; presence suites cover selected recipient transitions |
| `org.continuwuity.presence_v2.msc4532` | [MSC4532](https://github.com/matrix-org/matrix-spec-proposals/pull/4532); implemented revision unpinned | `unstable-msc4532` (implies MSC4495) | new contract: advertisement gate only; `revised-presence.md` describes narrower evidence |
| `org.continuwuity.presence_v2.msc4559` | [MSC4559](https://github.com/matrix-org/matrix-spec-proposals/pull/4559); implemented revision unpinned | `unstable-msc4559` (implies MSC4532/4495) | new contract: advertisement gate only; broad interoperability not established here |
| `org.matrix.msc4262` | [MSC4262](https://github.com/matrix-org/matrix-spec-proposals/pull/4262); implemented revision unpinned | `unstable-msc4262` | new contract: advertisement gate only; no end-to-end profile-extension evidence established here |
| `org.matrix.msc1763` | [MSC1763](https://github.com/matrix-org/matrix-spec-proposals/pull/1763); implemented revision unpinned | `unstable-msc1763`; `retention.enable=true` | `retention_tests::retention_http_disabled_is_not_advertised` and retention HTTP suites |
| `org.palpo.registration.email_otp` | Palpo extension | all builds; value follows `registration_email` presence | new contract: disabled advertisement only; registration policy HTTP suites are separate |



## Reproducing the focused contracts

The quality-control isolated-case manifest runs these three exact test names with
both default features and `--all-features`, each in a separate process and fresh
empty PostgreSQL database:

- `routing::capability_contract_tests::review_threepid_capabilities_and_rejections_match_routes`
- `routing::capability_contract_tests::review_thirdparty_queries_track_live_appservice_disable`
- `routing::federation::tests::review_onbind_respects_disabled_federation`

Set `PALPO_TEST_DATABASE_URL` to a dedicated empty database and invoke the selected
case with `cargo test --locked -p palpo --lib <exact-name> -- --ignored --exact`.
Run again with `--all-features` against another empty database. The onbind-disabled
case owns configuration even though it does not need database queries.

The regression entry points are executable evidence boundaries, not a claim that a
particular checkout has already passed them. Consult the matching-head CI run for
execution results. Remaining validation includes independent MSC revision pinning,
feature combinations beyond default/all, client/peer interoperability, and Spaces
or membership topology × permission × room-version combinations. Historical test
counts or passing advertisement tests must not be used as a substitute for that work.
