# M5 A3 — reversible autonomous decisions

## Inspected authority boundaries (before implementation)

Baseline: merged main `437acc4` (A2 PR #5). A3 does not alter the immutable
`DelegationContract`, provider profile, deadlines, admission checks or permission
broker. A decision is a recorded proposal and harness assessment, never a grant.

Concrete integration points inspected:

- `engine.rs::plan` / `ExecutionPlan::validate`: planner human requirements are
  frozen with the plan; `preflight.rs::inspect` turns every material requirement
  into a durable action. A3 adds separate structured decisions. It never removes
  an existing human requirement because a model called it reversible.
- `engine.rs::run_task`: coder summary/blocker currently has no decision history;
  `delegation.rs::apply_edits` validates the complete exact-file edit batch.
  A3 validates and checkpoints decisions before applying that batch.
- `agent.rs`: bounded Claude remains tool-free, fresh-session, bare/restricted,
  with the same 64 KiB prompt limit. Evidence reads use only exact allowed files
  via descriptor-relative scoped reads, or the original specification. Existing
  fixed pinned project context remains unchanged; no repository discovery.
- `engine.rs::fail_task`, `reconcile_before_retry`, service human resolution:
  permissions, missing prerequisites and uncertain external effects remain human
  gates. Failed checks and independent reviews remain authoritative; neither is
  an ambiguity to waive. Existing attempt/review budgets bound corrections.
- `journal.rs` / `store.rs`: revision serialization and SQLite transactions own
  workflow state. Decisions and audit events must commit with the run checkpoint.
  Recovery interrupts work without adopting PIDs or replaying providers.
- `review_task`: fresh reviewer, original specification, immutable plan, full diff,
  checks, criterion coverage and immutable tree equality remain required. Recorded
  decisions are untrusted review material, not instructions to accept the choice.
- CLI and authenticated daemon inspection expose run evidence. The native bridge
  uses typed fixed routes, bounded replies and existing redaction; adding an
  optional read-only ledger does not create a consent surface.

## Intended classification

A: only a local pattern, internal name, default value or test structure, supported
by exact available project/specification evidence, within the requested tasks and
exact A2 files. Unknown context or contradicting evidence prevents autonomous
acceptance. The implementation remains subject to checks and fresh review.

B: significant architecture/compatibility/product changes, insufficient or
conflicting evidence, unknown context or an explicit requirement conflict remain
unresolved. Task-scoped actions allow unrelated authorized tasks to progress.

C: authority, external operations, unknown outcomes and out-of-scope files go to
the existing human/permission workflow. This response language cannot encode an
operation or create a receipt. No textual classification expands authorization.
Missing prerequisites keep their existing distinct human-action classification.

Semantic interpretation is still fallible: exact evidence proves provenance,
not that a model's rationale follows logically. Structured conservative checks
and independent review constrain this risk; no confidence score claims authority.
A mislabeled preference still cannot execute outside A2's mechanical boundaries.

## Contract, persistence and recovery

`DecisionProposal` carries a stable ambiguity key, question, chosen option,
alternatives, concise rationale, kind, evidence claims, exact developer requirement
excerpts, assumptions, uncertainty, affected exact files/tasks and an optional
predecessor ID. It contains no commands, grants, confidence scores or private
chain of thought. The daemon validates field/array/payload bounds and credential
patterns before retaining the structured provider result. Missing `decisions`
means an empty array for old A2 provider responses; a present malformed array or
incomplete proposal fails closed. The new bounded provider schemas request the
field explicitly. Supervised schemas and approval behavior are unchanged.

A record's ID hashes `(run UUID, ambiguity key, predecessor ID)`. Identical replay
is a no-op; a changed payload with that identity is rejected. Rekeying the same
normalized question with overlapping task scope is rejected too. A new revision must
name the current predecessor and keep the affected task set. The journal lock
serializes concurrent proposals and allocates durable ordinals. The ledger has
at most 128 records, with at most eight proposals/32 KiB per provider response;
existing process, prompt, attempt, review and total runtime limits still apply.
There are no extra inference calls or decision retry loops.

The harness accepts only local pattern/name/default/test-structure kinds, with
exact-file authority, known original requirement excerpts, actual matching source
bytes, no contradictory support claims, and at least one supporting excerpt naming
the chosen option. Missing files, unavailable evidence, explicit uncertainty,
lexical requirement conflicts, existing material human requirements, or changes
involving another running/completed task prevent autonomous acceptance. Declared
significant choices remain B. External/unknown-effect proposals and file/evidence
references outside the allowlist remain C. C uses unsupported exact permission
requests in the existing broker; neither textual resolution nor an approval can
expand A2. Prerequisites retain their environment classification. No evidence path
outside the allowlist is probed or read.

`DecisionRecord` binds the proposal to the run, planner/coder session and optional
task/attempt. Verified provenance records the actual content SHA-256, excerpt,
option, original base and owned workspace ID. The proposal's evidence remains a
claim; only the separate provenance array contains byte-checked evidence. Hashes
identify supplied bytes, not a claim that those bytes prove the rationale.

Additive `0007_decisions.sql` adds `engineering_decisions` (immutable identity,
unique run/ID and run/ordinal) and `engineering_decision_events` (append-only
run/ID/sequence). Update/delete triggers protect both. The run JSON contains the
read projection, defaulting to empty on legacy documents. Prefix/identity checks
prevent rewrites or removal. Projection inserts, new events, the run revision and
lifecycle event share the existing SQLite transaction. Existing migrations,
permission receipts and completed attempt evidence are unchanged.

Planner decisions commit with the immutable plan. Coder proposals are assessed
and committed before any proposed file replacement. The complete edit batch then
passes the unchanged A2 decoder/path validator. A later edit rejection or crash
can leave an accepted decision **unimplemented/unverified**; it cannot leave an
unrecorded authoritative choice. Candidate tree, failed/passed checks, independent
review rejection/acceptance, preserved task artifact, supersession and human
resolution-receipt references append separate facts. Review findings stay in the
original immutable attempt evidence; audit events point to those task/attempts.
`verified()` requires preserved, independently accepted task evidence for **every**
affected task. This is task-scoped verification; the run and combined integration
status remain authoritative for the overall result. Passing tests alone is insufficient. Startup interruption retains
all committed facts and does not resume/replay an agent or external effect.

Human feedback references the existing immutable resolution receipt and never
rewrites a decision. An unresolved proposal remains unresolved even if the user
answers its human action; the receipt is visible separately. Automatically
reconciling/revising that proposal from feedback is left to A6.

## Workflow and inspection

The bounded planner and coder receive the structured contract, original
specification, exact allowed source data and existing current-run ledger. A coder
repeats a stable proposal or supplies an explicitly linked revision after
feedback. Decisions can hold affected tasks while the existing DAG scheduler
continues independent work. Existing planner human requirements are never erased
or automatically recategorized. A generic coder blocker is also never guessed to
be a reversible choice.

The fresh reviewer receives the ledger as untrusted data alongside the original
specification, immutable plan, configured check receipts, original trusted base
and full candidate diff. It has no authority to bypass checks or accept a
self-certified outcome. The coder/reviewer remain fresh, separate processes,
worktrees and sessions. The original tree-preservation gates remain intact.

Read-only inspection:

```sh
autono decisions <run-id>          # readable choices, evidence, state and audit
autono decisions <run-id> --json   # complete structured ledger
# Same existing loopback authentication/Host/Origin rules:
# GET /v1/engineering/<run-id>/decisions
```

The existing run response also includes the ledger. The native bridge transports
that field through its existing fixed Run route and redaction. Mission Control's
Verification tab adds a compact “Decisions Autono made while you were away”
section: alternatives, rationale, provenance, assumptions, uncertainty and event
history. No permission or revision UI is added.

## Deterministic acceptance and validation

`crates/autono-cli/tests/decisions.rs` uses the real CLI, detached daemon,
production tool-free Claude adapter, SQLite, Git worktrees, sandboxed checks and
fresh reviews. Its provider executable is a shell fixture; no installed provider,
login flow or paid inference is used. The seeded authorized `format.sh` says
“Default export format is CSV.” The request is exactly “Implement report export
with validation and tests.” The five-task DAG is:

```text
format ----+                 +--- tests
           +--- export -----+
validate --+                 +--- documentation
```

The planner records CSV versus JSON with the source digest; the coder implements
CSV output plus argument validation and meaningful success/failure tests. Both
per-task and combined checks run. Fresh reviewer fixtures assert they receive
the original request and decision evidence. The fixture asserts no human question
or routine approval/continuation/integration action, all tasks independently
verified, exact delegation unchanged, authorized isolated integration, main
unchanged, API/CLI equality, restart with zero replay, and interruption after the
ledger commit with retained unverified records. To retain real evidence:

```sh
AUTONO_DECISIONS_EVIDENCE="$PWD/docs/m5-decisions-acceptance.json" \
  cargo test -p autono-cli --test decisions --locked -- --nocapture
```

Core shell-backed tests cover missing context, explicit requirements overruling
preferences, unlisted files, external/unknown effects, architectural choices,
uncertainty, fabricated/contradictory evidence, harmless retry, repeated proposals,
reviewer rejection and linked correction, malformed/secret content, and executable
operation smuggling. Permission tests exercise denied textual and explicit
approval attempts. SQLite tests cover transaction rollback, concurrent duplicate
proposals, immutable rows/events, forged verification, reopen and migration from
an existing Supervised document. Frontend tests require every affected task's
verified artifact and preserve unresolved/superseded presentation; authenticated
bridge tests retain decisions and review rejections through redaction.

## Limits and next phases

- Exact excerpts and a lexical conflict gate cannot prove semantic correctness or
  discover omitted constraints. They deliberately reject some safe choices.
  Independent review and mechanical A2 enforcement remain essential. A malicious
  misclassification still cannot launch an operation outside the response language.
- No arbitrary repository discovery or read grant is added. Evidence is limited
  to original requirements and the exact consented files. Existing fixed pinned
  project context still informs agents/review, but is not a new evidence-reading
  capability. Missing context stays explicit. Full ledgers remain subject to the
  existing 64 KiB complete-prompt limit and block when too large.
- Bounded correction covers the current unfinished task. A choice already consumed
  by another running or completed affected task is held for human review/new
  planning; this PR does not reopen immutable completed evidence or rewind a DAG.
  A6 should support targeted revision runs while preserving those artifacts.
- The model must identify genuine choices and reuse their keys. This is not a
  semantic deduplication engine and does not reconstruct every implicit choice.
- Secrets are rejected by bounded credential-pattern input guards, not by a
  universal secret detector. Existing process-output storage/redaction rules are
  unchanged. Do not put credentials in specifications, source or model responses.
- Checkpoints establish at-least-once audit identities, not exactly-once process
  execution. A crash between acceptance and candidate recording retains an
  unverified decision. A decision's verification says its affected implementation
  passed configured checks and independent review, not that it is globally optimal.
- A4 should strengthen unattended outcome reporting and evidence completeness on
  this ledger. A5 can supply a separate provenance-bearing context interface with
  explicit access authority; no cross-run memory is implemented here. A6 should
  append targeted feedback/revision requests and revisit unresolved decisions
  without rewriting historical receipts. Dynamic discovery needs separately
  consented read/write scopes and a new A2 profile, never inferred grants from
  evidence references.

A small A2 prerequisite is included: `ProcessRunner` now applies the existing
four-second, transient-lock/pool-timeout-only persistence retry helper to process
**evidence reads**. Ledger checkpoints and polling can briefly occupy the bounded
reader pool. These retries never resubmit a process or repeat an effect; process
cancellation and deadline supervision remain independent. A deterministic test
holds all four reader leases past the pool timeout, releases them, then requires
one successful process receipt with complete output and an uncancelled workflow.
Persistent read failure still fails closed.

Empty decision fields are omitted from serialized plans. Planner proposals are
removed from the frozen plan and committed into the dedicated ledger; the
original planner response retains them. This preserves the old plan wire shape
(including old clients that reject unknown plan fields) while keeping decisions
independently revisionable. Empty legacy/Supervised ledgers remain compatible.

CI bounds concurrent Rust fixture runtimes to two (`RUST_TEST_THREADS=2`). All
scenario-internal worker races, deadline, cancellation, sandbox and reader-pool
contention assertions still run. The CLI fixture bounds retries of a slow/lost
authorization HTTP receipt to three identical-contract submissions; it asserts
one planner invocation and unchanged authorization/deadline on restart replay.
No provider process, decision attempt or external effect is replayed by that
client retry.

## Recorded delivery validation

On macOS arm64 the workspace suite passed **132 Rust tests**, with two intentionally ignored
paid-agent/subprocess-helper cases; all **103 frontend tests** passed. Final targeted
ledger/unit, adversarial correction and real-daemon CSV runs also passed. Checks
passed for TypeScript, zero-warning lint, formatting, strict all-target Clippy,
debug and release workspace builds, Rust 1.89 non-desktop targets, rustdoc,
production frontend startup, native Tauri packaging and authenticated native
inspection of the completed CSV run. Owned smoke processes were stopped.

The retained run is `b6557895-0b10-49bf-a751-fae58921dbb3`: 212 lifecycle events,
17 decision events, five completed tasks, no human actions, and isolated local
integration `c4b680535950de9994adc92c597e47bf28356f87`, with main unchanged.
See [actual run/journal](m5-decisions-acceptance.json) and
[validation manifest](m5-decisions-validation.json). Linux and remote macOS CI
results remain pending at submission; the matrix now includes release builds.
