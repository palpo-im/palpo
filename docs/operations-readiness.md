# Liveness and readiness

Standalone Palpo exposes two different operational checks:

- `GET /health` and `GET /healthz` return HTTP 200 and `ok` while the HTTP
  application is alive. They do not query PostgreSQL or depend on federation.
- `GET /ready` returns HTTP 200 when startup completed, PostgreSQL answers the
  bounded query-pool probe, the outbound scheduler recovered its durable queue,
  and shutdown has not been requested. Otherwise it returns HTTP 503. The JSON
  status is always sent with `Cache-Control: no-store`.

Use the liveness check for process restart decisions and readiness for admitting
traffic. A database outage should remove the instance from service without
causing a restart loop. Destination-specific federation, appservice or push
failures retain their durable retries and do not make the whole server unready.

The database probe has a one-second deadline covering both pool checkout and
`SELECT 1`. At most four probes can run concurrently; additional probes return
`database: "busy"` immediately with HTTP 503. Use a modest polling interval such
as five seconds per instance rather than treating readiness as a public traffic
endpoint. Responses contain no database URLs or raw database errors.

The JSON response includes `startup_complete`, `shutting_down`, the database
probe result, outbound worker `running`, `ready` and `restart_count`, and the
existing query/coordination pool `max_size`, `size`, `available` and `waiting`
counts. Monitor sustained unready responses, worker restart increases and pool
waiting/saturation. Limit public access to operational endpoints at the ingress
when these details should be restricted.

Worker readiness is cleared on task cancellation, panic, fatal startup failure
and failure to scan queued destinations. After the database recovers, the
supervisor restores failed startup work automatically; a failed periodic scan
clears on a subsequent successful scan (normally within the 30-second sweep
interval). This is a scheduler lifecycle check, not a proof that every queued
destination is delivering successfully or a watchdog for every optional task.

Readiness intentionally avoids scanning the entire outgoing queue on every
probe. Use [`GET /diagnostics`](development/outbound-diagnostics.md) for cached
queue depth, oldest known age, retry counts, scheduler heartbeat and bounded
latency histograms. Legacy rows without an enqueue timestamp report unknown age;
they cannot establish an age SLO. Diagnostics counters are process-local and its
queue sampler has a fixed refresh budget. The coordination pool is reported by
readiness but is not checked out by its connectivity probe.

Embedded applications keep control of their route names and restart policy.
`MatrixServer::router()` installs no health or readiness routes. Call
`server.readiness().await` (or `palpo::readiness::check().await`) and use its
`ready` field for the host's own HTTP status. Initialize and probe within the
same Tokio runtime that owns Palpo's workers.
