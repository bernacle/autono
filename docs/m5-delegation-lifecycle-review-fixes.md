# M5 A2 independent review: lifecycle repairs

Review baseline: `8d5f01e676b7a8573f02b99dba6c394b76948bcc`, Draft PR #5.
These repairs address the confirmed P1/P2 findings without adding a new grant,
workflow mode, desktop surface, migration, or protocol field.

## P1: authority was attached after admission

`ProcessRunner::start_spec` previously awaited daemon-owned `submit_keyed` and
only then installed a watcher that forwarded workflow cancellation to the
supervisor's independent token. SQLite contention could hold that admission
while the workflow was cancelled. Releasing the write lock let the supervisor
create and launch the process before the watcher existed.

The runner now submits an internal `ExecutionAuthority` containing the original
workflow token and absolute delegation deadline. The daemon-owned admission job
retains those controls while queued and while reconciling SQLite. Execution
checks them after durable creation and again after power acquisition, immediately
before synchronous spawn. Its running control loop directly selects workflow
cancellation and the authorization deadline alongside daemon shutdown, explicit
process cancellation, command timeout, persistence failure, and observed exit.
The runner also checks authority before submission and every capacity retry.

An in-flight SQLite mutation is **never dropped to implement cancellation**.
Once that mutation settles, stopped work receives the existing finalization
machinery's durable unstarted receipt: no PID, no start timestamp, complete empty
output, `CANCELLED` for cancellation, `FAILED` with an authorization-deadline reason
for expiry, or `INTERRUPTED` for shutdown. Queued duplicate keys reconcile to the
same identity. Replay returns the existing outcome; it cannot launch a second
process or reset the original execution controls. Conflicting argv remains a
submission conflict.

Observed exit retains precedence when exit and stop are both ready. A process
already observed to succeed is never rewritten as cancelled because of a late
signal or keyed replay. Processes stopped after launch retain their actual PID,
start, termination/reaping result, output completeness and cleanup warnings.

## P2: preparation started before the deadline watcher

Inactive execute, continue and preflight previously ran preparation using an
unbounded runner and the parent token. The deadline watcher was installed only
when preparation returned and the execution engine was constructed. Help-only
provider capability processes could consequently start after grant expiry.

The service now establishes a child cancellation scope, obtains the original
immutable expiry (or establishes the initial authorization lifetime), scopes the
runner to that expiry, and installs the watcher **before preparation**. The same
runner/token reaches repository probes, provider readiness, preparation,
planning, verification, independent review, continuation and integration.
Nested runner scopes can shorten a deadline but cannot extend or remove one.

Absolute time is checked independently of watcher scheduling before every process
submission. Provider readiness checks authority between probes rather than
converting expiry to an ordinary failed capability check. Preflight checks it
before publishing its result. Preparation checks it again on both success and
failure so probe errors cannot hide expiration. Scheduling, execution admission,
continuation, integration admission and successful completion checkpoints also
recheck their controls. Evidence and failure checkpoints remain writable after a
stop, preserving completed work and an explicit durable reason.

The watcher has a cancellation-token drop guard covering every return and unwind
path. Expiry cancels the child token; developer cancellation and daemon shutdown
retain their distinct parent signals. Supervised operations have no delegation
deadline and retain their existing explicit approval workflow.

## Deterministic regression evidence

The external `boundary.rs`, `run-lifecycle-tests.sh` and `lifecycle-results.txt`
were not available locally. The equivalent fixtures are committed in:

- `crates/autono-core/tests/admission_authority.rs`: an independent
  `BEGIN IMMEDIATE` transaction blocks keyed creation. The test waits for an
  acquired execution slot and two pending identical submissions, cancels/expires
  authority (or shuts down), then releases SQLite. It asserts no child marker,
  no PID/start timestamp, the correct durable terminal receipt, matching replay
  after reopening SQLite, conflicting-key rejection, and cancellation after the
  original admission waiter disconnects. Additional tests cover
  stopped-before-submit, capacity retry, a deterministic power-acquisition yield
  immediately before spawn, running cancellation/expiry, and late cancellation
  after observed success.
