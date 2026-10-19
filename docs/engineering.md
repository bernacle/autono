# Autonomous engineering loop v0

Autono is the harness. The Rust daemon owns plans, approval, task states,
dependencies, workspace ownership, retries, deterministic checks, review cycles,
and integration. Providers supply proposals and code; provider output cannot mark
a task completed. The existing process `Run` is still a process attempt.

## Local workflow

Start `autono daemon` in a service manager or persistent terminal. All work after
submission belongs to the daemon; the submitting CLI can close immediately.

```sh
autono plan feature.md --repository /absolute/project --base HEAD
# Prints an engineering UUID; planning continues in a separate read-only checkout.
autono status --engineering
autono show-plan <id>
# Inspect original requirements, task graph, acceptance criteria, and exact checks.
autono execute <id>              # Explicitly approves that immutable plan.
autono tasks <id>
autono findings <id>
autono logs --engineering --follow <id>
autono summary <id>
autono integrate <id>            # Only after all tasks are VERIFIED.
autono cancel --engineering <id>
```

`execute` and `integrate` return after durable acceptance; inspect status for their
outcomes. A VERIFIED run has individually checked/reviewed task commits. Only a
COMPLETED run has also passed integration checks. Integration is a deliberate
local operation and never merges into main or creates a PR.

The plan command prints its ID to stderr before submitting. After an ambiguous
response, reuse `--plan-id <id>` with identical repository/base/specification.
Retrying a stored plan returns it without another planner invocation. Duplicate
execute/integrate requests cannot start duplicate workers. There is one active
engineering run per daemon; submit another after it becomes inactive.

Engineering logs are durable JSONL lifecycle events, including session keys and
verification process IDs. `tasks`, `findings`, and `summary` include attempts,
process references, exit codes, bounded diagnostic excerpts, full structured
findings, and preserved workspaces. For raw/full provider or check output use
`autono logs --events <process-id>` or existing process SSE. The `agent_process`
and `verification_process` events expose process UUIDs
as soon as Runtime v0 accepts them; agent sessions also record the process UUID
while running. Submission keys remain available for correlation after a crash.
Provider JSONL is durably streamed by Runtime v0; final structured results are
decoded after process completion. No provider transcript is fed to the reviewer.

## Configuration

User `config.toml` and project-root `autono.toml` support these fields. Partial
project settings override corresponding user fields; runner admission and daemon
settings remain user-only. Settings are snapshotted when planning, shown before
approval, and immutable for the run. Commands are argument arrays, with **no
implicit shell**. Use `["sh", "-c", "..."]` explicitly when a shell is needed.
Only developer configuration can choose commands; plans provide verification
expectations, never executable commands.

```toml
[agents.planner]
provider = "claude"
timeout_seconds = 600

[agents.coder]
provider = "codex"
# model = "your-installed-provider-model"
timeout_seconds = 600

[agents.reviewer]
provider = "claude"
timeout_seconds = 600

[execution]
max_parallel = 2
max_attempts = 3
max_review_cycles = 2

[verification]
commands = [["cargo", "fmt", "--check"], ["cargo", "test"]]
timeout_seconds = 300
```

Parallelism is 1..8, coding attempts 1..5, and review cycles 1..5. A review cycle
includes each reviewer invocation, including invalid-response and infrastructure retries.
These retries review the same candidate and never consume coding attempts. Coding attempts bound all implementation invocations,
including fixes after failed checks. Each task stops on either exhausted bound.
Planner inference runs once per new plan. Agent/check timeouts are 1..3600 seconds;
Git operations have a 60-second timeout and capability probes 15 seconds.
Temporary process admission overload retries six times with cancellable exponential
backoff (50 ms through 1.6 s; 3.15 s total), preserving the submission key.
Parallelism cannot exceed the runtime active-process limit; unrelated raw process
runs can still temporarily occupy those slots. These
are invocation/time bounds, not a token or monetary budget. Verification requires
1..16 configured commands. Defaults target a Rust repository; configure other
projects explicitly.

## Domain and persistence

`autono-protocol::engineering` defines EngineeringRun, ExecutionPlan, Milestone,
EngineeringTask, TaskAttempt, ReviewAttempt, AgentSession, Workspace, GitOwnership, VerificationResult,
ReviewFinding, CriterionReview, and Blocker. Engineering IDs and session/process
keys are UUIDs; plan task/milestone/requirement IDs are stable strings.

