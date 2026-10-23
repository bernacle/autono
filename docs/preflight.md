# Autonomous preflight and human escalation v0

The daemon owns preflight, execution, human resolutions and continuation. Closing
the submitting terminal does not cancel them. Start the daemon separately and
inspect the persisted plan and checklist before approval:

```sh
autono plan feature.md --repository /absolute/project --base main
autono show-plan <run-id>
autono preflight <run-id>
autono blockers <run-id>
autono resolve <run-id> <action-id> --request resolution.json
autono preflight <run-id>
autono execute <run-id>
# If a task later pauses for a human:
autono blockers <run-id>
autono resolve <run-id> <action-id> --request resolution.json
autono continue <run-id>
autono tasks <run-id>
autono handoff <run-id> # verified local progress and exact remaining prerequisites
autono integrate <run-id> # after VERIFIED, as before
```

`preflight` prints JSON containing readiness checks, stable action IDs, human
actions with evidence/confidence/task relationships, and the snapshotted policy.
It runs before planner invocation and after planning, and refreshes before initial
execution and continuation. Known essential failures hold planning without spending
planner/coder inference; repair externally and refresh preflight. A never-invoked
planner admission may proceed, but an already-started planner is never replayed. It does not approve the plan. `execute` requires a READY
report and explicitly approves the immutable plan and verification commands.
Preflight is best effort: it cannot guarantee discovery of all future blockers.

## Checks and planning decisions

Checks inspect repository accessibility and pinned Git ownership, developer Git
working state, installed provider executables, configured providers' advertised
noninteractive/sandbox capabilities, verification executables, declared tools,
required directories in the planning checkout, and required environment variable
presence. A dirty developer checkout is informational: execution uses isolated
worktrees at the pinned commit. Checks never execute verification commands in the
developer checkout. Executable availability does not prove a shell script's
internal commands or an external service will work.

Authentication validity is reported as UNKNOWN when it cannot be established safely.
A2 bare mode additionally requires nonempty ANTHROPIC_API_KEY presence in the daemon
environment before planning; the value is never retained and presence does not
prove validity.
Autono does not inspect credential files or run potentially credential-bearing
login diagnostics. The existing provider login mechanism remains authoritative;
a structured authentication error during execution produces a human pause.
Capability probes are help-only, supervised, noninteractive, time bounded and
inference free. They are separate from invocation construction and require no
prompt/schema artifacts; Claude readiness never opens a placeholder schema.
Missing or incompatible CLIs block readiness. Both installed provider executables
are reported; all configured planner/coder/reviewer providers gate readiness.

The existing read-only planner also reviews the specification and architecture
for material unresolved decisions and declared permission/service prerequisites.
Its `human_requirements` include a reason, exact requested action, affected tasks,
blocking flag, evidence and confidence. Empty arrays are valid and encouraged
when there are no material questions. Proposals do not constitute permission and
semantic completeness still needs human inspection. Plans remain immutable;
explicit resolutions supplement them for both coder and independent reviewer.

User `config.toml` or project `autono.toml` can declare prerequisites:

```toml
[preflight]
required_env = ["SMTP_PASSWORD"]
required_tools = ["git", "cargo"]
required_directories = ["src"]
external_services = ["smtp"]
```

A supplied preflight table replaces the inherited preflight table. Environment
names and service names are identifiers, never values, credential URLs or secrets.
Directories must be relative and resolve inside the pinned planning checkout.
External service readiness requires explicit human confirmation; v0 does not
contact services. Failed deterministic checks cannot be waived with a resolution:
the probe must pass. Correcting a host prerequisite and refreshing preflight
records an audited system resolution. Environment changes generally require a
daemon restart because children inherit the daemon's environment. Configuration
and policy are immutable run snapshots; changes to their declarations require a
new plan.

## Human action lifecycle

Human actions have stable `H-...` IDs, type, reason, affected tasks, requested
action, status, resolution, timestamps and append-only audit history.

- PENDING: awaiting an explicit action or a successful deterministic host check.
- RESOLVED: decision/configuration confirmation, permitted approval, or successful
  deterministic recheck. System rechecks never grant permissions.
- DEFERRED: retained for later. A blocking action still blocks readiness/work;
  nonblocking actions may be deferred without preventing execution.
- REJECTED: records denial. Required work is not silently skipped or completed.

`blockers` lists all statuses and their audit history; `summary` includes the full
run. Existing `findings`/legacy blocker diagnostics and process logs remain
available. Runtime task pauses use BLOCKED_HUMAN, distinct from ordinary FAILED,
integrity BLOCKED, and dependency WAITING. A human-paused dependency leaves its
unfinished descendants WAITING. Independent tasks continue through verification,
review and preservation. Healthy workers are not cancelled by another task's
human pause. When all runnable work finishes, the run is BLOCKED and inactive.

