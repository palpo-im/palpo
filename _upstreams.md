# Upstream Ruma Changes Not Merged

Source comparison:

- Fork baseline: `palpo-im/ruma` `origin/main`
- Upstream baseline: `ruma/ruma` `upstream/main`
- Compared range: `origin/main..upstream/main`
- Reviewed on: 2026-04-24

This list tracks upstream functional changes that were reviewed but not merged into palpo in the
current sync. It does not list CI-only, dependency-only, changelog-only, formatting-only, or
test-only PRs.

Already merged in the current sync: Matrix `v1.18` type support, MSC3417 call room type support,
`m.room.policy` event content, `m.key_backup`, `ImageInfo::is_animated`, secret storage key
`name` serde compatibility, `FeatureFlag::Msc4323`, stable `FeatureFlag::Msc4380`, and
`AllowRule` inspection helpers.

| Area | Upstream change not merged | Upstream PRs | Notes |
| --- | --- | --- | --- |
| Rendezvous / MSC4388 | Update the rendezvous discovery mechanism and add `create_available` to the discovery response. | [ruma#2419](https://github.com/ruma/ruma/pull/2419), [ruma#2422](https://github.com/ruma/ruma/pull/2422) | Needs a client discovery API review against palpo's current endpoint layout. |
| Auth metadata | Add `NoAccessToken` as a distinct authentication scheme. | [ruma#2420](https://github.com/ruma/ruma/pull/2420) | Changes API metadata/auth semantics; should be reviewed with request generation and server routing. |
| OAuth device authorization | Stabilize OAuth 2.0 Device Authorization Grant support. | [ruma#2421](https://github.com/ruma/ruma/pull/2421) | Endpoint/type support was not audited against palpo's current auth modules. |
| Encrypted media metadata | Replace loose `EncryptedFile` fields with stricter version/hash/Base64 array types. | [ruma#2424](https://github.com/ruma/ruma/pull/2424) | Public type-shape change; likely needs a dedicated migration for existing media/event code. |
| Policy server endpoints | Add the client-server public-key information endpoint and federation event-signing endpoint for policy servers. | [ruma#2434](https://github.com/ruma/ruma/pull/2434), [ruma#2435](https://github.com/ruma/ruma/pull/2435) | Only the `m.room.policy` event content was merged; endpoints need server route/storage integration. |
| Account suspension / lock | Add client-server suspend and lock endpoints. | [ruma#2432](https://github.com/ruma/ruma/pull/2432) | Only `FeatureFlag::Msc4323` was merged; endpoint behavior requires product/server decisions. |
| Support discovery / MSC4439 | Add experimental MSC4439 support-file support. | [ruma#2446](https://github.com/ruma/ruma/pull/2446) | New discovery surface; should be considered with support/contact configuration. |
| Client API / MSC4406 | Add unstable support for MSC4406. | [ruma#2458](https://github.com/ruma/ruma/pull/2458) | Not mapped into palpo's client API modules yet. |
| Federation API / MSC4373 | Add unstable support for MSC4373. | [ruma#2461](https://github.com/ruma/ruma/pull/2461) | Needs federation route and compatibility review. |
| Matrix errors | Add `M_USER_LIMIT_EXCEEDED`. | [ruma#2418](https://github.com/ruma/ruma/pull/2418) | Small candidate for a later follow-up, but it was not part of the selected type/event sync. |
| API error model | Merge client API `Error` into common API error types and stop re-exporting old names. | [ruma#2447](https://github.com/ruma/ruma/pull/2447) | Broad public API reorganization; not safe to mix with this protocol-type sync. |
| User identifiers | Make `UserIdentifier` variants more non-exhaustive and support custom identifier types. | [ruma#2394](https://github.com/ruma/ruma/pull/2394) | Public API shape change; needs login/auth flow review. |
| Push rules | Refactor `Action` / `Tweak` serde, restrict custom constructors, and make push condition variants non-exhaustive. | [ruma#2395](https://github.com/ruma/ruma/pull/2395), [ruma#2396](https://github.com/ruma/ruma/pull/2396) | Broad push-rule API/serde compatibility work; should be isolated. |
| Profile fields | Move profile field name/value types to common, add extended profile fields to federation profile queries, and remove `compat-empty-string-null` from federation API. | [ruma#2400](https://github.com/ruma/ruma/pull/2400) | Cross-crate API move; requires checking palpo client/federation profile handling. |
| Custom event content | Use dedicated `Custom*EventContent` types for `_Custom` variants of `Any*EventContent` enums. | [ruma#2411](https://github.com/ruma/ruma/pull/2411) | Event enum API change; could affect downstream custom-event handling. |
| Signatures / verification errors | Refactor signature and canonical JSON error types, ignore signatures from missing keys, and reorganize signature modules. | [ruma#2402](https://github.com/ruma/ruma/pull/2402), [ruma#2415](https://github.com/ruma/ruma/pull/2415), [ruma#2442](https://github.com/ruma/ruma/pull/2442) | Security-sensitive and API-wide; should be migrated and tested separately. |
| Canonical JSON redaction | Move redaction functions/types to a separate module and use recursive redaction code. | [ruma#2445](https://github.com/ruma/ruma/pull/2445) | Affects event redaction behavior; needs dedicated state/event tests. |
| State resolution internals | Reorganize state-res tests and add/use `EventIdMap` / `EventIdSet` in reverse topological sorting. | [ruma#2416](https://github.com/ruma/ruma/pull/2416), [ruma#2441](https://github.com/ruma/ruma/pull/2441), [ruma#2451](https://github.com/ruma/ruma/pull/2451), [ruma#2460](https://github.com/ruma/ruma/pull/2460) | Mostly internal/test/perf structure; should be handled with a state-res focused pass. |
| Federation endpoint removals | Remove `create_(join/leave)_event::v1` endpoint types. | [ruma#2444](https://github.com/ruma/ruma/pull/2444) | Potential compatibility impact; requires checking palpo federation support policy. |
| Room aliases | Remove support for `m.room.aliases`. | [ruma#2449](https://github.com/ruma/ruma/pull/2449) | Behavior removal; should be a conscious compatibility decision. |
| Client API convenience | Add a `new` constructor to `delete_3pid::v3::Response`. | [ruma#2456](https://github.com/ruma/ruma/pull/2456) | Small API convenience not prioritized for this sync. |
