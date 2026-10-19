# Runtime v0 design

The daemon is the only process supervisor and mutation authority for an Autono
home. Its OS advisory file lock is acquired before migrations and restart
reconciliation. A second daemon using the same home fails, even if it would use
a different TCP port. The lock file is not unlinked on shutdown. The CLI does
not open the database for normal runtime operations; `doctor` may inspect it
read-only without creating or migrating it.

## Run lifecycle

`PENDING` can transition to `RUNNING`, `FAILED` (spawn failure), `CANCELLED`, or
`INTERRUPTED`. `RUNNING` can transition to `SUCCEEDED`, `FAILED`, `CANCELLED`, or
`INTERRUPTED`. Terminal states have no outgoing transitions. Only an observed
zero process exit is success. Timeout is failure; daemon shutdown/restart is
interruption. Cancellation acknowledgement means the request was accepted, not
that all cleanup has already finished. Cancellation racing observed completion
may return a terminal-state conflict or lose to already committed completion.

`Store` controls mutations. Each state transition validates the current durable
state and commits its run document and event in one transaction. A single SQLx
connection serializes writes; a separate bounded read pool serves status and
event reads. SQLite WAL permits read-only diagnostics during write contention.
Routine host status uses a cheap bounded probe; `quick_check` is confined to
startup and explicit diagnostics. A database CHECK restricts statuses, and triggers prohibit
event updates/deletes. New persistent changes require migrations, embedded at
compile time; no external SQLx database or build-time schema server is needed.

Submission first registers a bounded, tracked daemon-owned admission task, then
waits for its reply. Request disconnect or cancellation drops only the waiter.
Admission holds its own pending permit through validation/commit/registration.
A serial admission gate prevents duplicate claim races; shutdown never waits on
that gate to signal active processes. A committed `PENDING` run is registered and
supervised by that task even if nobody remains to receive its response. A daemon
crash in the commit-to-launch gap still reconciles honestly as interruption.

Each key is persisted in an indexed unique column and the run document. Typed
payload equality includes command, cwd, explicit environment, timeout, and power
eligibility. Matching retries return the original record; conflicts are HTTP 409.
An existing key is looked up before reserving active capacity, but all submissions
remain subject to the pending bound. Neither key reuse nor crash recovery promises
exactly-once external effects. Pre-key records migrate with a null key.

The process controller only handles OS ownership, launch-based deadlines, stop
signals, and output-reader lifecycle. It never waits for SQLite or for output
channel capacity. Pipe readers feed 8192-byte chunks into a 32-event channel and a
separate persistence consumer; disk stalls apply bounded backpressure to the child.
Known transient lock errors retry for up to four seconds per operation. Writer
failure signals the controller while the writer continues consuming/discarding
output to permit cleanup; missing output is explicitly reported.

Unix completion uses `waitid(WNOWAIT)` with SIGCHLD wakeups. The unreaped leader
pins its PID and dedicated group ID. Group signalling happens before the only
consuming wait. `ECHILD` revokes ownership; no group is signalled after reaping.
Observed status/code are captured before cleanup. Errors such as EPERM remain
visible warnings, never a replacement for an observed success/failure. Direct
termination is attempted when group cleanup fails. Reaping attempts are bounded
to three seconds and pipe draining to two; OS refusal/uninterruptible processes
and deliberately escaped descendants remain explicitly uncertain. The registry owns the observed terminal result and its permit separately from
the executing task. It switches to `FINALIZING` when an outcome is observed,
retains start/finish metadata, cleanup warnings, and explicit output-completion
state, and keeps retrying terminal persistence until successful or unsuccessful
shutdown transfers responsibility to an observable `RECOVERY_REQUIRED` record.
The output writer has a finite drain budget; its failure can mark output
incomplete, but cannot discard or overwrite the observed exit result.

Terminal retries use 100 ms exponential backoff capped at two seconds. Each
attempt is an atomic SQLite transaction; matching already-committed results are
no-ops, while conflicting terminal results are rejected. Ordinary lifecycle
transition methods still reject outgoing terminal transitions. The finalizer is
the only normal ownership/permit release point, and releases after a confirmed
commit. Persistent errors are exposed in live status; fresh admission is disabled
while persistence is degraded. Existing keys remain idempotent. The configured
active-run bound also bounds accumulated finalization records.

SIGKILL is immediate. There is no graceful child termination period in v0.
`Run` is a **process attempt**, not an engineering task. No dependency graph,
agent session, workspace, verification contract, retry policy, review finding, or
`BLOCKED_HUMAN` state belongs in this supervisor. The
[engineering layer](engineering.md) builds on these primitives with separate
durable domain checkpoints and leaves these process contracts intact.

## API v1

Read `daemon.json` to find the actual port and token; its PID is diagnostic only.
Send `Authorization: Bearer <token>` and the exact `Host: 127.0.0.1:<port>`.
All routes are authenticated, including status. JSON requests are limited to
1 MiB. Header/body reads have a bounded deadline. The daemon is a trusted local execution service, not a public HTTP server.