Migration 0003 adds `engineering_runs` (versioned aggregate documents),
`engineering_events` (append-only, increasing cursors), and
`engineering_workspaces` (unique path and repository/branch ownership). Nested
domain objects persist in the aggregate so a checkpoint and all related evidence
can commit atomically with lifecycle/task transition events. Existing runtime
tables and migration files are unchanged. A serialized journal and optimistic
revision check prevent stale writers. An OS file lock leases each owned workspace
through coding, checks, review, and artifact preservation. Workspace documents
retain pinned Git paths/configuration fingerprints. Each coding attempt retains
every reviewer session, isolated checkout, result, and failure diagnostic; the
latest reviewer/review fields remain available for existing consumers. New fields
have deserialization defaults so old records remain inspectable. Newly executing
plans require pinned ownership; legacy plans lacking it must be resubmitted.

Run lifecycle:

`PLANNING → AWAITING_APPROVAL → RUNNING → VERIFIED → INTEGRATING → COMPLETED`

Active stages can stop as FAILED, BLOCKED, CANCELLED, or INTERRUPTED. Tasks use
WAITING, READY, RUNNING, BLOCKED, FAILED, COMPLETED, CANCELLED, and INTERRUPTED.
Only completed dependencies unblock a task. Failed/blocked dependency states
propagate to descendants; independent tasks can finish. A blocked run does not
leave an agent waiting for interactive stdin. There is no unblock/resume command:
inspect retained work and submit a new plan after resolving prerequisites.

Plans require 1..32 tasks, 1..64 requirements, 1..8 milestones, unique IDs, known
dependencies, no cycles, acceptance criteria, verification expectations, full
requirement-ID coverage, and exactly one milestone per task. Topological ordering
is deterministic. Semantic completeness still needs developer plan inspection;
the harness cannot prove an LLM captured every nuance of prose.

## Providers

The injectable `AgentRuntime` prepares an invocation and decodes structured
events/results. `ProcessRunner` supplies streaming persistence, closed stdin,
timeout, cancellation, and Runtime v0 process ownership. All workflow Git,
capability probe, agent, and verification commands use the existing supervisor.
No API client, proxy, key manager, or authentication-file reader is introduced.

The adapters were developed against installed Codex CLI 0.163.0 and Claude Code
2.1.303. Each invocation inspects actual installed help and blocks on missing
required capabilities. It uses the user's existing CLI login. Model selection is
optional; Autono does not read, copy, or persist CLI authentication tokens.

- Codex: `--no-daemon -a never -s <mode> exec --json --ignore-user-config
  --output-schema <file>`. Coder uses workspace-write; planner/reviewer read-only.
  Explicit session UUIDs support `exec resume` for coder fixes. Global sandbox
  and approval options also apply to resume. Ignoring customization retains CLI
  authentication. `thread.started`, final `agent_message`, `turn.completed`, and
  error envelopes determine the result; exit zero without structured completion
  is rejected. Shared app-server execution is disabled to retain local process
  ownership.
- Claude: print/stream-json/verbose with a JSON schema, safe mode (preserves
  authentication while disabling custom hooks/plugins), strict MCP selection,
  and `--permission-prompts none`. Coder exposes Read/Glob/Grep/Edit/Write with
  acceptEdits; planner/reviewer expose only Read/Glob/Grep with dontAsk. The
  artifact directory is an explicit additional directory. Coder fixes can resume
  an explicit UUID. Result envelopes must have successful structured output;
  permission denials become manual blockers.

Reviewers always start a separate session in a separate checkout of the original
trusted base commit. Authoritative README/AGENTS/CLAUDE/architecture context is
read from regular Git blobs at that exact commit, never coder-modified files or
symlink targets. Reviewers receive the original task, its referenced requirements,
stable criterion IDs, complete changed-file inventory/statuses, diff statistics,
explicit truncation state, a session-local full diff artifact/digest, and verification
evidence. The complete delta from the original base includes completed dependency
changes. The reviewer checkout intentionally contains base code; candidate changes
are presented as diff data. Prompts mark all diffs, filenames and test output as
untrusted data, never authority to approve or override review instructions.
They do not receive coder conversation/reasoning. Criterion labels may be paraphrased;
immutable task-ID/ordinal identifiers, schema enums and exact coverage checks bind
the evidence. Invalid reviews retry a fresh reviewer within the same review budget.

