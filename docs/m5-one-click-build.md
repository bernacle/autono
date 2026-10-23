# M5 A1 — one-click Build

Dependency PR #7 was verified MERGED through GitHub before implementation:
`2026-10-23T12:59:52Z`, merge `514befbe940e7c446f567b77747f97e6f8e09d39`.
`feat/m5-one-click-build` starts at that fetched main in a separate T3 worktree.

## Inspected constraints and cohesive extension

The actual A2 `EngineeringService` already owns proposal persistence, atomic
authorization/run creation, idempotent submission UUIDs, tracked preparation,
planning, preflight, scheduling, bounded verification/review retries and optional
local integration. A3 supplies immutable-base evidence and append-only decisions;
A4 supplies essential-before-planning checks, task-scoped prerequisites,
independent-work continuation and the durable `RunHandoff`. Startup interrupts
lost execution; authorization replay never resumes it. These remain authoritative.

`workspace`, `git`, `scoped_files`, provider adapters, preflight, process admission,
SQLite evidence gates, CLI, native bridge, navigation, theme tokens and acceptance
fixtures were inspected. The missing mechanism was exact-file A2 scope being
unusable before implementation locations were known. This change adds one finite
source profile, candidate auditing and two entry surfaces to the existing
lifecycle. React never chains plan/execute/preflight/integrate requests.

## Contract and source protocol

The loopback API remains v1. `DelegationContract.version = 2` requires
`request.source_scope` with profile `version = 1`, `read_subtrees`,
`write_subtrees`, and `allow_create`. The legacy `writable_files` must be empty.
Absent source scope means contract v1 and its original exact-file semantics;
new fields are omitted from legacy serialization. Grants remain immutable.
Changing capabilities requires a new proposal UUID and fresh explicit consent.

The default desktop/CLI profile names `src` and `tests`. Advanced settings accept
subtrees, never an implicit repository root. API consumers can independently
bound reads and writes; writes must be inside a read subtree. Each list permits
1–8 canonical nonhidden relative subtrees. There is no wildcard/policy language.

Before consent, the existing trusted Git/configuration inspection pins canonical
repository identity, metadata, requested base commit, effective providers,
verification argv, budgets and owned worktree root. `ls-tree` provides a bounded,
deterministic **name-only** source inventory. No provider probe, paid inference,
dependency installation, repository script or discovery network request runs.
The consent view shows the authoritative repository, base, read/write resources,
agents/models/timeouts, exact checks, integration choice, runtime/attempt/review/
concurrency limits, missing spending cap and all exclusions/full JSON.

After consent, each fresh planner/coder/reviewer prompt gets the current tracked
ordinary files in its read subtrees, with names, SHA-256 and complete UTF-8
contents. The model selects implementation locations from that context; it has
no direct file tool. Staged files from prior attempts/dependency artifacts appear
on the next invocation without changing authority. New paths may be proposed
inside the write subtrees when creation is consented. There is no hidden exact
allowlist generated from the inventory. A3 separately reads cited immutable Git
objects at the original base, using read authority for evidence and write
authority for affected implementation paths.

The narrowly supported coder language remains complete `{path, content}`
replacements plus summary, blocker and decisions. Unknown command/permission/tool
fields fail closed. Every proposed path is checked by the daemon; a necessary
path outside the grant records its exact missing scope in a task hold. Independent
eligible work may continue. Neither decisions nor textual resolutions widen it.

## Filesystem and resource boundaries

- Canonical relative spellings only: no traversal, absolute paths, control
  characters, aliases, globs, hidden components or protected agent instructions.
- A fixed source-extension set permits common code, tests, styles and ordinary
  Markdown/text/CSV. JSON/YAML/TOML/lockfiles, credentials/private-key names,
  package/build configuration and CI/infra/deploy/scripts/generated/vendor
  directories are excluded. `source_scope.rs` contains the exact finite list;
  adding extensions/capabilities requires a profile change and new consent.
- Model reads use descriptor-relative `O_NOFOLLOW` traversal and bounded regular
  files with link count one. Nested `.git` markers are rejected at every traversed
  directory, including markers introduced after discovery. Git submodules are
  unsupported by the existing immutable candidate audit.
