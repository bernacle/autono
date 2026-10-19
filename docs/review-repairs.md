# Runtime v0 systems-review repairs

The final independent systems review passed with no remaining P0/P1 findings.
The original six P1 findings and subsequent blocking finalization ownership
finding are resolved as described below; the P2 paged-response starvation finding
is explicitly deferred. The earlier P3 protocol split added no crates. This remains a process-attempt runtime; engineering tasks, DAGs,
agents, workspaces, review policies, and automatic workflow resume are excluded.

## Current repair: finalization ownership

The second independent review correctly found that exhausting retries removed
ownership and capacity while a durable run remained RUNNING. Logging an error
and relying on restart reconciliation lost an already-observed outcome. That
behavior was a defect, not a supported recovery guarantee.

- The registry now owns each capacity permit and retained finalization record.
  Executing tasks no longer release either on persistence failure. After process
  exit/reaping, live phase is FINALIZING, separate from SQLite's last committed
  status. Retained data includes the observed target/code/time, cleanup warnings,
  and output-completion state. The bounded output writer is a separate lifecycle.
- A dedicated finalizer retries with backoff from 100 ms to two seconds while the
  daemon operates. It has no retry-budget handoff that discards ownership. An
  idempotent atomic terminal transaction writes the original result and terminal
  event; matching repeated commits are no-ops and conflicting results are errors.
- Status exposes executing/finalizing counts, retained ownership, persistence
  health, errors, and retry counts. Human CLI status shows FINALIZING instead of
  claiming the command is executing. Degraded persistence blocks new executions
  with a structured 503; existing submission keys remain idempotent. Permits bound
  accumulated finalizations and are released only after a confirmed terminal commit.
- Cancellation after an observed exit does not change its result. Shutdown signals
  processes independently, completes their bounded cleanup, and waits for terminal
  commits within its deadline. Unresolved records become RECOVERY_REQUIRED and
  remain owned while the runtime lives. Shutdown returns an error; HTTP/metadata
  cleanup still runs and the daemon process exits nonzero. A hard crash or failed
  exit before terminal commit may lose in-memory results. There is no second
  journal, automatic workflow resume, or exactly-once external-effects claim.

New regression evidence:

- `long_sqlite_lock_keeps_concurrent_results_owned_and_recovers_without_restart`:
  FIFO/SSE/notification barriers launch three commands, hold BEGIN IMMEDIATE
  through eleven failed persistence notifications (roughly 18 seconds, longer
  than the old budgets), and prove three exit-42 results remain owned. Executing
  count is zero, occupied/finalizing capacity is three, and health is degraded.
  Matching retries and cancellation preserve the result; new submissions fail
  explicitly. Unlocking commits FAILED/42 once per run and releases every permit,
  restores health, and permits new execution without restart.
- `cancellation_and_shutdown_preserve_pending_finalization_until_lock_release`:
  cancel and shutdown while finalizing, then unlock and verify successful shutdown
  persisted the original failure rather than cancellation/interruption.
- `persistent_failure_makes_shutdown_fail_and_retains_an_observable_recovery_record`:
  inject a persistent terminal-write failure, expire a short shutdown deadline,
  and verify recovery-required ownership/result/capacity remain visible, with no
  invented terminal event or successful shutdown.
- `status_exposes_finalization_degradation_and_idempotent_retry_keeps_the_owner`:
  verify the actual HTTP health/run views, FINALIZING metadata, 503 admission,
  cancellation, and key retry while locked, then recover through normal shutdown.
- `repeated_matching_terminal_commit_is_a_noop_and_conflicts_are_rejected`:
  repeated terminal commits produce one terminal event and warning; conflicting
  results and ordinary illegal terminal transitions are rejected.
- `persistent_finalization_failure_causes_nonzero_daemon_exit_and_cleans_metadata`:
  a real daemon retains the observed failure, CLI/doctor expose degradation, and
  SIGTERM exits nonzero within the deadline, removes metadata, and releases the
  ownership lock. Restart after removing the injected fault honestly records
  INTERRUPTED with no invented exit code.

## Deferred P2: paged-response HTTP control starvation

A burst of 128 unread paged-event responses can temporarily exhaust the global
connection pool. The existing POST/SSE budgets do not solve response write
backpressure for paged results. A reliable fix must hold response capacity through
transport flushing or add an appropriate write deadline; merely wrapping body
production can release capacity before buffered bytes are sent. This requires a
separate HTTP transport-lifecycle change and saturation tests, so it is deferred
rather than coupled to the blocking finalization repair. It does not block the
runtime's in-memory process signals or bounded shutdown.

## Earlier changes and regression evidence


- **Unowned accepted submissions:** a bounded tracked daemon admission task owns
  commit, registration, and launch. The HTTP future owns only a oneshot waiter.
  Shutdown closes a short synchronous gate and cannot cancel an accepted task
  between commit and registration. Tests abort a submission waiter while SQLite
  is locked, and reset a real TCP submission before releasing that lock. They
  verify the original request is durable and supervised before sending a retry.
- **Process groups and outcomes:** Unix `waitid(WNOWAIT)` observes the result
  without consuming the leader. The leader pins PID/PGID through group cleanup;
  group authority is revoked before the consuming wait. `ECHILD` revokes ownership.
  EPERM is reported, not globally suppressed. Cleanup errors become warnings and
  cannot replace observed status/code. Tests repeat rapid exit 42 eighty times,
  preserve complete stderr, and check both cancellation and leader-exit descendant
  cleanup. Reaping and output drain are attempted even after cleanup errors.
