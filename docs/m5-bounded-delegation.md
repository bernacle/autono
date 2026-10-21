# M5 A2 — bounded per-run delegation

This change starts from M4 main `d4a1a95`. The daemon can complete a small,
explicitly scoped feature without routine plan, execute, continue or integrate
approvals. The existing Supervised path remains available. This is an authority
slice with a CLI consent surface; the desktop visual system is unchanged.

## Architecture inspected

The implementation, rather than the old specifications, establishes these
boundaries:

- `EngineeringService` serializes inactive admission with its control gate and
  keeps one owned engineering workflow per daemon. Its task tracker owns
  preparation, acceptance, execution and accepted human resolutions independently
  of HTTP/CLI/window lifetime. Active workers share one revisioned `Journal`.
- `Engine::plan` creates a retained original-base worktree, invokes a read-only
  planner, validates requirements/DAG/criteria, freezes the plan, and runs
  preflight. Supervised planning ends at `AWAITING_APPROVAL`.
- Preflight checks host/Git/provider capabilities and produces durable human
  actions. Policy defaults and previous resolutions do not grant new operation
  scopes. Existing exact broker approval paths support only assigned worktree
  writes and the immutable configured check set.
- Supervised `execute` refreshes preflight and explicitly records approval before
  scheduling. Tasks execute in dependency order with bounded parallelism. Human
  pauses retain independent workers and leave descendants waiting. Supervised
  resolution and continuation remain distinct operations.
- Each candidate must pass configured checks, complete-output/process evidence,
  independent fresh review with exact criterion coverage, content/mode audits,
  and equality between verified and preserved immutable trees. Rejection/failure
  retries retain attempt/review budgets. Unknown effects require reconciliation.
- `VERIFIED` means all tasks meet those requirements. Supervised integration is
  separately admitted, creates an isolated integration worktree/branch, merges
  task artifacts in deterministic order and runs combined checks. It never merges
  into the developer's checked-out branch or pushes.
- SQLite stores revisioned run documents, immutable inputs/completed evidence,
  workspace reservations, append-only lifecycle events, preflight/action
  projections and immutable resolution receipts atomically. Startup reconciles
  active stages to `INTERRUPTED`; saved PIDs are never adopted or blindly replayed.
- Loopback authentication, Host/Origin checks, body/deadline/admission limits and
  process submission keys are existing authority boundaries. The native Rust
  bridge retains token ownership, fixed routes, redaction and stale-view checks;
  JavaScript never receives raw provider environments or daemon credentials.

Relevant prior evidence: [M4 acceptance](desktop-acceptance.md),
[M4 independent-review repairs](desktop-review-fixes.md),
[M3 preflight/effects](preflight.md), and
[immutable-tree review repairs](engineering-review-repairs.md).

## Contract and consent

`POST /v1/delegations/propose` takes a `DelegationProposalRequest`. It uses the
same repository/configuration machinery as supervised planning but invokes no
agent. It returns and durably stores the full immutable `DelegationContract`.
Read it through `GET /v1/delegations/{run-id}`. Proposal creation runs trusted,
read-only Git probes; it is not authorization to plan or code.

The contract includes the exact specification/run UUID; canonical repository;
Git worktree/common-directory metadata hashes and device/inode/owner identities;
requested base and resolved commit; the daemon-owned run worktree root; exact
writable relative files; all effective role/provider/model/timeout settings;
ordered verification argv/timeouts; parallelism/attempt/review-cycle bounds;
permission policy; a user-config-file digest; sandbox version; total runtime;
optional isolated local integration; and readable limitations.

`POST /v1/delegations/{run-id}/authorize` requires that exact stored contract,
including its displayed limitations. Editing any field is a conflict. The daemon
rebuilds the contract from current repository/configuration state before accepting
consent. Consent and initial run creation commit atomically. `authorized_at` and
`expires_at` are immutable; the deadline includes planning, retries, checks,
reviews, integration and idle time. Repeated consent never resets it.

The run UUID is the proposal/submission/idempotency key. Matching replay returns
the existing run/current view and its original authorization, including after
completion, cancellation or interruption. It never restarts a planner or stage.
Concurrent first requests can encounter the existing bounded-admission overload
response; retry the identical contract. A UUID cannot select a different proposal,
specification or supervised workflow. This does not claim exactly-once execution
of an arbitrary process or external effect.