## Resolve and continue

Resolution JSON is an explicit user request with a reusable UUID:

```json
{
  "request_id": "b4bcfa15-565d-42c5-8304-0bdedac684bd",
  "expected_action_version": 1,
  "disposition": "resolve",
  "note": "Invitations expire after seven days; send a new invitation afterward",
  "effect": "not_performed",
  "permission": null
}
```

Dispositions are `resolve`, `approve`, `reject`, `defer`. Effect states are
`not_performed`, `already_performed`, `unknown`. Notes contain product choices or
nonsecret setup evidence. Configure credentials through the existing external
mechanism; **never enter credentials into a resolution, specification or prompt**.
An input guard rejects recognizable credential material; arbitrary text is not a
general secret detector. Environment values are inspected only for nonempty presence and are never
retained or exposed by preflight.
Existing process/provider logs retain their Runtime v0 behavior, so providers and
trusted commands must obey the prohibition on printing secrets.

New clients bind `expected_action_version` to the observed action audit length
(the handoff exposes it directly); stale versions conflict. Legacy clients retain
identity/status guards. Retry an ambiguous resolution with exactly the same UUID
and payload. Matching
retries return the original accepted response snapshot; a reused UUID with
another payload/action/run conflicts. Inspect `summary` for current workflow
liveness: the resolution response describes the revision that accepted it.
Receipts predating migration 0005 have no historical response snapshot and return
the current run view while retaining their original request identity.
State, resolution receipt, checklist projections and lifecycle event commit in
one SQLite transaction. Active requests share the workers' serialized journal;
inactive requests serialize with admission/cancellation. Once accepted, resolution
mutations are bounded and tracked by the daemon through commit, in-memory
publication and reply delivery, even if the HTTP/CLI waiter disconnects. Shutdown
drains accepted mutations. Ambiguous commit errors trigger a durable reread and
journal reconciliation; if the state cannot be established, the workflow stops,
health reports the failure and shutdown fails honestly. This does not replay
agents or resume an interrupted workflow. Cancellation never waits
for a resolution/database lock before signalling active processes.

In Supervised mode a resolution alone never launches a worker. `continue` explicitly schedules
eligible resolved live pauses, including alongside healthy independent workers.
A2 delegates this continuation to the daemon under the original immutable grant;
resolution and repaired preflight may re-admit eligible work after all relevant
authority/readiness checks pass. They cannot renew an expired grant.
It never replans, recreates the task worktree, remerges dependency commits, resumes
a suspended provider tool call, resets attempt/review limits, or reruns completed
tasks. It validates pinned Git ownership, branch/HEAD, index and the saved
workspace snapshot before a fresh invocation. Edited paused workspaces need a new
trusted plan. Completed task documents, commits and verification/review evidence
are immutable. Fresh checks, independent review and preserved-tree equality remain
required after continuation; integration still runs combined verification.

Before a live pause can continue, the developer must inspect the operation and
explicitly state whether its external effect happened. UNKNOWN effects keep it
blocked. An `already_performed` resolution is delivered to the fresh provider with
an instruction not to repeat that effect. Applicable resolutions are reconstructed
from durable actions as authoritative context for every invocation, including
fresh sessions and retries, separately from mutable diagnostics. Autono does not
claim exactly-once external effects or independently verify the developer's
statement. Privileged
external operations are unsupported and stay blocked by the broker/adapter.

After a task has required runtime effect reconciliation, a failed invocation,
failed verification, or review rejection cannot automatically start another
coding invocation. The adapters provide no authoritative external-effect receipt.
The task returns to BLOCKED_HUMAN with an `effect_reconciliation` action; inspect
the last invocation and provide a new resolution before explicitly continuing.
This rule applies even when the previous resolution said `already_performed`:
that statement describes the inspected checkpoint, not proof about later
invocations. Attempt and review budgets are preserved, including after a new
resolution; exhausted budgets cannot be bypassed. Ordinary retries without
runtime effect context still receive all applicable product decisions.

Generic missing prerequisites and coder-requested human decisions can continue
within existing bounds. Provider permission denials without a safely grantable
operation, ownership/instruction/configuration mutations, merge conflicts, and
planning/integration/integrity failures require external inspection/new trusted
planning; a textual resolution cannot make them continuable. Deferring an action
does not bypass required tasks, checks or review.

## Permission policy and limits

Policy is user-only `config.toml`, snapshotted with each run. Project files cannot
override it:

