# Local alias mutation contracts

Standard directory creation is create-only. An existing or concurrently claimed
name returns HTTP 409, as specified by the
[Matrix v1.12 directory PUT](https://spec.matrix.org/v1.12/client-server-api/#put_matrixclientv3directoryroomroomalias)
and the [v1.19 endpoint](https://spec.matrix.org/v1.19/client-server-api/#put_matrixclientv3directoryroomroomalias).
The documented example uses `M_UNKNOWN`; it does not require `M_ROOM_IN_USE` for
this endpoint. In contrast, a `createRoom` alias conflict returns
`M_ROOM_IN_USE`, including when a competing insertion wins after the precheck.
Both paths check the actual unique insertion result instead of treating
`ON CONFLICT DO NOTHING` as success.

Directory deletion authorizes a mapping snapshot. After any room-state lock
wait, it rechecks that snapshot and the caller's current namespace/room rights.
The final database delete compares all mapping fields and a database-assigned
generation. Insertions and updates, even same-value updates, receive a new
generation; a delete-and-recreate within one millisecond cannot reuse an old
authorization. A changed mapping returns 409. Administrators deleting a room
also remove only the mappings they observed, without deleting replacements
which have moved to another room.

Canonical/alternative alias cleanup preserves unrelated aliases and custom
content. Directory deletion permission is independent of permission to publish
room state. A mapping to a room not stored locally can therefore still be
removed by its authorized owner; optional state cleanup does not require that
remote room's local version/state.

The [room upgrade procedure](https://spec.matrix.org/v1.12/client-server-api/#post_matrixclientv3roomsroomidupgrade)
explicitly moves local aliases to the replacement. This uses a snapshot CAS,
preserving the alias's creator and creation time. The administrator CLI's
explicit force operation has a separate permission-checked replacement path;
ordinary directory PUT never overwrites an existing mapping. These operations
protect directory rows, not an atomic rollback of all room creation/upgrade
PDUs or external effects. They hold no global database lock over network I/O.

## Application-service namespaces

The authenticated registration identity is retained through C-S alias routes.
The [registration](https://spec.matrix.org/v1.12/application-service-api/#registration)
and [AS permissions](https://spec.matrix.org/v1.12/application-service-api/#server-admin-style-permissions)
contracts apply to directory PUT, directory DELETE and `createRoom` aliases:

- An AS may create aliases only within its own alias namespace; otherwise the
  request fails with `M_EXCLUSIVE`.
- Another registration's exclusive namespace prevents creation/deletion by
  people and other ASes, including ordinary C-S administrator tokens. An
  overlapping nonexclusive pattern does not bypass this restriction.
- The same AS can manage its namespace across virtual users, independently of
  which virtual user originally created the mapping.
- AS deletion outside its own namespace is not categorically prohibited when
  no other AS has an exclusive claim. The asserted user must then satisfy the
  ordinary directory deletion permission checks.

These rules also appear in the
[v1.19 AS specification](https://spec.matrix.org/v1.19/application-service-api/#server-admin-style-permissions).
Disabled registrations conservatively retain exclusive reservations; pausing a
bridge does not transfer its names. Explicit vendor/server administration is
kept separate from an ordinary C-S administrator access token.

The PostgreSQL integration targets `alias_atomic_contracts` and
`alias_generation_upgrade` each require a separate empty database through
`PALPO_TEST_DATABASE_URL`. They cover gated HTTP races, same-timestamp ABA,
namespace intersections, unknown-room mappings, and legacy migration/restart
behavior. CI runs them separately from shared library database cases.
