# M5 A4 — intelligent prerequisites and truthful partial handoff

Base inspected: main `ad698662c552efa9c70fdcded2f41a19231f49e4`, merged A3 PR #6.

## Architecture findings and smallest change

The existing `EngineeringRun.human_actions` is already the durable blocker model:
stable IDs, exact permission requests, task scopes, immutable requirements,
append-only audit, workspace snapshots and idempotent resolution receipts.
`blockers` is historical diagnostic evidence, not a second queue. A3 decisions
already have their own provenance and verification ledger. A4 extends these
entities and the serialized journal; it does not introduce another scheduler.

`ExecutionPlan::validate` checks coverage, bounded task count and acyclic known
edges. `ready_tasks` requires every direct dependency COMPLETED. The scheduler
already continues independent workers, leaves human-paused descendants WAITING,
propagates terminal dependency failures, and retains verified task commits.
A2's `autonomous_preflight_ready` already admits scoped planner/decision actions
while all deterministic host gates and run-wide actions still gate the run.
Consequently preflight's run-wide NOT READY does **not** itself stop independent
A2 work. Supervised approval intentionally continues to require full READY.
Unscoped configuration prerequisites cannot safely be guessed to be task-local.

The actual gap is that preflight ran after paid planning, and neither the durable
run nor the CLI exposed a structured partial-completion receipt. Existing active
continuation checked the deadline but did not refresh full delegation context and
preflight until the worker's admission. The narrow changes are a preplanning
check, classification metadata on actions, an atomic evidence projection, and
revalidation of active continuation. Existing verified-tree/preserved-tree,
checks, fresh independent review and combined local integration remain required.

## Classification and interruption policy

- **Essential before start:** deterministic host/environment/provider/check gates,
  and requirements with no safe task scope. Check before planner invocation.
  The bare A2 provider needs nonempty ANTHROPIC_API_KEY presence; this is a
  necessary check, not authentication validity. No credential file or value is
  retained. Supervised authentication remains UNKNOWN when no safe check exists.
- **Deferrable manual dependency:** an exact task-scoped environment/product
  requirement, or a task-local hold returned by the tool-free A2 response.
  Only those tasks and actual DAG descendants wait. Scope is never inferred from
  a prose assertion. Running independent agents are not cancelled.
- **Safely resolvable internally:** use A3 accepted reversible decisions and their
  existing evidence, file scope, consumption and independent-review ledger. No
  human action or permission grant is created to represent this category.
- **Unsafe or uncertain:** permission requests, unbounded runtime outcomes,
  integrity failures and unknown effects retain the existing broker and live
  reconciliation gates. Deferral never authorizes an operation. A textual answer
  cannot bypass DENY, adapter restrictions, unknown effects, checks or review.

Identical frozen planner requirements are grouped only when all fields other than
task membership match exactly. Their exact scopes are unioned; an unscoped
requirement remains run-wide. Distinct permission scopes are never combined.
Runtime question identity binds the task and attempt checkpoint. A refresh reuses
current action state rather than asking again. A safe alternative is not inferred;
any local mock/default must pass A3 and the original immutable test contract.

## Durable state and handoff

`HumanAction.prerequisite` holds category, daemon classification basis and required
verification. Identity, scope, permission, status, timestamps, resolutions and
history stay on the existing Human Action. Descendant impact is deterministically
projected from the frozen DAG. Migration 0008 adds only append-only action audit
projection rows, written in the same SQLite transaction as run state and events.
Legacy records deserialize without classification or handoff.

Each journal checkpoint derives `RunHandoff` from actual task artifacts, candidate
implementation trees, configured check receipts and fresh review. It separately
reports locally independently verified, implemented without full verification,
waiting prerequisites, not implemented and failed work. The summary includes
unresolved exact action/version references and real dependent descendants.
Partial completion is a projection while the run remains BLOCKED, never VERIFIED
or COMPLETED. Complete feature states reject unresolved required actions. Combined
checks are separately reported; local or mock success never claims real-service
success. This API cannot add a network capability to the A2 sandbox.

Read `GET /v1/engineering/<id>/handoff`, `autono handoff <id> [--json]`, or the
existing full summary. The desktop adds a compact partial handoff panel using the
existing typed/redacted Run bridge. Credentials stay outside JavaScript.

## Resolution and recovery

New clients send `expected_action_version` equal to the observed audit length.
The daemon checks it under the journal lock before accepting a new receipt;
legacy clients retain status/identity guards. Identical request UUID/payload
replays return the original receipt before version checks. Receipts do not launch
processes themselves. A2 continuation separately rechecks the immutable grant,
repository/configuration, original deadline, budget, preflight and task workspace.
Active resolution refreshes these gates while healthy workers remain owned.
Supervised mode retains explicit continuation. Unknown effects, terminal runs and
interrupted runs cannot auto-replay. Restart retains evidence and interrupts
active ownership; no PID adoption or generalized crash resume is implemented.

