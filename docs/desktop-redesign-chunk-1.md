# BRU-342 — Native Mission Control redesign, Chunk 1

Chunk 1 implements the approved **Instrument + Observatory + Soft Neon** direction on the existing M4 client. Draft [PR #4](https://github.com/bernacle/autono/pull/4) remains the delivery PR; the branch is `feat/bru-342-native-mission-control`. Chunk 2 has not started.

## Architecture and boundaries

React, TypeScript, Vite, Tailwind, Radix/shadcn-style controls, TanStack Query and Lucide remain inside the Tauri 2 application. The existing Rust bridge still owns authenticated daemon communication, credential metadata validation, action admission, sanitization and artifact access checks. No daemon implementation, orchestration, migration, protocol, endpoint or production bridge payload changed.

The only Rust source change is optional bounded pacing in the shell-only acceptance example. Native capabilities add window dragging and native maximize behavior; they add no shell, HTTP or credential filesystem access. Fonts and texture are bundled under the existing self-only CSP. Closing the application does not signal the daemon.

The preflight verified main `11c5ddde583acbe400f444386a0bf6b24ea22364`, merged PR #3, and the independently reviewed M3 preflight/escalation implementation. Work continued from M4 commit `8b972d8dc0741877ea5aece47f98b2ac52c30cab` on the existing PR branch. User-provided `docs/design/` was already untracked; it was read and preserved without edits or staging.

## Components and tokens

| Area | Changes |
|---|---|
| `styles.css`, `lib/appearance.ts`, assets | Dark default, live System/Dark/Light preferences, opaque inset sheet, ground-only static grain, bundled Inter and JetBrains Mono, reduced motion/increased contrast support, focus rings and shared controls |
| `App.tsx` | Native overlay shell, 228 px sidebar / 56 px rail, connection popover, safe command palette, Working now, Needs you, compact activity, recent runs, processes and last-known disconnected state |
| `human-action.tsx` | Master queue / pinned authoritative request / scrolling context / fixed decision area; minimum-window switcher; durable receipt identity across remounts; explicit revision reinspection; read-only terminal records |
| `run-detail.tsx`, `lifecycle.tsx` | Header metadata, lifecycle, one primary action, disabled reasons, cancellation overflow, structured blockers, six contextual tabs and guided Readiness |
| `tasks.tsx`, `dependency-graph.tsx` | Milestone list, real DAG edges, human-held descendants, keyboard selection, inspector and responsive focus-trapped overlay |
| `new-run.tsx` | Full-sheet six-step guide, native folder picker, pinned-base input, byte limit, configured planner and explicit inference acknowledgment |
| `settings.tsx`, `evidence.tsx` | Read-only configuration boundaries, local appearance editing, complete retained sessions/checks/reviews/findings/worktrees/integration evidence |
| UI primitives | Native radio semantics with no default decision, Radix overflow menu and accessible copy feedback |

Dark semantic tokens: running `#9FADF6`, attention `#FFBB7B`, success `#8FDABD`, failure `#EC8697`, neutral `#83868B`. Light equivalents: `#5460B3`, `#9E5711`, `#22765C`, `#B03D58`, `#66696E`.

Historical coding is neutral `rgb(255 255 255 / .14)` in dark and `rgb(0 0 0 / .10)` in light. Only live execution uses the luminous running token. These segment tokens prepare the visual foundation; no execution timeline was added. Glow is disabled.

## Native acceptance

The actual optimized macOS Tauri window was operated through native accessibility and screenshot inspection. HTML design mockups were used only as design references. Captures include 1440×900 and 960×680, plus inspector checks at 1280×900 and 1100×900.

The fixture used the real daemon router, bearer authentication, SQLite persistence, process supervisor, Git worktrees, checks and reviews with injected **shell-only** agents. `AUTONO_FIXTURE_DELAY_SECONDS=8` made live phases inspectable. No Codex/Claude executable or paid inference API was invoked.

| Check | Observed result |
|---|---|
| Native repository picker and planning | Selected the disposable repository; explicit acknowledgment submitted the original planning request and produced five tasks / two milestones |
| Permission safety | No disposition selected; effect starts Not inspected; exact `run_checks` operation and full scope visible verbatim at 960×680; confirmation remains visible while context scrolls |
| Resolution payloads | Product decision, exact permission and runtime checkpoint persisted with request IDs and nonsecret notes; operation/scope matched the daemon object |
| Runtime effect guard | Unknown effect prevented resolving a continuable checkpoint; selecting Not yet reset the inspection checkbox |
| No implicit execution | After the runtime resolution the run stayed BLOCKED at revision 80 with B BLOCKED_HUMAN and D WAITING; a separate confirmation was required to continue |
| Live state and dependencies | Two initial workers appeared; A/C/E completed, B escalated, and the DAG accurately marked D as waiting on B |
| Window close | Closed the macOS window during continued work; daemon PID 18941 still answered with RUNNING revision 98 and B RUNNING; reopening restored the run |
| Verification and integration | Five accepted independent reviews and matching verified/preserved task trees; explicit integration completed at revision 133 with a passing combined check |
| Original checkout | Fixture main remained `ac5b3b07024b7339f8e498964b79b6295dc1cc36`; integration commit is `585fe0029c7d7864e84af343cf56f857d42149ef` on the daemon's local integration branch |
| Stale action | Planner completion changed a second run from revision 3 to 6 while cancellation confirmation was open; the request was rejected. Reinspection then cancelled it at revision 7 |
| Terminal records | Cancelled action records remain exact and inspectable, with no resolution controls; they are excluded from actionable Needs you counts |
| Events and output | Main run replayed 155 ordered durable events; journal filtering and sanitized process/check output worked; native output showed confirmed cursors |
| Disconnection | Temporarily moved only the disposable metadata file; one banner identified unavailable connection, retained saved state and disabled mutations; restoring metadata reconnected |
| Appearance and interaction | Dark/light main screens, texture off/on, 960/1100/1280/1440 layouts, context scrolling, minimum permission confirmation, native header dragging, Cmd+N, Cmd+1/3, Cmd+K search/Enter, graph arrows and inspector Escape/focus handling inspected |

The acceptance app and owned disposable daemon were stopped after capturing evidence. The disposable repository and database remain under `/tmp/autono-native-bru342-redesign-02` for inspection. Credentials are not included in committed evidence.

Machine-readable results: [evidence.json](desktop-redesign-chunk-1/evidence.json). Native screenshot index: [screenshots](desktop-redesign-chunk-1/screenshots/README.md). Dimensions and hashes: [screenshots.json](desktop-redesign-chunk-1/screenshots.json). The portable [gallery](desktop-redesign-chunk-1/gallery.html) opens locally with the adjacent screenshot folder.

## Screenshots

| Native screen | Dark | Light |
|---|---|---|
| Overview / live work | [1440×900](desktop-redesign-chunk-1/screenshots/overview-running-dark-1440.jpg) | [1440×900](desktop-redesign-chunk-1/screenshots/overview-running-light-1440.jpg) |
| Permission / three-zone workspace | [1440×900](desktop-redesign-chunk-1/screenshots/human-permission-dark-1440.jpg) | [1440×900](desktop-redesign-chunk-1/screenshots/human-permission-light-1440.jpg) |
| Permission / minimum window | [960×680](desktop-redesign-chunk-1/screenshots/human-permission-dark-960.jpg) | [960×680](desktop-redesign-chunk-1/screenshots/human-permission-light-960.jpg) |
| New run | [1440×900](desktop-redesign-chunk-1/screenshots/new-run-dark-1440.jpg) | [1440×900](desktop-redesign-chunk-1/screenshots/new-run-light-1440.jpg) |
| Readiness | [1440×900](desktop-redesign-chunk-1/screenshots/readiness-dark-1440.jpg) | [1440×900](desktop-redesign-chunk-1/screenshots/readiness-light-1440.jpg) |
| Tasks / graph | [Human-blocked DAG](desktop-redesign-chunk-1/screenshots/graph-blocked-dark-1440.jpg) | [Planned DAG](desktop-redesign-chunk-1/screenshots/graph-light-1440.jpg) |
| Inspector / minimum window | [960×680](desktop-redesign-chunk-1/screenshots/task-inspector-dark-960.jpg) | [960×680](desktop-redesign-chunk-1/screenshots/task-inspector-light-960.jpg) |
| Local integration | [1440×900](desktop-redesign-chunk-1/screenshots/integration-dark-1440.jpg) | [1440×900](desktop-redesign-chunk-1/screenshots/integration-light-1440.jpg) |
| Disconnection | [960×680](desktop-redesign-chunk-1/screenshots/disconnected-dark-960.jpg) | [960×680](desktop-redesign-chunk-1/screenshots/disconnected-light-960.jpg) |
| Settings | [Appearance](desktop-redesign-chunk-1/screenshots/settings-appearance-dark-1440.jpg) | [Daemon](desktop-redesign-chunk-1/screenshots/settings-light-1440.jpg) |

## Validation

Fresh checks for this chunk:

| Command | Result |
|---|---|
| `npm run typecheck` | Passed |
| `npm run lint` | Passed, zero warnings |
| `npm test` | 29 passed across two files |
| `npm run build` | Passed TypeScript and production Vite build |
| `npm run tauri -- build --bundles app` | Passed optimized arm64 macOS application build |
| `cargo fmt --check` | Passed workspace formatting |
| `cargo clippy -p autono-desktop --all-targets -- -D warnings` | Passed, including the acceptance example |
| `cargo test -p autono-desktop` | Four passed: three bridge unit tests and one authenticated daemon integration test |
| `git diff --check` | Passed |

Frontend tests cover blocked/failed/interrupted rendering, lifecycle placement, graph edges and keyboard navigation, Readiness, final confirmations, server rejection, disconnection, bounded durable event replay/reconnect, no initial decision, effect inspection, exact payloads, idempotent remount retry, changed actions, pinned revisions, terminal records, local appearance, text contrast on allowed surfaces and bundled asset budgets.

The optimized bundle is **14.81 MiB**, compared with M4's 14.63 MiB. On-disk allocation grew by 176 KiB (15168 versus 14992 KiB), within the 1 MB design budget. The static grain is 50,167 bytes, and the two bundled fonts total 113,324 bytes. CSP and asset inspection confirm fonts do not depend on a remote CDN.

The original M4 full-workspace validation is documented separately in [desktop-acceptance.md](desktop-acceptance.md). This report claims the fresh relevant Rust checks above, not a repeat of all original workspace checks.

## Limits and deliberate differences

- Full Execution observability, timeline lanes, progress signals, translated journal, findings rail, log dock and verification ledger remain Chunk 2. Existing session rows, bounded cursor journal, verification evidence and process-output dialog remain functional.
- The DAG uses lightweight SVG connections and ordinary keyboard-accessible buttons. It adds no graph dependency or animated layout engine.
- Human permission inspection and confirmation stay fixed in a separate reading/decision structure, even when static mockup layout would scroll them away. Closed workflow records are read-only and excluded from actionable attention counts, matching daemon admission.
- Nonsecret daemon/project configuration is accurately read-only. Only desktop appearance and texture preferences are editable here.
- Latin font subsets fall back to platform fonts for other scripts. The deterministic grain generator produces a 50 KB file rather than the specification's approximate 12 KB estimate; the complete app remains well within the release-size budget.
- macOS arm64 was built and inspected. Windows/Linux and signing/notarization were not validated in this chunk. The local `.app` is a development delivery.
- Existing daemon process events reported an `EPERM` warning during process-group cleanup on this host. The UI retained that warning; shell checks, five reviews and integration nevertheless completed. Daemon cleanup semantics were not changed to hide it.
- Provider authentication, unavailable-provider behavior and paid coding quality were not exercised with real paid agents. Deterministic transport tests and the existing Rust bridge tests cover the corresponding rejection/disconnection paths.