- `crates/autono-core/tests/delegation.rs`: inactive preflight/execute/continue
  begins with under one second left. A real capability process is held on a
  release gate across expiry. Even a fixture that attempts the next probe after
  its first process is stopped cannot admit it. No successful preflight replaces
  the earlier checkpoint; the durable reason identifies runtime exhaustion.
  Cancellation is covered separately. Planner/reviewer preparation, configured
  verification, local integration, original-consent replay, completed-task
  preservation and original-checkout isolation have actual lifecycle coverage.
- `crates/autono-cli/tests/delegation.rs`: a separate authenticated disposable
  daemon uses the production Claude adapter with a private shell-only fake CLI.
  Inactive preflight starts approximately 900 ms before expiry. The first
  help-only capability process is gated across expiry. Exactly **one** probe
  starts; **zero** subsequent probes or coding processes start. The operation
  fails, its previous preflight is retained, and the authenticated run records
  `delegation_denied: total runtime budget exhausted`. Existing generic HTTP
  error redaction remains unchanged.

[Recorded daemon evidence](m5-delegation-lifecycle-evidence.json) includes the
expired preflight and a new unattended successful safe run:
`f0fd9309-9af7-4ac7-9e3f-aba62dd25e7c`. It completes coding, a safe retry,
deterministic verification, independent review and local integration at
`35853bc31bce836e69c7e4a6400cd9036647edb0`, with no routine manual execute,
continue or integrate event. Original main remains
`9fce1ae9d147f3371f459d43aa31e8c401839fcc`. Restart does not replay it.
No paid inference or installed coding-provider invocation is used.

The rebuilt native Tauri app was also terminated during run
`6efb98a5-ddb8-4200-adec-bec00d46f5d5`. The separate daemon remained alive and completed
local integration at `9fa2f37ac2965365d0f8a3511637bb996a96f0e4`; original main
was unchanged. Both owned processes were stopped after this smoke test. The
machine-readable evidence records the desktop/daemon process IDs and run states.

## Validation

The complete delivery validation matrix and current CI results are maintained in
[Draft PR #5](https://github.com/bernacle/autono/pull/5). Reproduce the lifecycle
checks with `cargo test -p autono-core --test admission_authority`,
`cargo test -p autono-core --test delegation`, and
`cargo test -p autono-cli --test delegation`. The full validation additionally
runs the workspace suite, strict Clippy/formatting, frontend tests/lint/typecheck,
production builds, Rust 1.89 checks and native Tauri packaging.

The existing immutable-consent, conflicting-authorization, legacy/Supervised,
authenticated native bridge, migrations, persistence, unknown-effect and
original-repository tests remain enabled.

The earlier Ubuntu CI failure occurred at the mandatory bubblewrap capability
probe. The disposable hosted Linux runner now explicitly enables unprivileged
user namespaces and smoke-tests bubblewrap before running the unchanged sandbox
acceptance tests. This affects CI setup only; runtime sandbox requirements,
filesystem/network confinement and fail-closed admission remain unchanged.
[Ubuntu documents this AppArmor namespace restriction](https://ubuntu.com/blog/ubuntu-23-10-restricted-unprivileged-user-namespaces).

## Remaining limits

SQLite reconciliation/finalization may finish after cancellation or expiry;
process authority does not permit a new child during that wait. OS spawn,
signalling and natural exit can race: the controller retains the observed result
and performs termination/reaping rather than claiming atomic or exactly-once
external effects. Existing cleanup uncertainty is retained in receipts. These
fixtures prove owned local process behavior, not provider billing or hostile
same-UID isolation. Crash recovery still interrupts active runs and never adopts
saved PIDs or blindly resumes uncertain effects. A2's original tool-free provider,
fixed checks, isolated integration and spending-accounting limitations remain.