- Full workspace snapshots before inference, after inference and immediately
  before edit application reject observed concurrent changes. Existing untracked
  targets cannot be replaced. New files use atomic no-clobber publication;
  existing files publish a private inode, preserving mode without truncating a
  shared inode. Worktree leases serialize daemon writers. Same-UID hostile
  processes and trusted installed provider binaries remain the documented local
  trust boundary; this does not claim a kernel compare-and-swap for an existing
  filename against a malicious concurrent same-UID writer.
- A separate immutable candidate diff audit enforces the write scope, regular
  file modes, create policy, no deletion/rename/chmod and at most 128 changed
  files, independent of provider responses. It runs before verification and on
  the combined integration candidate. Existing verified/preserved-tree equality,
  post-check snapshot/mode audits and independent review remain required.
- Discovery examines at most 4,096 Git entries, reads at most 128 source files,
  each at most 256 KiB and 1 MiB aggregate. Edit batches/candidate deltas have the
  same file/payload bounds. Every complete tool-free prompt, including the full
  independent review diff and decision ledger, must fit 64 KiB. Oversize work
  blocks rather than silently truncating review or restoring tools.
- Known key/token markers in v2 source are rejected before entering prompts.
  Excluded filename/marker guards are not a universal secret detector. Developer
  source and requests must contain no credentials. Trusted Git/candidate audits
  still inspect repository bytes for integrity; excluded content is not supplied
  as model source context.

## Execution and actual provider capabilities

Consent persists before agent work. Existing revision-safe transitions advance
through planning, preflight, execution, verification, fresh review, isolated local
integration and handoff without new routine approval. The original absolute
deadline/cancellation token follows queued admission and final process spawn.
Identical consent replay returns the original run and lifetime. A disconnected
desktop cannot own or pause the workflow. Interrupted execution retains evidence
and is never automatically replayed.

Only the existing tool-free Claude CLI/API adapter is supported autonomously:
bare/restricted mode, no local filesystem/shell/MCP/subagent/session-resume tools,
hooks/settings/skills disabled, first-party API credentials configured externally.
Codex subscription login and general Claude/Codex CLI coding tools are **not**
provided by this profile. All roles must explicitly use the supported provider;
missing configuration/provider/auth prerequisites block before paid planning.
No paid inference is used by acceptance fixtures.

Configured developer verification argv is immutable. The daemon never accepts a
model-generated check or command. Seatbelt on macOS and Bubblewrap on Linux are
mandatory, with cleared check environments, system runtimes, assigned checkout,
per-command temporary storage and no network/unrelated user-home files. There is
no unsandboxed fallback. Inference access and check/write authority remain
separately constrained. Token costs and spending caps are unavailable.

No remote push, external mutation, publishing, deployment, dependency install,
automatic main merge, deletion, rename, permission change or credential access is
authorized. Source/read limits make this useful for bounded multi-file changes
in small local applications; it does not promise universal repository support.

## CLI, native bridge and persistence

```sh
autono build feature.md --repository /absolute/repo
# one full contract and one exact BUILD <UUID> terminal confirmation
autono build feature.md --repository /absolute/repo --source-tree lib --source-tree tests

# Deliberate noninteractive consent: inspect the immutable JSON before submitting.
autono build feature.md --repository /absolute/repo --plan-id <uuid> --proposal-only > contract.json
cat contract.json
autono authorize-delegation contract.json
autono show-delegation <uuid>
autono status --engineering <uuid> --json
autono handoff <uuid> --json
autono decisions <uuid> --json
autono logs --engineering <uuid>
```

`build` refuses piped/nonterminal consent. There is no implicit `--yes` bypass.
`--plan-id` safely retries ambiguous inspection; submitting the same consent file
never renews or replays a run. Existing Supervised commands are unchanged.

The native Build screen reuses Soft Neon dark/light surfaces and the OS repository
picker. Its advanced entry is explicitly named Supervised mode. The Rust bridge
adds two fixed typed routes: propose and authorize. It fetches the original
contract, compares its redacted projection with exactly what was displayed, then
sends the original contract to the daemon. JavaScript never receives credentials
or constructs a wider authority. Pending submission identity survives window
reload as a local cache, never as authorization. Daemon storage alone decides
whether consent already exists. The feature summary consumes A4 handoff and A3
records, shows actual verified counts/current task goals/recorded progress, exact
human requests and integration location, and links to existing detailed views.
No percentages, ETA or invented inference costs are added.

The existing SQLite immutable proposal/authorization documents persist v2 and
their replay identities atomically. No schema change or migration is necessary;
existing migration, immutability and restart suites continue to run. This avoids
reinterpreting old rows or adding another scheduler/state machine.