### Functional CLI

Configure all three roles to use Claude with API credentials configured through its
existing environment mechanism (never a consent payload), and choose checks available inside the
system-runtime sandbox. An absolute or relative executable in the assigned
checkout is also possible. User-home toolchains, package downloads and dependency
installation are outside this first sandbox profile. Preflight reports missing
executables and proves the actual sandbox with an inference-free `/usr/bin/true`
probe before coding starts.

```sh
# Existing explicit Supervised workflow:
autono plan feature.md --repository /absolute/repository --base main

# Autonomous proposal: inspect every field before consenting.
autono propose-delegation feature.md \
  --repository /absolute/repository --base main \
  --plan-id <new-uuid> \
  --allow-file src/feature.py --allow-file tests/test_feature.py \
  --max-runtime-seconds 1800 --local-integration > delegation.json
cat delegation.json

# This is the single initial consent action; the submitting client may then exit.
autono authorize-delegation delegation.json

autono show-delegation <run-id>
autono summary <run-id>     # includes original recorded authorization and deadline
autono blockers <run-id>
autono logs --engineering <run-id>
```

Omitting `--local-integration` authorizes work only through `VERIFIED`. Even an
explicit call to the old integrate endpoint cannot expand that run's delegation.
Changing scope requires a new proposal and consent. The desktop remains a
Supervised creation surface; the CLI is the autonomous consent/inspection surface
for A2. Closing the desktop has no relationship to either workflow's ownership.

## Exact unattended authority

The daemon may perform only these engineering operations under the contract:

1. Create its run-owned planner, task, reviewer and optional integration worktrees
   from the pinned base; read pinned project context and authorized regular files.
2. Invoke the configured agents in fresh tool-free sessions. Initial autonomous
   support is Claude only: bare/restricted mode, no filesystem/shell tools,
   disabled skills/commands/hooks/settings sources, denied MCP tools, strict MCP
   configuration, noninteractive permission denial and no provider resume. The
   trusted provider binary uses configured API credentials in bare mode for
   inference; authentication files are never read by Autono. Bare mode skips
   customization discovery and does not use OAuth subscription login/keychain.
   The v1 profile rejects alternate cloud/custom endpoint/config-directory
   environment overrides by name without exposing their values. Model arguments are fixed; provider defaults,
   routing, actual billing and hidden vendor accounting cannot be certified.
3. Accept coder JSON containing only `summary`, `blocker` and exact `edits` of
   `{path, content}`. The daemon validates the entire batch before writes. It may
   create/replace the explicitly listed regular UTF-8 files in that task's
   worktree. Paths cannot be absolute, aliased, hidden, globs, symlinked,
   hard-linked, or instruction/configuration files. No delete, chmod, rename,
   command execution or inferred permission is encoded in this response language.
   Reads traverse retained directory descriptors with `O_NOFOLLOW`. Replacements
   publish a new private inode with descriptor-relative rename, so a racing link
   from a sandboxed check cannot make the daemon read/write outside its checkout.
4. Run only the exact configured check argv, plus deterministic readiness probes.
   macOS uses Seatbelt; Linux uses Bubblewrap with isolated namespaces. There is
   no unsandboxed fallback. Check environments are cleared; the system PATH is
   fixed; source checkout and command-specific temporary storage are writable;
   user-home file contents, unrelated writes and networking are unavailable.
   The profile grants system-runtime reads and directory traversal/metadata
   needed to launch tools. It does not authorize arbitrary agent-generated shell.
5. Merge accepted dependency artifacts, run bounded retries, verify and review,
   preserve matching immutable trees on owned task branches, and optionally
   integrate into `autono/<run-id>/integration` with combined verification.

Files are limited to 128 exact names, 256 KiB each and 1 MiB total source/edit
payloads. Complete tool-free prompts, including the full independent-review diff,
are limited to 64 KiB. Oversized work blocks instead of silently truncating review
or re-enabling file tools. Existing stricter artifact/capture/plan limits apply.

