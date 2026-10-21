# Autono desktop — Native Mission Control v0

Tauri 2 + React/TypeScript/Vite, Tailwind, shadcn-style Radix components,
TanStack Query and Lucide. This is a client of the existing Rust daemon.

The Instrument shell uses an inset opaque content sheet over an Observatory
ground. Soft Neon colors identify live work, human attention and failures;
verified states use small mint glyphs. Dark is the first-launch default. Settings
offers explicit System / Dark / Light choices and a local background-texture
toggle. Inter, JetBrains Mono and the static grain tile are bundled for offline use.

## Develop and build

Install Node 24+, Rust 1.90+ and the [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/).
The existing daemon crates retain Rust 1.89 support. On Linux the native webview
requires WebKitGTK 4.1 and GTK development packages (see CI).

```sh
cd apps/desktop
npm ci
npm run tauri -- dev
npm run tauri -- build --bundles app # macOS application, unsigned for local use
```

Start the existing daemon separately using `cargo run -p autono-cli -- daemon`.
Both processes must use the same `AUTONO_HOME`, or the existing OS-local default.
Launching from Finder does not inherit a shell's custom PATH or AUTONO_HOME;
launch the app binary with those variables when using a nondefault daemon home.
Opening the desktop never starts the daemon, runs commands, installs tools or
invokes an agent. Closing it has no daemon shutdown hook.

```sh
npm run typecheck
npm run lint
npm test
npm run build
```

## Boundaries and operations

- `src-tauri/src/bridge.rs` implements a fixed operation enum, UUID/identifier
  parsing, bounded responses, timeouts and owner-only metadata validation. It
  rereads metadata on every invocation to reconnect after daemon token rotation.
  It accepts only nonzero loopback addresses, disables proxies and redirects, and
  never returns the bearer token. No generic HTTP, filesystem or shell command is
  exposed to JavaScript. Only the local main window has bridge/dialog capability.
  CSP denies remote scripts, navigation frames and network connections.
- Daemon APIs remain unchanged and authoritative. The bridge checks the inspected
  revision before mutation. This is a stale-view guard, not an atomic compare-and-
  swap API; the daemon performs the final state/admission checks. Mutations never
  retry automatically. Explicit confirmations cover execution, continuation,
  cancellation and integration. Accepted means accepted, not completed.
- A planning submission UUID is retained while its payload is unchanged. Human
  resolution retries retain the exact request UUID/payload across panel remounts
  for the desktop session, including the exact permission object. Decisions start
  empty; effects start Not inspected. Changed action objects require explicit
  reinspection. The bridge allows matching audited receipts to be retried
  after the run revision changes. An ambiguous action response is never shown as
  success: refresh state before taking another action.
- Run/task states are displayed as stored. The dependency view labels every edge
  and recursively identifies blocked ancestors without converting WAITING into a
  fabricated failure. Readiness, immutable checks, full review attempts, criteria,
  findings, task/integration commits and owned workspace paths are inspectable.
- Engineering and process events poll by durable `after` cursors, 100 records per
  page. A full page catches up at 250ms; normal polling is 2 seconds. Each viewer
  retains at most 500 entries. Replay starts at zero on a fresh app session;
  disconnects retain the last confirmed cursor. Run aggregates are paged 20 at a
  time. Overview and human-action queues identify their recent-run coverage. The daemon
  supplies total executing-process counts; the recent process list is the latest
  50, not an asserted complete inventory.
- Process environments, command argv and structured agent results are excluded
  from process-list projections. Rust buffers output across byte/event/page
  boundaries until complete lines, redacts the bearer, recognized credential
  patterns and sensitive desktop environment values and sensitive process environment overrides, strips controls, and
  withholds lines over 16 KiB. Readers cap at 32; an expired reader requests an
  explicit replay. Redaction cannot identify arbitrary unlabeled secret text:
  providers and verification commands must obey the daemon's existing rule not
  to print secrets. No credential file is read. Arbitrary artifact contents are
  never read or executed; associated workspace accessibility is checked on demand.
- Settings are accurate read-only user-file inspection, with a separate live
  daemon health/power view. CLI availability is a filesystem PATH lookup, not a
  command invocation. Effective project settings are the immutable run snapshot.
  Configuration editing is intentionally left to the user's editor.

## Navigation and keyboard

Overview prioritizes Working now and Needs you. Runs have Tasks (List / Graph),
Execution, Verification, Plan, Preflight and Integration tabs, with one eligible
primary action and cancellation in the overflow menu. Tasks open a contextual
inspector. Human actions use pinned exact scope, scrolling context and a separate
decision bar. New run and Readiness are full-sheet flows.

`⌘N` opens New run; `⌘1/2/3` selects Overview / Human actions / Runs; `⌘K`
opens safe navigation; `⌘\\` toggles the sidebar; `⌘R` refreshes daemon state;
`⌘⇧C` copies the selected run ID. Lists support J/K and Enter; graph arrows
follow dependencies; Esc closes the inspector. G then L/G selects task List/Graph
outside text inputs. `⌘↵` submits a valid planning or human-decision form.
Opening a form or page never grants execution authority.

The full execution timeline, journal-derived intervals, new log dock and
observability engine are a separate Chunk 2. Existing sessions, journal filters,
review evidence and bounded process logs remain available in this chunk.

## Native acceptance without inference

The example is test-only and requires a fresh absolute disposable directory:

```sh
cargo run -p autono-desktop --example disposable_daemon -- /tmp/autono-desktop-check
AUTONO_HOME=/tmp/autono-desktop-check target/debug/bundle/macos/Autono.app/Contents/MacOS/autono-desktop
```

It uses the existing daemon API/router, supervisor, SQLite, real Git worktrees,
preflight, permissions, verification and integration, injecting only fake provider
commands through the existing `AgentRuntime` test seam. No Codex/Claude executable
or paid API is called. For slower native inspection, set
`AUTONO_FIXTURE_DELAY_SECONDS=8` (numeric 1–60; default 2). Select its printed
fixture repository using Browse, paste a
specification, acknowledge planning, inspect the graph, resolve the product choice
and exact checks permission, refresh preflight and approve execution. B pauses for
an effect inspection while A/C/E finish; D waits on B. Resolve the live pause with
an inspected effect and explicitly continue, then integrate the verified run.
Inspect process output, close the native window and query the still-running
daemon. Stop the disposable daemon yourself afterward.

Native screenshots and observed results: [acceptance evidence](../../docs/desktop-acceptance.md).
The redesign's separate validation status is in
[Chunk 1 report](../../docs/desktop-redesign-chunk-1.md); the original M4 captures
do not certify the redesigned native layouts.

## Current limits

No generalized resume, configuration write UI, file-content viewer, daemon service
installer, push/merge/deploy action, cloud service or additional provider. Large
single run aggregates above 16 MiB require CLI inspection. Event cursors are not
saved across application restarts; replay is explicit and bounded. This unsigned
macOS bundle has no distribution signing/notarization or auto-updater configured.
