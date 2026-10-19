# Autono

Autono is a local-first autonomous engineering harness, with a durable Rust
daemon supervising processes and autonomous engineering workflows. The runtime
is the source of truth; coding agents are replaceable execution providers.

It can plan a Markdown feature, wait for developer approval, run coding agents
in isolated Git worktrees, verify changes, request independent review, apply
bounded fixes, and integrate verified commits into a local branch. Existing
Runtime v0 command supervision, durable logs, cancellation, timeouts, restart
reconciliation, and macOS idle-sleep inhibition remain available. Agent workflows
use installed Codex/Claude CLI login; raw process execution requires neither.
No cloud service or desktop UI is required.

See [the engineering workflow](docs/engineering.md) for commands, configuration,
contracts, safety boundaries, and the opt-in real-agent acceptance scenario.

## Build and run

Use Rust 1.89 or newer and a native C toolchain (SQLx builds bundled SQLite).
macOS and Linux are tested in CI. Windows currently has direct-child termination
and no sleep inhibition; complete process-tree supervision is Unix-only.

```sh
cargo build --workspace --locked
cargo install --path crates/autono-cli --locked
autono doctor
autono daemon
```

The daemon runs in the foreground. In **another terminal**, submit a command:

```sh
autono run -- sh -c 'for i in 1 2 3 4 5; do echo "step $i"; sleep 5; done'
# Prints a run UUID and exits immediately.

autono status
autono logs <run-id>
autono logs --follow <run-id>
autono status <run-id>
autono cancel <run-id>
```

Closing the **submitting** terminal does not stop the run. The daemon owns the
process. Keep the daemon running; if its own terminal must close, launch it with
your service manager or, on Unix, `nohup autono daemon >autono-daemon.log 2>&1 &`.
There is no automatic daemon startup or service installation in v0.

Additional options:

```sh
autono run --cwd /absolute/project --env MODE=test --timeout 60 -- make test
autono run --no-keep-awake -- sh -c 'echo hello'
autono logs --events <run-id>       # JSON Lines with lifecycle/output events
autono logs --follow --after 123 <run-id>
autono run --submission-key <uuid> -- sh -c 'echo retryable-submission'
autono status --json --limit 50 --offset 0
autono --help
```

Commands are argument arrays, executed directly, with no implicit shell. Use
`sh -c` explicitly for shell syntax. Stdin is closed (`null`); interactive commands
are unsupported. Environment overrides are applied to the daemon's inherited
environment, not the submitting terminal's environment. `run` succeeds when the
submission is accepted. A UUID submission key is printed to stderr before sending;
reuse it with `--submission-key` after an ambiguous response. Command failures are shown by `status` and lifecycle
events. `logs` replays stdout/stderr as raw bytes, including terminal escapes;
use `--events` for structured inspection of untrusted output.

`doctor` reports the platform, optional git/Codex/Claude/Docker/gh tooling,
repository status, power capability, daemon connectivity, and SQLite health.
Missing optional tools do not prevent execution. On a fresh installation it
reports an uninitialized database without creating one. Diagnostic findings are
printed for humans; they are not a machine-readable readiness gate.

## Data and configuration

Defaults use the OS user data directory: `~/Library/Application Support/autono`
on macOS, `$XDG_DATA_HOME/autono` (usually `~/.local/share/autono`) on Linux, and
the local application-data directory on Windows. `autono doctor` prints the
resolved path. Set an absolute `AUTONO_HOME`, or pass the global `--home` option,
to isolate an installation. All clients and the daemon must use the same home.

The directory contains:

- `runtime.sqlite3` and SQLite WAL files: runs, metadata, and append-only events.
- `daemon.lock`: OS-held exclusive daemon ownership; its presence alone does not
  mean a daemon is alive. Do not delete it while a daemon is running.
- `daemon.json`: current loopback address, PID, API version, and secret token.
  Published atomically, rotated at startup, removed on graceful shutdown.
- `config.toml`: optional user configuration, read at daemon/CLI startup.
- `engineering/<run>/`: retained planner/task/integration worktrees, OS-held
  workspace locks, structured-output schemas, and review diff artifacts.