Policy `DENY` wins. Initial explicit consent can satisfy `REQUIRE_APPROVAL` for
only the contract's baseline workspace-write/check scopes. A planner proposal,
policy `ALLOW`, learned preference, or human-action resolution cannot extend the
file list, commands, model configuration, integration permission or deadline.

## Execution-time admission and interruptions

`Engine::admit` checks the effective contract before planning/worktree admission,
agent invocation and process launch, file application, verification launch,
review-worktree creation, dependency merges, artifact preservation and integration.
It compares repository identity/ownership, pinned base, current effective project
settings, user configuration digest, root/sandbox identity, current process
capacity, policy and expiration. The original authorization remains immutable.
The stage APIs perform the same validation before preflight or state mutation.
Admitted/denied decisions have durable journal events. Denials produce explicit
human reasons and retain work; task-local denials do not cancel healthy unrelated
workers.

Internal progression reuses consent through `AWAITING_APPROVAL → RUNNING →
VERIFIED → INTEGRATING → COMPLETED`, with every original readiness, revision,
verification, process-receipt, criterion, independent-review and immutable-tree
gate intact. SQLite independently verifies the full launched sandbox wrapper,
check argv, cwd, timeout and process key against authoritative process evidence.
It does not accept an unsandboxed process as a delegated check receipt.
Autonomous reviewers also receive the original developer specification and full
immutable plan, so task criteria cannot silently replace original intent.

Readiness blockers remain inspectable. Every deterministic host check and
run-wide human gate must pass. Task-scoped planning actions hold only affected
tasks and descendants; independent authorized tasks may complete even while the
whole-plan preflight report remains NOT READY. SQLite permits that partial
admission only with a recorded delegation and successful deterministic host gates.
Supervised admission continues to require the complete ready report.
Repairing a prerequisite and refreshing
preflight reuses the original consent. An exact human resolution can schedule
eligible autonomous continuation internally; Supervised resolution still cannot
launch a worker. Its immutable resolution response remains the original receipt,
so inspect the current summary for subsequent liveness. The original operation,
files and all remaining budgets are rechecked. Unknown effects remain blocked;
failed work with previous effect context still requires fresh reconciliation.

A deadline watcher cancels owned active processes and prevents further admission.
Cancellation never waits for a SQLite checkpoint to signal processes. All budgets
and completed artifacts survive. Interrupted/cancelled/failed runs are not resumed
by authorization replay. Startup does not adopt saved PIDs or recreate worktrees.
The existing process-group cleanup and hostile same-UID trust limits remain:
tracked process cancellation is not a hard OS-wide resource quota or a guarantee
that detached descendants cannot survive. No automatic external-effect replay is
introduced.

## Always outside this initial authority

Remote push, publication, deployment, external mutations, destructive migrations,
merge into main, credential inspection by model tools, dependency installation,
changes outside assigned checkouts, protected instruction/configuration changes,
unknown commands and unlisted file operations do not execute. Some require
external setup/new trusted planning; unsupported operations cannot be made
executable by a textual acknowledgement or permission resolution. Product choices
and effect reconciliation remain genuine human judgments.

Token/monetary usage is not measured or capped. The contract says so before
consent. Enforced resource bounds are wall-clock expiry, owned process timeouts,
task concurrency, coding attempts and review cycles. Capability/help probes have
15-second process limits; harness Git commands have 60-second limits, all still
bounded by the original total deadline. Installed providers and
system runtimes remain trusted local software; no general hostile same-UID
isolation claim is added. The OS sandbox applies to project verification, while
provider confinement uses its documented restricted/tool-free interface.