Instruction/configuration changes (including nested AGENTS/CLAUDE files and agent
configuration directories) block before verification/review, requiring a human to
approve them into a new trusted base. There is no implicit plan-based exception.
Build/test/verification configuration changes are explicitly flagged for scrutiny.
The harness does not claim prompts alone make a reviewer immune to injection. Critical,
high, and medium findings block completion. Every criterion must have explicit,
nonempty evidence and be marked satisfied. Structured findings include severity,
file, line, explanation, recommended fix, and confidence.

Authentication/approval failures are classified only from recognized structured
provider envelope codes/permission-denial fields. Runtime cancellation/timeout
outcomes take priority. Arbitrary transcript/stdout/stderr words cannot create
credential or permission blockers; unknown failures stay provider failures. Other protocol/provider errors and timeouts consume bounded attempts.
Unrecognized interactive behavior is stopped by the invocation timeout, not an
unbounded stdin wait. The daemon does not broker approvals or automatically log
in, install CLIs, or grant broader permissions. Claude coders intentionally lack
Bash: project commands belong to deterministic verification.

## Workspaces, verification, review, and integration

The base must resolve to a Git commit in the submitted repository root. Developer
uncommitted changes are not copied. The harness reserves workspaces under
`<home>/engineering/<run>/` before creation and creates unique `autono/<run>/...`
branches without force. Harness Git uses explicit pinned Git-directory/worktree paths, clears inherited
Git redirection variables, ignores system/global configuration, disables hooks,
fsmonitor, external diffs/textconv and signing, and rejects executable filter/merge/
diff drivers, configuration includes, alternate object stores and grafts. Ownership
checks fingerprint the .git pointer, back-reference, commondir, repository/worktree
configuration, attributes and alternate-store metadata before Git execution.
Changed ownership metadata is rejected, not followed. Raw HEAD/index state is
audited too: coder changes to index flags/staging block rather than hiding code
from preservation, and optional Git index refresh is disabled. Git operations use the
standard `env -u` utility available on supported macOS/Linux hosts. Developer
repositories using unsupported Git configuration must simplify it before planning.

Task checkouts merge verified direct-dependency commits
before coding. Existing paths/branches are rejected, never reset, removed, or
reused. Workspaces survive failure, cancellation, and completion.

The coder must leave commits and branches to the harness. After coding, the
harness stages the local changes, runs all configured checks, then asks a fresh
reviewer. Required checks must have observed zero exit and complete persisted
output. Evidence distinguishes test failures from timeout, cancellation, spawn and
infrastructure failures. Infrastructure problems stop with an explicit blocker,
without asking the coder to repair the implementation. Runtime cleanup/power warnings remain distinct and visible; they do not
rewrite an observed command result. Bounded diagnostic excerpts go into prompts;
full output remains in process events. Full diffs are retained as artifacts;
review prompts include up to 48,000 characters plus their full artifact path.

Snapshot audits cover HEAD, current branch, index, tracked files, and nonignored
untracked files, including file modes and symlink targets. Audit limits are 4096
files / 64 MiB; captured command output is capped at 2 MiB per stream in memory
while full logs remain durable. Each invocation has its own UUID artifact directory, exposed only to that
session by provider permissions. Prompts, schemas and diffs are sealed read-only;
Unix directories are mode 0500. Fingerprint/permission checks cover all sealed
artifacts in the active operation before and after invocations. Large prompts
use the same sealed session-local instruction file, never a shared writable path.
This prevents ordinary permitted writes and detects retained mutations; it is not
same-UID OS isolation. Prompts over 64 KiB use instruction-file references to avoid
OS argv limits. Resumed coders receive only new feedback plus stable context/task
references; new independent reviewers still receive their own trusted context. Project context is read only from regular blobs at the trusted original base.
Checks that change audited files block completion.
A reviewer mutation rejects the review and records a blocker without resetting
the evidence. The harness commits only after checks, accepted review, criterion
coverage, and mutation audit agree. SQLite independently rechecks referenced
process results before committing COMPLETED task state.