## Reproducible acceptance

```sh
AUTONO_BUILD_EVIDENCE="$PWD/docs/m5-build-acceptance.json" \
AUTONO_BUILD_DENIALS_EVIDENCE="$PWD/docs/m5-build-denials.json" \
RUST_TEST_THREADS=2 cargo test -p autono-cli --test build --locked -- --nocapture
cargo test -p autono-desktop --test build --locked
```

The primary fixture starts a real authenticated daemon with SQLite, production
tool-free provider adapter backed by a shell executable, real Git worktrees,
mandatory sandboxed checks and independent reviews. A small existing reporting
application receives the exact CSV feature request. Its grant contains no exact
files. Five tasks discover source, create implementation/tests/documentation,
consume a grounded A3 column-order choice, correct failed checks and rejected
review, integrate locally, and preserve main. Restart/duplicate consent proves no
inference replay. Additional real-daemon cases deny out-of-scope/credential/
configuration/traversal/command proposals and demonstrate Category B and deferrable
holds alongside independent verified work. Native bridge acceptance drops and
reopens the client while the daemon completes and checks exact-consent tampering.
Existing A2/A3/A4 suites exercise queued expiry/cancellation, configuration/base/
repository drift, unsafe effects, provider failure, evidence gates and migration.

Final validation receipts and actual packaged-app screenshots are recorded below
as validation completes. Pending CI is never represented as passed.

## Remaining milestones

A5: explicitly scoped, provenance-bearing Developer Memory. A6: targeted feedback
and revision runs, without rewriting historical decisions/evidence. A7: general
crash recovery with supported replay semantics. These are not implemented here.
Real-provider acceptance still requires explicitly authorized paid inference on
the supported Claude API profile. General CLI coding tools and subscription
providers require a separately confined, freshly consented profile.

Validation also exposed transient SQLite reader-pool startup failure on a busy
host: initial connection creation inherited the normal 250ms query-acquisition
budget. Read-only connection creation now uses the existing four-second,
known-transient-only persistence retry helper. The 250ms steady-state budget is
unchanged; no migration, write, agent or effect is replayed. A real SQLite test
forces the first connection handshake past that deadline and requires successful
startup with an empty engineering-run table. Fixture startup failures now retain
bounded daemon-log context so CI failures identify the cause.

## Native evidence and final authority checks

The [packaged native gallery and receipts](m5-build-native/README.md) cover the
requested dark/light size matrix, native repository selection, prompt, consent,
planning/execution, partial outcome, verified handoff, advanced graph and
Supervised entry. This is an actual packaged WKWebView connected to a disposable
real daemon; the documented accessibility-tool limitations are retained.

The final scope filter also excludes configuration directories, `*.config.*` /
`*.conf.*`, and reserved CI/build/deploy/release/publish/setup stems even when
those files have source extensions. Every delegated edit must appear in the
staged candidate before checks may run: `.gitignore` cannot make verification
observe a model-written source file omitted from the preserved tree. The eighth
real-daemon denial case retains that ignored file without any check/review
receipt or verified artifact, while independent work completes.

Native proposal dismissal distinguishes an unsubmitted proposal or definitive
409 conflict from an uncertain authorization response. The former can obtain a
fresh proposal after a changed base/configuration. A lost consent response keeps
its original submission identity across dismissal/navigation/reload; the daemon
remains the sole source of authorization. Essential preplanning holds use the
structured blocked outcome and expose the already-supported prerequisite refresh,
without offering routine execution approvals or crash replay.

Local validation and log hashes are retained in the
[validation manifest](m5-build-validation.json). Final committed-tree Linux/macOS,
MSRV and native packaging job URLs are kept in the Draft PR description and its
live checks. Both platform matrices passed for the initial implementation and
are rerun for the final scope/candidate guard corrections. The paid real-provider
scenario stays intentionally ignored.

Partial-clone rejection happens in trusted local Git configuration validation,
with an additional `GIT_NO_LAZY_FETCH` guard. No configuration key names or values
are collected through a logged discovery process: subsection names themselves can
contain credential-bearing URLs. The keyword check is deliberately conservative;
materialize partial clones externally. A regression verifies generic rejection
without echoing a credential-bearing subsection. The primary CLI acceptance also
uses relative feature/repository paths, resolved canonically before proposal.
