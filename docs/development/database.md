# Database connection lifetimes

Ordinary buffered queries use `palpo_data::connect()` and Horme's execution traits.
`db()` is an equivalent shorthand. Both functions are synchronous and return a
pool handle, never a checked-out connection:

```rust,ignore
use diesel::prelude::*;
use horme::{OptionalExtension as _, RunQueryDsl};

let mut db = palpo_data::connect();
let user = users::table.find(user_id).first::<DbUser>(&mut db).await.optional()?;
```

The handle contains a cloned pool handle, not a checked-out connection. Each
query acquires one connection when polled and releases it before returning its
buffered result. Keeping a handle or an `if let` condition temporary alive across
business calls therefore does not reserve a connection. Clone the handle for
concurrent tasks.

Import Horme's `RunQueryDsl` for these queries. Its `OptionalExtension` converts
only a missing row to `None`; pool timeouts and other database failures remain
errors. `DataError` and `AppError` preserve Diesel query error classification,
including existing not-found responses.

Use `db.transaction::<_, DataError, _>(async |conn| { ... })` for atomic work.
Pass the callback's `conn` to every helper participating in that transaction.
Ordinary query helpers accept a Horme
`GetConnection<Connection = AsyncPgConnection>` provider so they can reuse either
the lightweight handle or the transaction callback's connection. Helpers that
specifically acquire locks take the callback's `&mut AsyncPgConnection`; passing
a lazy handle to those helpers would lose the lock between queries. Reaching
for the global pool inside a transaction starts a separate checkout and can
still exhaust a small pool.

`db.build_transaction().read_only().repeatable_read().run(...)` preserves
PostgreSQL transaction settings without checking out a connection before `run`
is polled. The transaction returns its lease before follow-up business work,
even if the handle or builder remains alive. Cancellation closes an unfinished
transaction's backend immediately, including cancellation during BEGIN; locks
must not remain in an idle pooled object until the next checkout.

`coordination_connect()` also returns a lightweight handle. Use its scoped
transactions for locks that fence work on the ordinary query pool. The two pools
still share the configured total connection budget, and wait metrics record
actual acquisition in the appropriate pool.

The explicit `checkout().await` escape hatch is reserved for physical-session
instrumentation and database regression tests (backend termination, deliberate
pool exhaustion, and manually blocked transactions). Business code uses query
handles and scoped transactions. Streaming may acquire a deliberately longer
lease through Horme's `GetConnection::get_mut_conn`, with a bounded scope.

The provider records pool wait metrics at actual acquisition. Pool size,
statement timeout, acquisition timeouts, and deadpool recycling retain their
existing configuration. Cancelling a query drops its lease; driver cleanup may
still finish the SQL before that connection is reused.

Horme 0.4 requires Rust 1.96 or newer. Run lifecycle regressions with a dedicated
PostgreSQL database (these tests require no Palpo schema):

```sh
cargo test -p palpo-data pool::tests::
PALPO_TEST_DATABASE_URL=postgres://localhost/palpo_test \
  cargo test -p palpo-data pool::tests:: -- --ignored
```