Provider tool restrictions and mutation audits are **not a security sandbox**.
The audit excludes ignored build artifacts and Git shared metadata outside the
explicit ownership/configuration fingerprints and recorded HEAD/branch/index. It cannot detect transient/reverted writes or
changes outside the assigned checkout/artifacts. Same-user programs with arbitrary
command access can chmod files or race/restore writes; they are outside the
permission/audit guarantee. Runtime v0's detached-descendant and
uncertain-cleanup limitations still apply. Use trusted projects/providers and
inspect reported cleanup warnings. Do not claim guaranteed reviewer isolation.

Explicit integration creates another owned checkout from the original base and
merges task commits in topological order. It checks each task branch/HEAD and
clean status, then runs combined verification and a mutation audit. Conflicts
remain in place with an explicit blocker. Failed combined checks preserve their
process evidence and yield FAILED. No automatic conflict repair, push, PR, or
merge to a developer branch occurs.

## Recovery and operational limits

The daemon registers admission/execution work independently of HTTP waiters.
Normal cancellation signals the currently owned process and stops new work;
workspaces and checkpoints remain. Workflow shutdown is signalled alongside the
existing supervisor shutdown, with a bounded checkpoint wait. Persistent
workflow checkpoint failure is reported through `/v1/engineering/health`, prevents
new engineering work, and causes unsuccessful shutdown when unresolved.

After exclusive daemon ownership and process reconciliation, startup changes all
PLANNING/RUNNING/INTEGRATING workflows to INTERRUPTED, records an explicit
blocker, and interrupts unfinished tasks. AWAITING_APPROVAL and VERIFIED runs
remain inspectable and can enter their next explicit stage. Interrupted runs
never automatically resume. No stored PID is adopted/signalled. After abrupt
death, surviving processes may still write: inspect them before new work. A
crash can lose an in-memory result not yet checkpointed; no exactly-once external
effects, automatic repair, or crash-safe agent continuation is claimed.

## API additions

All routes use the existing bearer/Host/Origin/body-deadline protection:

- `POST /v1/engineering`: PlanSubmission (`id`, absolute `repository`, `base`,
  Markdown `specification`), returns accepted durable EngineeringRun.
- `GET /v1/engineering?limit=20&offset=0`: run documents (limit capped at 100).
- `GET /v1/engineering/health`: active run and workflow persistence error.
- `GET /v1/engineering/{id}`: complete run, tasks, attempts, evidence, blockers.
- `POST /v1/engineering/{id}/execute`: approve/start the stored plan.
- `POST /v1/engineering/{id}/integrate`: controlled local integration.
- `POST /v1/engineering/{id}/cancel`: asynchronous owned-work cancellation.
- `GET /v1/engineering/{id}/events?after=0&limit=500`: durable lifecycle history.

Existing process SSE/log routes serve agent and verification streams. Engineering
CLI follow polls durable history; it stops at inactive approval/verified/terminal
states. The Runtime v0 HTTP transport-backpressure debt remains unchanged.

## Validation and real-agent acceptance

Normal tests inject fake `AgentRuntime` adapters but use real Git worktrees,
SQLite, process supervision, verification commands, and both provider JSONL
envelopes. No login or inference is required. Tests cover plan validation,
dependencies/cycles, success/failure, mutation audits, automatic fixes, limits,
blockers, cancellation/timeouts, concurrency, integration failure, ownership,
durable events, forged evidence, and restart reconciliation, including a real
daemon kill/restart with an admitted engineering checkpoint and no provider login.
HTTP tests also saturate planning-body admission and verify cancellation remains
available.

The sample in `examples/config-validator` has working standard-library tests.
An explicitly opt-in test creates a disposable Git repository and uses a real
Claude planner, Codex coder, Claude reviewer, deterministic checks, and local
integration. It preserves the home, workspaces, logs and phase result JSON files
in the printed `autono-real-agent-<uuid>` directory even on failure:

```sh
AUTONO_REAL_AGENT_ACCEPTANCE=1 cargo test -p autono-cli --test real_agents -- --ignored --nocapture
```

This consumes inference and requires working installed CLI login/network access.
It reports failure/blockers honestly; installing the tools alone is not evidence
that an autonomous feature completed.

Independent review repair details and deterministic regressions: [engineering review repairs](engineering-review-repairs.md).