The Claude adapter's tool-free controls follow the
[official CLI reference](https://code.claude.com/docs/en/cli-reference) and
[bare-mode authentication/discovery contract](https://code.claude.com/docs/en/headless#start-faster-with-bare-mode).
Capability probes fail closed if the required controls are not advertised.
No paid inference was used to validate this implementation.

## Database/protocol compatibility

Additive migration `0006_delegations.sql` adds immutable proposal and authorization
projections with update/delete guards. The authorization projection is inserted
with the initial run and lifecycle event in one transaction. Existing migrations
and resolution receipt semantics are unchanged.

`EngineeringRun.delegation` is optional/defaulted. Missing/null means Supervised;
no existing run is promoted. Existing request shapes and v1 endpoints remain
valid. The added proposal/inspect/authorize endpoints use the same authentication,
Origin/Host restrictions, body bounds, slow-body deadlines and admission limits.
The v1 contract/profile semantics must not be widened in place: broader tools,
backends or filesystem grants require a new contract/profile version and fresh
consent, or continued enforcement of the original version's restrictions.
No token, credential value or raw inherited provider environment is added to the
protocol or native bridge. The UI design, provider tools for Supervised runs,
completed evidence, revision guards and local integration gates remain intact.

## Acceptance and validation

The reproducible separate-process fixture is
`crates/autono-cli/tests/delegation.rs`. It drives the real CLI and daemon,
production Claude adapter, real disposable Git worktrees, SQLite, process
supervision, automatic preflight and sandboxed checks. Its `claude` and `codex`
executables are deterministic shell fixtures on a private PATH; installed providers
and inference APIs are never invoked. The fixture asserts the production adapter
keeps tools empty and never adds directories or resumes a session.

The feature creates executable shell programs: `a.sh` prints A, `b.sh` prints B,
and `c.sh` sources A then prints C. Configured sandboxed checks require successful
exits and exact output. A/B execute independently; C depends on A. B's first check
fails, its second
coding attempt fixes the content, all three receive fresh independent reviews,
and local integration passes the combined check. No routine `approved`,
`continued`, or `integration_requested` event is permitted after consent. Main
and the developer checkout remain unchanged. Completed consent replay after a
real daemon restart does not invoke another provider. Additional real-daemon
cases crash during a planner and remove the configured provider executable;
interruption/replay and durable prerequisite failures are checked explicitly.

The recorded safe run is `f11058f4-dd46-4a8c-8781-6ac4e46f148d`, with 142
lifecycle events. Its pinned main remained
`7701fa3276136af253d035638f359a11cf354b7b`; integration preserved
`46a7930e77b397e24752cfb2ab230617d9071bd0`.

A separate native lifetime smoke launched the optimized bundled macOS app against
the same disposable fixture home. After consent, the owned desktop process
(PID 68635) was terminated during PLANNING; daemon PID 68382 remained alive and
run `67130986-5445-458b-9c2b-a409851e09d9` completed unattended. Its local integration
commit is `08a1727b8766b33843a7e3fb93e91ec1e6b020c4`, with main unchanged. Both owned
application/daemon processes were stopped after the smoke; no installed coding
provider or inference API was invoked.

Focused core tests cover out-of-language shell commands, unlisted/path-escaping
edits, uncertain effects, login failures, independent-work preservation, changed
checks/base, runtime configuration/repository-identity drift, policy denial,
deadline/cancellation, exact human resolution with automatic continuation,
nondelegated integration, immutable projections and legacy document decoding.
Sandbox tests attempt real external reads/writes, inherited-secret access and a
live loopback connection. Existing Supervised, authenticated bridge, immutable
receipt/evidence, restart and adversarial suites also run unchanged.

Final macOS validation passed: **116 Rust tests**, two intentionally ignored;
**100 frontend tests**; TypeScript; zero-warning ESLint and Clippy; workspace
formatting/debug and release builds/docs; Rust 1.89.0 checks for all non-desktop targets; production
frontend/startup smoke; optimized native Tauri build; authenticated bridge and
migration/persistence regressions. Heavy test fixtures are bounded separately
from each run's actual worker concurrency. Linux is left to CI and requires
Bubblewrap; missing/unusable sandbox support blocks autonomous admission.

To retain an acceptance result and all disposable artifacts:

```sh
AUTONO_DELEGATION_EVIDENCE=/absolute/evidence.json \
  cargo test -p autono-cli --test delegation -- --nocapture
```

The machine-readable evidence and final command results are recorded in
[m5-delegation-acceptance.json](m5-delegation-acceptance.json).

## Remaining work for one-click Build

A prompt-first desktop consent surface, dynamic scoped source discovery, broader
file operations/providers, larger prompt delivery, richer sandboxed toolchains,
reliable vendor usage accounting and general crash-safe continuation are not part
of A2. Neither are developer memory, autonomous product decisions, intelligent
access deferral or final decision feedback. The CLI currently requires an explicit
file list and supported local checks. A fresh independent security/architecture
review is still required before treating this milestone as approved.