- **Storage-independent control:** pipe readers and a separate bounded persistence
  worker replace inline writes in the controller. The timeout clock is captured
  at launch, before any persistence await. Active cancel and shutdown signals
  use only in-memory ownership; no database-dependent mutex is held. A separate
  read pool and cheap bounded availability probe serve routine status. Explicit
  quick checks have a five-second diagnostic/startup bound. Tests hold a SQLite
  `BEGIN IMMEDIATE` lock, release a command through a FIFO, and confirm timeout,
  cancel, and shutdown reap the child while the test still owns the write lock.
  The two-second forbidden side effect never occurs.
- **Stalled HTTP shutdown:** authenticated bodies and headers have deadlines.
  The server owns connection tasks, stops accepting, completes process cleanup,
  then allows a bounded connection drain before aborting/joining remaining tasks.
  Tests exercise body timeout, incomplete authenticated POST, unread SSE, clean
  SIGTERM exit, metadata removal, and reacquisition of the daemon ownership lock.
- **Admission:** configurable active/reserved/finalizing run capacity and pending
  admission capacity return HTTP 429 with `code: overloaded`. Separate HTTP
  submission/SSE caps preserve control capacity. Tests saturate active and pending
  admissions and simultaneously occupy 32 incomplete bodies plus 32 SSE streams,
  while status/cancel/shutdown remain usable. `100 Continue` provides an explicit
  synchronization barrier for incomplete-body occupancy.
- **Submission idempotency:** migration 0002 adds a unique persisted submission
  key. Matching typed payloads return the existing run; conflicting payloads return
  409. CLI keys survive automatic request retries and can be supplied explicitly
  with `--submission-key`. Tests cover lost responses, conflicts, repeated retries,
  one observed external execution, and key reuse after daemon crash/restart.
- **Power recovery:** a reference-counted monitor detects helper loss, publishes
  durable warnings to eligible run writers, and reacquires with exponential
  backoff capped at two seconds and one second per acquisition. The final lease
  cancels recovery. A paused-clock test verifies the schedule and stopping rule.
  A macOS test kills the actual helper during two active runs, observes persisted
  recovery, checks a replacement PID and `PreventUserIdleSystemSleep` assertion
  with `pmset`, and verifies final release. Display sleep is permitted.
- **Log follow:** the CLI advances its cursor only after consuming an event,
  accepts `--after`, limits reconnects to five, and reloads metadata/token during
  retry. It checks the durable terminal tail on graceful stream EOF. The CLI
  acceptance test follows across daemon death/restart and token/port rotation,
  checking increasing, nonduplicated cursors and the final interruption event.
- **Validation/CI:** CI on macOS/Linux explicitly runs fmt, warnings-denied Clippy,
  tests, workspace build, and docs. Rust 1.89 compatibility is checked separately.
  Polling used for OS/file readiness is bounded; normal run/event readiness uses
  notifications, FIFO barriers, SSE, or HTTP acknowledgements.
- **Protocol boundary:** serializable contracts and their lightweight value
  validation now live in `autono-protocol`. Core re-exports preserve its existing
  type paths. The protocol depends on Serde, Chrono, UUID, and anyhow, without
  Tokio, SQLx, process code, or any extra crate/framework.

## Local validation

Validated on macOS with the full fmt/Clippy/test/build/doc commands in the README,
plus Rust 1.89 `cargo check --workspace --all-targets --locked`. The suite contains
33 passing tests, including the live macOS power-helper test. Linux CI is
configured; it was not executed locally in this macOS environment.

The original command with five iterations and five-second sleeps was also run
manually through separate CLI processes: submission exited, a second CLI observed
RUNNING, follow returned all five steps, status became SUCCEEDED, and SIGTERM
removed metadata cleanly. Cancellation, lock-stalled timeout, reset/retry,
repeated failures, incomplete-client shutdown, admission saturation, helper loss,
and honest crash reconciliation are covered by the regression scenarios above.
Source/diff inspection found no unfinished core TODOs or stub implementations.

## Intentional limits and re-review focus

Submission idempotency is **not exactly-once external effects**. Hard daemon death
can leave children executing. Restart never adopts or signals a saved identifier;
unfinished durable runs become INTERRUPTED and matching retries return that
record. Detached descendants can escape groups. OS refusal/uninterruptible work
can prevent confirmed termination, which is reported separately from execution.

Output retries and HTTP drains have explicit bounds; finalization retries retain
ownership while the daemon runs. Persistent storage failure causes explicit
unhealthy admission restriction and unsuccessful bounded shutdown, not silent
abandonment. SQLite's worker can finish an already-issued operation after an
async finalization future is cancelled, so the runtime does not assert that
cancelling a database future rolled back an operation. Admission includes
finalizing attempts to keep this work bounded. There is no workflow queue, log
retention, sandbox, service installer, automatic resume, or full Windows tree
supervision.

The final independent review confirmed the finalization ownership repair without
remaining P0/P1 findings. The non-reaping Unix ownership boundary, daemon-owned
admission handoff, controller/writer separation, and bounded shutdown sequencing
remain important invariants for future changes.
