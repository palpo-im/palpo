# Legacy Node migration fixtures

This is the unchanged Node entry point and its module dependencies from Palpo
main at `3d23916bc002db6a25cec8f326dcd184a7fed5a1`. It preserves the real SQLite
writer and startup behavior for `../../node_migration.py` and
`../../legacy_adoption.py`, including the database lock and Rust-ownership fence.
The migration regression checks that the old entry point refuses a database
owned by Rust and releases its process lock.

These are frozen test fixtures. Frontend assets, packaging and deployment files
are deliberately absent. Node is needed only for migration regressions; the
Rust Operations service has no Node runtime dependency. Keep the historical
schema, imports and writer-ownership checks intact.
