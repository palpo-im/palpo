# Hagency coordinator workflow contract

First Rust implementation slice of [Rinx ADR 0011](https://github.com/hagency-org/Rinx/blob/main/docs/adr/0011-hagency-server-engagements.md).

The Matrix administrator authorizes server associations. The resource owner's
assigned Hagency coordinator approves projects and agents. An authorized owner
may also approve agent allocations. One Rinx mini-app approval is sufficient;
Hagency automatically checks authority, reserves capacity and provisions it.

This crate implements versioned command data, distinct server-engagement and
agent-allocation IDs, current-state policy checks and checked budget arithmetic.
It does **not** add live endpoints or replace the Node workflow backend yet.
Do not advertise `hagency.coordinator_approval.v1` merely because this crate builds.

## Trust and transaction boundaries

- Authenticate Matrix users in the server adapter. At Hagency, authenticate the
  machine envelope and verify the actor proof; never trust a body-supplied actor.
- Load engagement/coordinator, request and project state from trusted storage.
  Deserializing these types does not authenticate their source or delegation.
- Call the policy function in the transaction that checks pending state/revision,
  records the verdict and inserts the outbox command. Its `Ok` alone is not a
  persisted approval. Validate that requested resources are funded, visible and
  belong to the engagement; duplicate resource IDs are refused by the contract.
- Canonicalize the full immutable agent definition, including resource settings,
  target rooms and limits, identically on both sides before enabling the protocol.
- For execution, recheck registration/delegation/project/request revisions and
  expiry, then reserve the engagement's remaining capacity and record the command
  receipt atomically. Serialize competing approvals in the database, not here.
- A replay with the same command ID and identical content returns the stored
  receipt; a changed payload conflicts. Fetching a prior result never executes
  revoked work again. Crash-safe outbox and receipt storage are integration work.
- The first approval can grant a positive amount no greater than requested.
  An increase is a separate top-up request against the existing agent allocation.
  `TokenTopUpApproval` freezes the existing agent, expected allocation and
  requested addition. The same coordinator/owner policy applies; adapters must
  compare the actual allocation and remaining parent capacity atomically.

The shared canonical encoder matches JavaScript UTF-16 object ordering,
array-index keys and finite transport numbers. Signed operation definitions
permit only exact JSON integers; transport events additionally allow finite
floating-point values.

`BudgetSnapshot` describes one explicit account/resource/period scope selected by
the trusted writer. Available = allocated - consumed - reserved unused. It does
not release spent tokens or supply synchronization. Missing allocations and
overdrawn observations refuse new capacity. Amounts stay within exact JSON integer
range for OctoScript consumers. An engagement allocation is reserved at its parent;
allocating an agent within it must not charge the same parent reservation twice.

## Remaining integration

1. Add independent engagement profiles, delegation proof and the durable parent/
   child reservation ledger to Hagency; materialize every delivered approved agent
   before provisioning, including failures.
2. Complete the [Rust Operations service](../operations/README.md): persistence,
   sessions and coordinator Inbox decisions are implemented; admin profile export,
   Hagency execution, notification delivery and legacy reconciliation remain.
   Preserve stable IDs and role bindings.
3. Wire these checks into both adapters and all alternative request routes. Add
   transactional concurrency, replay, restart and migration tests.
4. Add Rinx role-aware screens and verified agent/usage projections; run Makepad
   instrumentation and live E2E showing one coordinator decision and no console
   approval. Unknown usage remains unknown; job summaries are a future extension.

Run `cargo test --locked -p palpo-hagency-contract`.
