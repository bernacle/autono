# Packaged native acceptance

These are unedited captures of the packaged macOS Tauri WKWebView, not browser
mockups. JPEG dimensions and SHA-256 hashes are recorded in
[screenshots.json](screenshots.json). The OS repository selector has its own
880×448 sheet; application windows were measured at 1440×900, 1280×900 and 960×680.

[Authenticated daemon receipts](receipts.json) bind the successful native run
`c27938af-dce7-4aa1-8ced-c4a10bba4227` and partial run
`f1174a5c-9019-461e-b71f-4ccd04ffc924` to v2 grants with empty exact-file lists.
Each has one authorization event and no extra routine approval, continuation or
integration request. The successful run has five independently reviewed/verified
tasks and passing combined integration checks. The partial run preserves one
verified task and the exact validation prerequisite, with descendants waiting.
Main remains `996734b7994115ded16c51b582ea52f96be9b92e`.

The desktop was quit while the first run showed active implementation, then
reopened after the same daemon completed it. The re-opened native view showed
“Ready for review”, all five source paths, integration location and the actual
A3 decision ledger. No external service or paid inference was invoked. The
native fixed-route bridge test separately drops/reopens its client before the
real daemon finishes and asserts exact-consent idempotency/tamper rejection.

The existing shell-only Claude fixture from `crates/autono-cli/tests/build.rs`
was retained using `AUTONO_BUILD_KEEP=1`. Start its actual daemon with the fixture
binary directory on PATH, `AUTONO_FIXTURE_ROOT=<retained-root>`, a dummy
`ANTHROPIC_API_KEY`, and its owner-only `AUTONO_HOME=<retained-root>/home`; launch
the packaged app with that same `AUTONO_HOME`. No user installation/login is used.
A test-only gate around the fixture planner and its existing coder hold made
live planning/execution visible. A separate planner response added a task-scoped
validation prerequisite for the partial capture. These changes were to the
trusted disposable provider fixture, not production code or the selected source
repository. The executable fixture never calls an inference service.

Observed native checks:

- OS repository selection, prompt entry, bounded initial authorization, running
  tasks, verified handoff and exact partial blocker are visible in the captures.
- Build and handoff layouts were inspected at all three requested sizes; dark
  and light Build surfaces were inspected at every size.
- The minimum-size authorization scrolls to its complete exclusions and consent
  controls. Escape dismisses an unsubmitted proposal, keyboard focus can leave
  the prompt, and Cmd+N opens Build. The graph and Supervised mode remain reachable.
- Necessary requests stay inspectable; no routine execution/integration
  approval is presented by the autonomous summary.
- Native accessibility automation intermittently returned an empty tree or
  timed out. Fresh full snapshots and actual window screenshots were used to
  verify outcomes; those failed calls were not counted as successful checks.
  This was tool-driven smoke acceptance, not a comprehensive VoiceOver audit.

Planning/execution images record the first packaged run. Its later handoff and
size/theme captures include the final plain-language progress refinement. The
runtime authority, source profile and native bridge workflow are the same.

| View | Actual native capture |
| --- | --- |
| Default Build, dark | [1440×900](default-dark-1440x900.jpg), [1280×900](default-dark-1280x900.jpg), [960×680](default-dark-960x680.jpg) |
| Default Build, light | [1440×900](default-light-1440x900.jpg), [1280×900](default-light-1280x900.jpg), [960×680](default-light-960x680.jpg) |
| Repository selector | [Native OS sheet](repository-selection.jpg) |
| Feature request | [Prompt entry](prompt-dark-1440x900.jpg) |
| Initial grant | [Dark](authorization-dark-1440x900.jpg), [minimum-size light](authorization-light-960x680.jpg), [scrolled consent controls](authorization-light-960x680-scrolled.jpg) |
| Live planning/execution | [Planning](planning-dark-1440x900.jpg), [execution](execution-dark-1440x900.jpg) |
| Verified handoff | [1440×900](ready-dark-1440x900.jpg), [1280×900](ready-dark-1280x900.jpg), [960×680](ready-dark-960x680.jpg) |
| Partial completion | [Outcome](partial-light-960x680.jpg), [exact request and retained decision](partial-request-light-960x680.jpg) |
| Advanced observability | [Dependency graph](advanced-graph-light-960x680.jpg) |
| Supervised mode | [Original planning controls](supervised-light-1440x900.jpg) |
