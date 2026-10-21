# BRU-342 — Execution Observability, Chunk 2

Chunk 2 continues the approved Instrument + Observatory + Soft Neon client on `feat/bru-342-native-mission-control`, following Chunk 1 `aa3cccb98b1ae82eb4188e1abf995368ad3ca844`. Delivery remains Draft [PR #4](https://github.com/bernacle/autono/pull/4). No merge, push from the application, deployment or paid provider acceptance was performed.

## Architecture and implementation

The existing Tauri 2 / React client remains a client of the authoritative Rust daemon. No daemon orchestration, migrations, protocol semantics, authentication, endpoints, permission admission or resolution receipts changed. JavaScript receives no bearer token, credential file, process environment or raw provider result. The only production Rust bridge addition projects the timestamp already present on sanitized process events onto complete log lines. Existing redaction and bounded replay remain intact.

The pure TypeScript model normalizes recorded journal conventions and joins UUIDs to authoritative run snapshots. It derives task-state intervals, coding/check/review phases, execution epochs, attempts, retries, human pauses, dependency waits and local integration. Sequence numbers establish causal order; record times establish duration. Missing starts, ambiguous process joins, unknown kinds, redacted identifiers and contradictory boundaries remain unparsed/incomplete rather than generating estimated phases. Details and test fixtures are in [event mapping](desktop-redesign-chunk-2/event-mapping.md).

| Component | Result |
|---|---|
| `lib/execution-model.ts`, `lib/execution.ts` | Tested pure projection, bounded normalized history, singleton live clock, interval unions and transitive task relationships |
| `components/execution.tsx` | Operational statistics, compact/expanded timeline, whole-run/15-minute ranges, live-follow, attempt markers, dependency/human waits, exact phase sources, reviewer findings, verification ledger and retained session archive |
| `components/journal.tsx`, `lib/journal.ts` | Human-readable chronological journal, task/attempt attribution, useful filters and original sanitized evidence disclosures; unparseable results never acquire a guessed verdict |
| `components/phase-source.tsx` | Lazy retrieval of an exact older source event through the existing cursor API |
| `components/log-dock.tsx`, `activity.tsx` | Sanitized nonmodal process dock, timestamps, bounded history, follow control, expand/close and keyboard focus restoration |
| `App.tsx`, `agent-progress.tsx` | Actual open agent sessions, relevant work and dependency lanes, verified task counts, human attention, last recorded engineering progress and last complete process output kept distinct |
| `run-detail.tsx`, `tasks.tsx`, `dependency-graph.tsx` | Shared task/phase selection across list, graph, timeline and inspector; URL selection; stronger directed graph edges; unrelated branches dimmed |
| `new-run.tsx` | Planner acknowledgment and footer remain reachable at 960×680 |
| Bootstrap / build | Visible loading state, recoverable render boundary, production module smoke check, vendor split and native module loading without fetch-based preloading under the unchanged IPC-only CSP |

Dark and light semantic tokens, ground-only static grain and opaque data surfaces remain those approved in Chunk 1. Historical coding uses neutral white 14% / black 10%; luminous lavender denotes live execution only. There is no glow, invented completion percentage, ETA, token count or cost.

## Native acceptance and recorded evidence

Acceptance used the actual optimized macOS arm64 Tauri application, native accessibility/input and native window screenshots on an Apple M1 Pro. Providers were deterministic `sh` processes injected by the disposable daemon example, including structured Codex/Claude-shaped results decoded through the existing adapter. No paid Codex or Claude process was invoked. All repositories, daemon homes, approval operations and integration branches were disposable.

The preflight confirmed the existing branch, clean tracked worktree, approved Chunk 1 HEAD, merged M3 base `11c5ddde583acbe400f444386a0bf6b24ea22364`, and passing Chunk 1 CI. Pre-existing untracked `docs/design/` references were read and preserved without editing or staging.

| Scenario | Observed result |
|---|---|
| Planning, readiness, explicit execution | Native folder picker and planner acknowledgment; real plan and exact permission review; execution started only after explicit confirmation |
| Failed check and retry | Task C's first check failed; attempt 2 passed. Both check outcomes and both coding attempts remain visible |
| Rejected independent review | A's first review rejected its artifact with a medium finding; coding attempt 2 passed review with a retained low finding |
| Invalid review and retry | E's first review was invalid; a second independent review passed on the same coding attempt |
| Human interruption | B paused, descendant D waited, and the run reached BLOCKED. Resolving the action left the run BLOCKED at revision 109 with zero processes |
| Explicit continuation | Native Continue confirmation reached RUNNING revision 118 and B attempt 2. Resolution alone launched no agents |
| Local integration | Five verified tasks, native Integrate confirmation, integration verification and COMPLETED revision 162. Original main remained pinned to its original commit |
| Terminal verification failure | Separate run failed three checks across three coding attempts, reaching FAILED revision 41 with zero verified tasks |
| Journal replay and cursor gaps | Completed run replays 184 events; failure run replays 45 records with a real global sequence gap from 8 to 19 caused by a second run. Reload preserves order and attempts |
| Exact historical source | Native inspector fetched `agent_started #48` for an older rejected review through the existing cursor API, beyond the initially rendered journal page |
| Close/reopen | UI closed while run revision 56 had two processes; the same daemon PID remained reachable and work continued. Reopen recovered durable state |
| Connection loss/recovery | Temporarily moved only disposable owner-only metadata; saved state remained visible and mutations disabled; restoring the same file reconnected |
| Themes and window sizes | Native dark/light at 1440×900 and 960×680; minimum New Run, permission inspection, inspector and log dock checked |
| Navigation and keyboard | Native resize/drag, sidebar navigation, command navigation, graph/phase selection, inspector scrolling, dock focus/Escape; deterministic tests cover row/phase arrows and the joined minimum-window focus loop |

Machine-readable identities, revisions, check counts and commit identities: [evidence.json](desktop-redesign-chunk-2/evidence.json). No credential metadata or database is included in delivery. The fixture state remains under `/tmp/autono-native-bru342-observability-*` for inspection. Restarting the completed/failed fixture daemons for final captures does not start any agents.

## Screenshots

All images are unmodified captures of native Tauri windows, not HTML mockups. The [screenshot index](desktop-redesign-chunk-2/screenshots/README.md) identifies each state and size; [manifest](desktop-redesign-chunk-2/screenshots.json) records dimensions and SHA-256 hashes. The final copyable delivery folder also includes the optimized application archive.

## Validation

| Check | Result |
|---|---|
| TypeScript typecheck and frontend lint | Passed, zero lint warnings |
| Frontend unit/integration tests | 62 passed across four files |
| Production frontend | Passed; emitted modules also render the disconnected application with `fetch` disabled |
| Native Tauri release build | Passed, macOS arm64 `.app`, approximately 14.82 MiB |
| `cargo fmt --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 92 passed, two ignored (paid-agent acceptance and an isolated helper) |
| `cargo build --workspace` | Passed |
| `cargo doc --workspace --no-deps` | Passed |
| Authenticated Rust bridge coverage | Four tests pass, including RFC3339 process timestamps, sanitization, credential metadata validation and access checks |
| Full diff | Reviewed; no design-reference files staged, no daemon semantic changes |

CI results for the pushed head are reported in the PR and final handoff. Intermediate local failures were corrected before delivery, including custom-scheme selection, minimum-window focus ordering, and conservative journal result labels.

## Performance and remaining limits

History fetches pages of 100, catching up every 250 ms and following every 2 seconds. Snapshots poll every 2.5 seconds and health every 3 seconds. The normalized timeline is capped at 5,000 events with an explicit limit notice; the journal/log buffers retain at most 500 records/lines. The journal initially renders 50 records and reveals more on request. Exact old phase evidence can be fetched separately. Queries release unused history after 60 seconds.

The model is memoized on history/snapshot changes. A shared one-second clock updates live geometry without reparsing history. A deterministic 5,000-event synthetic projection measured roughly 9–13 ms median on this M1 Pro under normal load; host contention produced higher results. These are projection timings, not a claim of sustained 60 fps. Large milestone groups collapse initially above 25 tasks.

Deliberate differences and remaining limits:

- Event strings are current implementation conventions rather than a typed phase protocol. Unknown or incomplete records remain inspectable; they do not acquire guessed bars or verdicts.
- Times are daemon journal record times at JavaScript millisecond precision. Admission may precede actual agent process start. Optional process-start/end projections were not added.
- Projection recomputes the bounded history on snapshot/history changes; per-task incremental projection and formal frame-rate profiling remain future optimizations.
- Verification commands retain individual sourced intervals instead of one aggregate check bar.
- Only adjacent dependency connectors appear inside the timeline. The synchronized DAG provides the complete dependency structure and directed edges.
- The fixture can emit an existing supervisor cleanup `EPERM` warning; it stays visible as original process evidence. Orchestration was not changed to hide it.
- Native acceptance covers macOS arm64. Signing/notarization, other native platforms and real paid-provider sessions were not exercised.
- Configuration editing and broader workflow behavior remain unchanged. This chunk does not add auto-resume or automatic integration.

PR #4 remains Draft. The separate independent technical, security and native UX review has not been started.