An essential preplanning hold has no planner session or task plan. Repair and
refresh preflight can retry that never-invoked planning admission under the
original authorization. A pinned existing planner worktree must still have its
original ownership, branch, HEAD and clean state. This path cannot replay an
already-started planner.

## Follow-ups

A1 can consume the distinction between full readiness and independent-work
readiness without inventing authority. A5 may supply separately authorized
context; no Developer Memory or dynamic discovery is included. A6 can consume the
structured handoff and receipts for final review/revision UX. Real external
integration remains outside A2's network-free tool-free v1 profile: external
verification requires an explicitly supported authoritative contract, not a fake
successful service call in an acceptance fixture.

## Independent review and corrections

Fresh independent correctness and security reviews inspected the implementation
and re-reviewed corrections. They found and verified fixes for:

- legacy action hashes changing when requirements were normalized/grouped;
  legacy records now retain exact original grouping and order;
- persistent failed essential checks causing an automatic preflight retry loop;
  only a scheduler-drain race is retried, and a repaired inactive preflight can
  re-admit resolved tasks under the original delegation;
- a passing prefix of configured checks being reported as a passed suite;
  reporting now requires the complete exact ordered command contract;
- earlier retained candidate work disappearing from the latest handoff after an
  empty blocker response; safe pause checkpoints now retain its candidate tree;
- recovered preplanning work staying AWAITING_APPROVAL while its planner ran;
  narrowly guarded admission commits PLANNING before invocation, preserving
  failure, cancellation and restart ownership semantics;
- Git write-tree updating index cache bytes after a continuation snapshot;
  candidate inspection now precedes the final durable pause snapshot.

Both reviews ended with no unresolved correctness/security objections. These
were static reviews, supplemented by the actual daemon and persistence tests;
they do not claim independent real-service integration or universal crash resume.

The six new disposable-daemon tests cover the five-task planned/runtime scenarios,
shared prerequisites, multiple blockers, actual dependent waiting, a failed local
test branch, expiry, late cancellation, stale/duplicate resolutions, retained
candidates, and repaired planner failure/restart. Existing A2/A3 suites retain
sandbox, explicit prohibition, unknown-effect reconciliation, reviewer rejection,
bounded retries, supersession, provenance and deadline/cancellation regressions.

The complete A2 suite additionally reproduced a denial-recording regression:
a blanket prerequisite guard rejected the denial checkpoint during INTEGRATING,
masking the original blocked failure as FAILED. The corrected guard applies to
entry into VERIFIED/INTEGRATING and every COMPLETED checkpoint. In-flight denial
evidence can commit before existing failure handling records BLOCKED. Existing
VERIFIED local artifacts are not rewritten or recertified by a new finding; the
handoff reports incomplete evidence and blocks further integration admission.
Both independent reviewers accepted this correction. The original A2 denied
integration regression remains unchanged and passes.

The native bridge now uses the explicit action audit version for new resolution
requests, allowing unrelated worker progress without asking the developer to
inspect the same unchanged action again. Legacy requests retain their run-revision
guard. The daemon remains the atomic authority for version, status, permission,
run eligibility and original delegation checks.

Native packaging and inspection used the existing disposable injected-provider
example, separate from the A2 five-task fixture. The actual packaged WKWebView
showed three verified local tasks, B's exact unresolved checkpoint and D waiting
on B. After quitting the desktop, the daemon accepted the explicit Supervised
resolution, waited for explicit Continue, finished the remaining fresh reviews,
and passed explicitly requested combined local integration checks. All three
previously completed task artifacts remained unchanged. See the retained
[native smoke receipt](m5-prerequisites-native-smoke.json).

## Validation

Local validation on macOS arm64 passed 151 Rust workspace tests and 109 frontend
tests. The two ignored Rust entries are the explicitly opt-in paid real-agent
acceptance test and an isolated helper exercised by its parent preflight test.
No paid inference was used. This includes six new disposable-daemon scenarios,
SQLite migration/persistence tests, protocol/CLI/native bridge checks and the
previous A2/A3 authorization, sandbox, decision-provenance, supersession,
deadline/cancellation and reviewer-retry regressions.

Formatting, strict Clippy, TypeScript, lint, production frontend smoke, debug and
release workspace builds, rustdoc, Rust 1.89 core/daemon/protocol/CLI checks and
Rust 1.90 desktop checks passed. The native app was packaged and inspected, with
the daemon-close/explicit-Supervised-continuation receipt above. Exact commands,
log hashes and platform details are in the [validation receipt](m5-prerequisites-validation.json).
The existing Linux/macOS, MSRV and native GitHub Actions run on push/PR; their
live status must be read from the PR rather than inferred from local results.