```toml
[permissions.rules]
workspace_write = "ALLOW"
run_checks = "REQUIRE_APPROVAL"
protected_configuration = "REQUIRE_APPROVAL"
install_dependencies = "REQUIRE_APPROVAL"
environment_change = "REQUIRE_APPROVAL"
outside_workspace = "DENY"
destructive_migration = "DENY"
external_operation = "DENY"
```

These are the defaults except `run_checks`, whose default is ALLOW. Preflight
classifies the baseline workspace/check operations and all proposed permission
requirements. Only the generated run-wide scopes for workspace writes in assigned worktrees
and immutable configured checks have an executable broker approval path in v0.
Other scopes, even for the same operation category, remain unsupported. For approval,
copy the **exact** stored `permission` object into a request with disposition
`approve`. Different scope/operation, textual approval via `resolve`, DENY policy,
and unsupported adapter operations are rejected. ALLOW policy does not expand
adapter capabilities. Broader operations remain blocked even with explicit user
approval. Configure prerequisites externally or establish a new trusted base.

Codex `-a never`, workspace-write/read-only sandboxing, ignored customization,
and Claude safe mode/tool allowlists/noninteractive permissions remain unchanged.
The broker is a harness admission policy for declared operations, not syscall
interception or OS sandboxing. It cannot identify every operation inside agent
tools or developer-configured shell commands. Existing mutation audits and
verification/review authority remain in force. Trust the repository, installed
providers and configured commands as required by SECURITY.md.

## Persistence and interruption

Additive migration 0004 adds inspectable preflight/action projections and immutable
resolution receipts. Migration 0005 adds immutable original resolution response
snapshots to those receipts, in the same run/event transaction. The versioned run
document, history and projections commit atomically. Existing migrations and process supervisor state machines are intact.

There is no generalized crash-safe resume. Startup still reconciles active
PLANNING/RUNNING/INTEGRATING runs to INTERRUPTED, including human-paused tasks in
an otherwise active run. Blockers/resolutions and completed evidence survive, but
`continue` refuses INTERRUPTED/CANCELLED/FAILED workflows. Inspect surviving
processes and external effects, then create a new plan when safe. No saved PID is
adopted or signalled. A fully drained, inactive BLOCKED live pause may be explicitly
continued after restart, with the same snapshot/effect checks. Planning and
integration failures are not resumable stages.

Normal tests use deterministic injected agents, real disposable Git worktrees,
SQLite and process supervision. They cover preflight, decisions, exact approvals,
duplicate/concurrent requests, cancellation/restart, live continuation, dependent
waiting, unchanged completed evidence, effect checks and authoritative integration.
No normal test consumes inference. The existing paid real-agent acceptance test
remains explicitly opt-in and is not part of this milestone's automatic checks.

## Reproduced M3 review regressions

The deterministic counter regression creates an external `x`, pauses for a human,
then fails the first continued invocation. The harness stops before an automatic
fresh retry can turn it into `xx`; a new human checkpoint is required. A separate
fresh-session retry test checks that product decisions survive overwritten error
diagnostics.

A test-only journal barrier stops resolution publication after SQLite commit.
Cancelling the request waiter at that exact point leaves the owned mutation
running. A concurrent duplicate and shutdown both wait; releasing the barrier
publishes the committed state, returns the original receipt, drains shutdown and
leaves no RUNNING workflow or stale controller journal.

Production preflight tests run in isolated subprocess environments with fake
Claude/Codex executables first on PATH. Both coder/reviewer provider combinations
are checked, compatible Claude reports READY without dummy artifacts, and
missing safety capabilities block readiness. An argv log permits only help
commands. These tests never invoke installed providers or inference APIs.


## A4 task-scoped prerequisites and handoff

A2 already allows independent work when only task-scoped actions are pending,
even when full preflight is NOT READY. `independent_work_ready` exposes that
separate admission fact without declaring the complete run ready. Run-wide
configuration prerequisites remain essential unless the frozen plan identifies
an exact safe scope. Supervised approval still requires full READY.

`autono handoff <run-id> [--json]` and the authenticated `/handoff` endpoint report
retained implementation, complete configured check receipts, independent local
verification, failures, exact unresolved actions and actual waiting descendants.
The daemon checkpoints this projection with every run change. BLOCKED with partial
verified work remains BLOCKED; local/mock tests do not establish real-service
integration. Legacy artifacts lacking modern tree evidence are retained as
implemented with incomplete evidence rather than relabeled unimplemented or
certified by a new receipt. Migration 0008 appends immutable prerequisite audit
rows alongside the existing Human Action projection and resolution receipts.

See [A4 architecture, classification and review evidence](m5-intelligent-blockers.md).
