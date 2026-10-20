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

## Validation of the first repair (d90aaa0)

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
The subsequent independent review verified those repairs, but reproduced the
remaining Git candidate/preservation defect described below.

## Final Git integrity repair

The P1 at d90aaa0 came from trusting Git's stat cache and inherited repository
settings during checkout/staging. `core.ignoreStat=true` could initialize index
entries with assume-unchanged flags before the coder started. Hashing those index
bytes only detected later mutations; it did not establish their safety.
`core.fileMode=false` could also omit executable changes. Consequently checks and
review could describe filesystem content different from the eventual commit.

The repair binds verification, review and preservation to one immutable tree:

- Every harness Git command explicitly sets `core.ignoreStat=false` and
  `core.fileMode=true`, including initialization and worktree-add's child checkout.
  Existing explicit ownership, hooks/fsmonitor/signing/driver restrictions and
  inherited environment/configuration isolation remain in place.
- Index inspection runs after checkout, before coder admission and staging, and
  throughout audits. Missing indexes, assume-unchanged, skip-worktree, exposed
  fsmonitor-valid and nonordinary index states fail closed. Raw HEAD/index hashes
  continue to detect agent control-state mutations; they are no longer the sole
  index integrity check.
- Before checks, the harness writes a candidate tree and compares its complete
  path/mode/blob manifest with the index. It independently hashes actual regular
  file bytes using `hash-object --no-filters` in bounded batches, compares symlink
  targets and executable bits, and rejects missing/unstaged files or unsupported
  transformations. Git stat-cache assertions have no authority over this audit.
- Check snapshots and repeated content/tree audits reject retained changes.
  Reviewer inventory, statistics and full diff come directly from the pinned
  candidate tree, with its identity included in the independent review payload.
  Reviewers still run in separate original-base checkouts with sealed artifacts.
- Preservation rechecks ownership, candidate bytes/modes and the verified snapshot,
  creates a commit using `commit-tree` with the exact reviewed tree and expected
  parent, rechecks the snapshot, then publishes only the owned branch using
  compare-and-swap `update-ref`. Post-publication checks verify branch/HEAD,
  actual candidate bytes/modes and the commit tree. It never asks porcelain
  `commit` to reread a mutable index as the source of the artifact.
- Additive/defaulted `verified_tree` and `preserved_tree` fields accompany the
  existing durable snapshot and artifact commit. SQLite rejects new completion
  transitions without matching tree evidence. Legacy completed records remain
  readable; their old evidence is not retroactively certified. Controlled local
  integration also audits task artifacts and combined bytes/modes against trees.

Four new deterministic regression tests cover tracked malicious AGENTS.md under
inherited ignoreStat, source edits, executable changes, combined settings, normal
repositories, preexisting assume-unchanged/skip-worktree flags (separately and
combined), and stale bytes/modes/missing/unexpected/staged/index changes before
preservation. Assertions inspect preserved blobs, tree IDs and `100644`/`100755`
modes, including integration commits, rather than trusting status output. The
existing forged-completion test also rejects missing/mismatched tree evidence.
Fixtures retain real Git, SQLite and process supervision with fake agents only.

The existing same-UID trust limit remains: this is an integrity audit, not an OS
sandbox against arbitrary hostile processes racing/reverting writes or modifying
shared Git objects. Immutable-tree commit creation removes the mutable-index
preservation gap within that boundary. Ignored artifacts remain outside the
candidate; Git records content and executable modes, not every filesystem
permission. Submodules, unsafe sparse/index states, incompatible clean/EOL transformations,
unrepresentable execute bits and non-UTF8 Git output fail closed. Unix mode
auditing is explicit. No workflow continuation or broader product scope is added.

Final local validation on macOS (2026-10-20):

- `cargo fmt --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test --workspace`: **74 passed**, **1 opt-in real-agent test ignored**.
- `cargo build --workspace`: passed.
- `cargo doc --workspace --no-deps`: passed.
- Complete repair diff inspected; `git diff --check` passed.

No real agents, additional agents, inference APIs or paid acceptance tests were
invoked. The remaining P1 is addressed with no finding deferred; renewed
independent review is still required. PR #2 remains Draft and is not merged.