Routes:

- `GET /v1/status`: API version, daemon PID/platform, bounded database availability, admission/phase counts, retained persistence
  ownership/errors, and power status.
- `POST /v1/runs`: a `Submission`, returning HTTP 202 and the committed/existing run.
  HTTP 429 with `code: "overloaded"` means admission is full; 409 with
  `code: "submission_conflict"` means the key has another payload.
- `GET /v1/runs?limit=100&offset=0`: runs, newest first, capped at 500 per page.
- `GET /v1/runs/{uuid}`: committed run fields plus live `runtime` ownership.
  `runtime.phase` distinguishes STARTING/EXECUTING/FINALIZING/RECOVERY_REQUIRED;
  `runtime.observed_result` retains the terminal target, exit code, warnings,
  observed finish time, and output-completion flag.
- `POST /v1/runs/{uuid}/cancel`: HTTP 200 with `{"accepted":true}`; inspect or
  follow events for completion. Missing IDs return 404, terminal/unowned runs 409.
- `GET /v1/runs/{uuid}/events?after=0&limit=500`: ordered durable events.
- `GET /v1/runs/{uuid}/events/stream?after=0`: SSE replay followed by live events.

Example submission body:

```json
{
  "submission_key": "83c66946-ab5f-49fa-9234-66d12f62d285",
  "spec": {
    "command": ["sh", "-c", "echo hello"],
    "cwd": "/absolute/project",
    "env": {"MODE": "test"},
    "timeout_seconds": 60,
    "keep_awake": true
  }
}
```

Events have a global monotonically increasing `sequence`, `run_id`, UTC
`timestamp`, and tagged `type`: `created`, `state_changed`, `output`, or `warning`.
`state_changed` includes `from`, `to`, and optional `detail`. `output` includes
`stream` (`stdout`/`stderr`) and `data` (base64 bytes). Chunks can split lines or
UTF-8 characters. Binary output is preserved. Timestamps are informational;
sequence numbers are the ordering/cursor authority.

SSE uses `event: run_event`, the serialized event as `data`, and the sequence as
`id`. Reconnect with `Last-Event-ID` or `after`; the larger cursor wins. Sequences
can have gaps for any one run. Streams replay all events after the cursor and
close once the terminal event has been drained. An in-memory notification wakes
readers, but SQLite is always the replay source. Slow/disconnected consumers
cannot block process supervision. No event retention/deletion is implemented.
Stream errors require reconnection from the last consumed durable cursor.

## Host power ownership

Eligible tasks acquire a shared inhibition reference before spawning. The first
starts one backend lease; the last releases it. The daemon configuration can
disable the backend globally. Acquisition failure leaves execution functional
and records a warning. A supervised macOS helper holds an idle-system assertion
only; its `-w` parent monitor releases it after abrupt daemon death. Host status
reflects helper health, not proof that the machine cannot sleep. Other platforms
return unsupported through the same backend interface.


The HTTP server owns at most 128 connection tasks. Submission body/handler work
and SSE each have an independent 32-slot HTTP budget, leaving capacity for
status/cancel during operational saturation. Idle/incomplete headers are bounded
by the header deadline; there is no claim of protection from a malicious local
user exhausting OS resources. On stop it stops accepting,
signals process control without a database-dependent lock, completes bounded
process/output cleanup, and waits for terminal commits until a ten-second deadline
starting at shutdown. If any ownership remains, shutdown returns an error, marks
those records `RECOVERY_REQUIRED`, stops scheduling further retries, and keeps the
records/permits observable until daemon exit. Already-issued SQLite operations
may still complete; cancelling an async operation is never assumed to roll it
back. HTTP drain/forced teardown and metadata cleanup run on failure too, and the
daemon exits nonzero. A hard crash or failed exit before terminal commit can lose
the in-memory result; startup reconciliation must not invent it.
After process/finalization shutdown returns, including on failure, the configured
HTTP drain deadline begins. At its end, remaining connection tasks are aborted
and joined; sockets cannot keep metadata or the ownership lock alive. The token and lock cleanup
runs after the server returns. Authentication/body parsing never owns lifecycle
work after admission has begun.

Helper health is checked automatically every 100 ms while eligible references
exist. Loss and recovery are sent to active run writers; a full output queue
retains the warning for terminal persistence. Reacquisition backs off to at most
two seconds, with each acquisition capped at one second. Unsupported platforms
do not repeatedly retry an unsupported backend. The final reference cancels the
monitor and releases the helper. `helper_pid` is diagnostic metadata, never a
capability for adopting processes after daemon restart.


Paged-event responses currently lack a budget that spans transport flushing.
128 unread paged responses can temporarily occupy every HTTP connection despite
the POST/SSE caps. Fixing this safely needs transport-lifetime response admission
or write deadlines; it is deferred technical debt. Process signalling, retained
finalization ownership, and bounded shutdown do not depend on freeing those HTTP
connections.
