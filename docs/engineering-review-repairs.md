# Engineering Loop v0 independent review repairs

The review of 3279e0e was NOT READY FOR MERGE. This repair addresses its P1 and
P2 findings without adding a new workflow, approval broker, provider, or service.
A fresh independent review is still required; deterministic tests are not that
review. No real Codex/Claude calls were made for these repairs.

## Trust boundaries

- Context is read from regular Git blobs at the original pinned base. Each fresh
  reviewer runs in its own base checkout, with candidate changes supplied through
  an explicitly untrusted diff/evidence payload. Candidate instruction/configuration
  changes block before checks/review and require human approval into a new trusted
  base. README/architecture edits cannot replace authoritative context. Provider
  read-only restrictions and mutation audits remain defense in depth, not an OS
  sandbox or proof of LLM prompt-injection immunity.
- Git ownership/configuration is captured before agents run and persisted in each
  workspace. Explicit Git directory/worktree flags replace ambient discovery;
  pointer/back-reference/commondir/config/attributes/exclude/alternate metadata is
  checked before commands. Hooks/fsmonitor/external diff/textconv and signing are
  disabled; system/global config and inherited redirect variables are neutralized.
  Configuration includes/driver sections and object alternates/grafts fail closed.
  HEAD/index cannot be symlinks. Raw HEAD/index hashes reject coder control-state
  changes (including assume-unchanged flags) and participate in mutation audits.
  Optional Git index refresh locks are disabled for workflow commands. macOS/Linux use standard `env -u` for isolation.
- Session artifacts use separate UUID directories, create-new files, read-only
  permissions, and content/permission audits before and after calls. Sealing compares against
  harness-authored bytes, rejecting pre-seal tampering instead of trusting disk
  contents as a new baseline. Prompt/diff paths use JSON escaping. Providers
  receive only their own additional artifact directory. Large prompts use the
  same sealed session-local files. A provider with arbitrary same-UID command
  access can defeat filesystem permissions or race/revert writes; these controls
  detect retained mutations and constrain supported provider tools, not hostile
  programs running with the user's full authority.

## Reliability and inference

- Every reviewer invocation has durable session/workspace/result/diagnostic
  history. Invalid JSON, invalid criterion coverage and retryable provider/runtime
  failures retry fresh reviewers against the same checked candidate, consuming
  only the explicit review budget. Rejected valid reviews can request coder fixes;
  invalid/infrastructure responses cannot spend a coding attempt.
- Stable task/criterion ordinal IDs are schema-enforced and coverage-checked;
  labels can be paraphrased. Existing exact-text reviews remain readable as legacy
  evidence, but new responses must carry IDs.
- Known provider envelope codes and explicit permission-denial fields classify
  credential/permission blockers. Transcript and stderr keywords have no authority.
  Runtime cancellation and timeout precede JSON decoding/classification. Unknown
  error formats remain provider failures rather than invented human blockers.
- Process admission uses six cancellable backoff retries (3.15 seconds total),
  with one idempotency key. Planning/execution validate task parallelism against
  configured runtime capacity. Other raw process runs can still occupy slots;
  exhaustion stops explicitly rather than claiming a coding defect.
- Verification records distinguish passed/tests-failed/timeout/cancelled/
  infrastructure outcomes. Timeout/spawn/output failures block without coder fixes.
- Every review gets full changed-file statuses/inventory, statistics, explicit diff
  truncation state, digest, full session-local diff reference, and scrutiny flags
  for build/test/verification/configuration changes. Oversized capture limits stop
  explicitly; the harness never silently reviews an incomplete full artifact.
- Task prompts include referenced requirements, not the full feature repeatedly.
  Resumed coder prompts contain only new feedback and stable task/context digests;
  reviewers retain independent trusted context. The original specification remains
  durable and inspectable. Plan semantic completeness still needs human approval.

## Deterministic regression evidence

The fake-agent suite uses real Git, SQLite and Runtime v0 supervision, including:

- malicious AGENTS.md forcing acceptance; modified README kept out of authority;
- coder/reviewer .git-pointer attacks; persisted config ownership checks; disabled
  fsmonitor/hooks; configuration includes and executable filter rejection;
- malformed and exhausted reviews; misleading error text; genuine structured auth;
- reviewer process failure/spawn retry without coder reexecution; stable criterion
  IDs with paraphrased labels and rejection of forged IDs;
- verification timeout/spawn failures; bounded/cancellable/idempotent process
  admission; parallelism exceeding process capacity;
- cross-session and pre-seal artifact tampering; sealed large prompt-file delivery;
- complete inventory/statistics/full artifact after diff truncation;
- shorter resumed feedback while preserving review independence;
- existing cancellation, dependency/concurrency, verification mutation, integration,
  forged evidence, durable event and daemon restart regressions.

Resource-heavy test fixtures are bounded independently of workflow parallelism;
concurrent task regressions still require overlapping independent tasks.

## Intentional limits

No monetary/token budget, automatic human approval for instruction changes,
replanning, intelligent infrastructure repair, automatic workflow continuation,
strong same-UID isolation, or Windows hardening claim is introduced. Detached
processes, uncertain cleanup, ignored artifacts, transient/reverted changes,
unaudited shared Git objects/refs, and Runtime v0 transport backpressure remain
explicit limitations. Previously persisted plans without pinned Git ownership
remain inspectable but must be resubmitted before execution. New fields are
additive/defaulted; no original migration is rewritten.

## Final local validation

On macOS, the final repaired tree passed:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`: **70 passed**, **1 real-agent test ignored**.
- `cargo build --workspace`
- `cargo doc --workspace --no-deps`
- `git diff --check` and complete repair diff inspection.

No real Codex/Claude agents or inference APIs were invoked. The prior real-agent
sample acceptance predates these repairs and is not independent review of Autono.
Linux/MSRV results belong to GitHub Actions, not this local macOS evidence.
All listed review findings are addressed; none are deferred. The explicit trust
and operational limitations above remain and require renewed independent review.