Example `config.toml`:

```toml
[runner]
keep_awake = true
max_active_runs = 8
max_pending_submissions = 32

[daemon]
port = 0 # Choose an available port; alternatively set a fixed localhost port.
body_timeout_seconds = 5
drain_timeout_seconds = 2
```

An optional `autono.toml` in the submitted working directory can set
`[runner].keep_awake`. Parent directories are not searched. Project configuration
can also override
[agent, engineering execution, and verification settings](docs/engineering.md#configuration).
It cannot change daemon settings or process admission limits. The daemon's
`keep_awake = false` is an overall
off switch; project configuration or API requests cannot override it. Restart the
daemon after changing its configuration.

On macOS a shared `caffeinate -i -w <daemon-pid>` helper inhibits **idle system
sleep**, allowing the display to sleep. It is held while eligible runs are
active and released after the last one. Acquisition failures become run warning
events. Helper loss is detected automatically, persisted as a warning on eligible
runs, and retried with backoff from 100 ms up to 2 seconds (each acquisition has
a 1-second bound). Multiple runs still share a single lease.
Power inhibition does not guarantee execution through lid closure, forced sleep,
logout, shutdown, or power loss. Linux/Windows report unsupported, and runs still
work. See [SECURITY.md](SECURITY.md) before exposing local data or executing
untrusted commands.

## Recovery and intentional v0 limits

State changes and lifecycle events commit in one SQLite transaction. The database
uses migrations, WAL, foreign keys, and `synchronous=FULL`. Output is persisted in
bounded chunks before being exposed to API clients. Stdout/stderr ordering is the
order observed by the supervisor, not a guarantee of the child's write order.

At startup, after acquiring exclusive ownership, **all persisted `PENDING` and
`RUNNING` runs become `INTERRUPTED`** with an explanation and event. Saved PIDs
are diagnostic metadata only. The daemon never adopts or signals a saved PID.
A hard-killed daemon may leave children executing and buffered output may be
lost. Restart does not claim to resume, cancel, or clean up those children.
Inspect them manually before rerunning work that has side effects.

Graceful Ctrl-C/SIGTERM stops owned commands and marks unfinished work
`INTERRUPTED`. Cancellation and timeout use immediate process-group `SIGKILL` on
Unix and reap the direct child; timeout is `FAILED` with a distinct explanation.
On Unix the leader is observed with `waitid(WNOWAIT)` and remains unreaped while
its process group is signalled; the group capability is revoked before reaping.
Cleanup errors (including EPERM) are warning events and `cleanup_warnings` on the
run, and cannot replace an observed exit result. Remaining group members are
also stopped when their leader exits. Processes that deliberately detach into another group/session can escape supervision. On
non-Unix systems only the direct child is terminated. Output drain is bounded to
two seconds after process termination; incomplete output produces a warning.

Admission has separate active-run and pending-submission limits. Active capacity
includes reserved launch slots and finalizing attempts; pending capacity bounds
daemon-owned admission operations, not a durable workflow queue. Overload returns
HTTP 429 with `code: "overloaded"`, without consuming cancel/status capacity.
A retry can also receive 429 while admission is saturated; retry later with the
same key. There is no log retention/rotation, security sandbox,
checkpoint restart, resource quota, service installer, or schema downgrade support.
Logs grow on disk until you manage the installation's data. Keep the database on
a local filesystem. Back up with SQLite's backup facilities or stop the daemon
before copying the complete database; copying only the main file while WAL is
active can omit committed work.

Process control runs independently of the bounded output writer. Timeout starts
at launch; cancellation and shutdown can stop children even while SQLite is locked.
Output-write lock errors have bounded retries and output draining remains bounded.
Once a process result is observed, the daemon retains it and its capacity permit
in a separate **FINALIZING** phase until an atomic terminal commit succeeds.
Terminal persistence retries use backoff from 100 ms up to two seconds, without a
running-daemon expiry. Temporary locks therefore do not require restart. Cleanup
warnings, observed exit/code/time, and output-completion state remain available.
If output could not be fully persisted, `output_complete` is false; missing bytes
are not silently described as complete.

`status` lists live ownership phases; JSON also keeps the last committed SQLite
status separate under the run fields. Inspect `runtime.phase` and
`runtime.observed_result` when persistence is behind execution. `/v1/status`
reports `persistence.healthy`, owned results/errors, and separate executing and
finalizing counts. `active_runs` counts occupied capacity, including finalization.
A degraded runtime rejects fresh executions with HTTP 503
`code: "persistence_degraded"`; matching submission-key retries still return the
existing run. Finalization retains the original observed outcome even if cancel
or shutdown is requested after the process exited. Once all results commit,
ownership/permits are released and normal admission recovers automatically.

Graceful shutdown signals/stops processes first and waits for finalization within
a ten-second deadline starting at the shutdown request (process cleanup always
finishes its existing bounded attempts before HTTP teardown). If results remain
unpersisted, shutdown marks them **RECOVERY_REQUIRED**, retains their records and
permits until confirmed commit or daemon exit, reports the affected IDs, and exits nonzero
after bounded HTTP cleanup. It never reports successful graceful shutdown in
that condition. The records are in memory, not a second durable journal: a hard
crash or unsuccessful exit before terminal commit can lose an observed exit code.
Subsequent startup can only reconcile the nonterminal record as `INTERRUPTED`.

Known HTTP debt: a burst of 128 unread paged-event responses can temporarily
exhaust the connection limit and delay control requests. The independent POST
and SSE budgets do not cover transport backpressure from paged responses. A
transport-lifetime response budget/write deadline is deferred; raising the
connection limit is not a fix. This does not block process-control signals or
bounded daemon shutdown.

## Architecture and local API

- `autono-core`: domain/state transitions, SQLite migrations, event persistence,
  process supervision, recovery, configuration, and host power inhibition.
- `autono-protocol`: lightweight serialized domain/request/response/event contract;
  it does not depend on the runtime, Tokio, or SQLx.
- `autono-daemon`: exclusive runtime owner and authenticated Axum API.
- `autono-cli`: submission, status, logs, cancellation, and diagnostics. The
  `autono daemon` command invokes the same daemon library as `autono-daemon`.

The API binds only to `127.0.0.1`, requires a bearer token, rejects browser
Origins and foreign Host headers, and enables no CORS. It supports run listing,
inspection, submission, cancellation, paged events, resumable SSE event streams,
and host/database/power status. A future UI should use this API with the daemon
as an independent process. Complete request bodies must arrive within the configured
deadline. On shutdown, owned processes are stopped before a bounded HTTP drain;
remaining connection tasks (including slow SSE receivers) are then aborted.

Submission keys have a SQLite unique constraint. The same key and payload return
the existing run, including after daemon restart; a conflicting payload returns
HTTP 409. This is **idempotent submission, not exactly-once external effects**.
Do not allocate a new key just because a response was lost. Retrying an interrupted
run with the same key returns that interrupted record without executing it again.
Log following retains its sequence cursor, attempts at most five reconnects with
bounded backoff, and reloads daemon address/token on each retry. If that budget is
exhausted, use the printed `--after` cursor to resume.

See [the runtime design](docs/runtime.md) for routes,
event cursors, and ownership invariants.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
cargo run -p autono-cli -- doctor
cargo doc --workspace --no-deps
```

Tests exercise state transitions, durable events, process success/failure,
cancellation, timeouts, environment/cwd/stdin, restart reconciliation,
stay-awake reference counting, API authentication and replay, and a separate-CLI
submission followed by daemon SIGKILL/restart. The regression suite also exercises
SQLite locks, TCP resets, idempotency, admission saturation, stalled HTTP clients,
log reconnect, repeated rapid failures, and live macOS helper replacement. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Roadmap

The first local planner/coder/verifier/reviewer loop is implemented above Runtime
v0. Future milestones may add intelligent preflight, permission brokerage,
autonomous replanning, and a desktop interface. They are not part of this version.

MIT licensed; see [LICENSE](LICENSE).

Review repair details and validation evidence: [runtime review repairs](docs/review-repairs.md).
