# Engineering loop v0 acceptance evidence

Executed locally on macOS on 2026-10-19 using Codex CLI 0.163.0 and Claude Code
2.1.303, with the user's existing CLI login. No authentication files/tokens were
read or copied by Autono. The opt-in test used a separate submitting CLI process.

The disposable standard-library Python seed initially passed 3 tests. Claude
planned one implementation task for configuration validation. Codex implemented
it in its own worktree; deterministic verification passed 18 tests. A fresh
Claude reviewer accepted every criterion with no blocking findings. The harness
preserved a task commit, then merged it into a separate local integration branch
and passed the same 18 tests there. The run became COMPLETED in 89.89 seconds.
No developer checkout was modified and no merge to main/push/PR was performed.

- Engineering run: `bb121a5c-3c05-442b-adbd-91e16e0fa106`.
- Seed commit: `59b08210ffb17b2a84d7e6dc9e4bfcdc766d047c`.
- Task `T1` commit: `65f3faa04c968fb0d24e29b4e8bf3adba0378cdc`.
- Integration commit: `dac02d5032fb17d857e768cda5a63e8192504758`.
- Integration branch: `autono/bb121a5c-3c05-442b-adbd-91e16e0fa106/integration`.
- Coder process: `6783b036-ff71-46d5-a3e1-5065fdafab25`.
- Reviewer process: `3bd973c8-327b-4a1f-ab50-a4e8329f75e3`.
- Task verification process: `4b00e164-cb5a-4085-a05b-619bce22906b`.
- Combined verification process: `fc0a303e-7a7c-4288-a0b8-a48b49a815ed`.

Retained directory:

`/private/var/folders/7z/d0cj4crj1x96fmpvld8rlb4h0000gn/T/autono-real-agent-4632712d-253e-4893-901d-e676e1b1060d`

It contains `planning-result.json`, `execution-result.json`, `final-result.json`,
`daemon.log`, the SQLite database/event streams, schemas, diff artifacts, and
planner/task/integration worktrees. This is a temporary-directory location:
preserved by the test, but not a permanent archival guarantee from the OS.

The reviewer recorded two nonblocking suggestions: catch unusual JSON decoder
ValueError/RecursionError cases, and add a subprocess-level error-output test.
One was LOW and one INFO; the configured policy therefore needed no fix cycle in
this real run. Deterministic fake-agent tests separately exercise rejection,
automatic fixes with coder resume, re-verification, and review exhaustion.

The host reported Runtime v0 process-group cleanup EPERM warnings on several
short-lived commands. These remain in process/evidence records independently of
observed zero exits and complete output. This acceptance result does not assert
that group cleanup was confirmed or that escaped descendants cannot survive.

An initial acceptance attempt failed before inference because Claude's variadic
`--add-dir` argument consumed the prompt. Adding the explicit `--` separator
fixed it; the successful run above used that correction. Its failed predecessor
is also retained in `autono-real-agent-772e48a9-187f-43d1-a744-87f9edb04850` under
the same temporary parent directory. Failed runs were never labelled completed.

This reviews the sample implementation through Autono. It is **not independent
review of the engineering harness itself**. The new checkpoint/cancellation
boundaries, provider permissions, workspace auditing, and integration paths still
need independent systems/code review before treating this milestone as approved.

Final local workspace validation on macOS passed:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`: 53 passed; real-agent test ignored by default.
- `cargo build --workspace`
- `cargo doc --workspace --no-deps`
- `git diff --check` and staged diff inspection.

All 33 original tests remain passing. New tests include cancellation while SQLite
is locked, cancellation while planning HTTP admission is saturated, context
symlinks escaping a checkout, actual daemon restart reconciliation, and rejected
forged completion evidence. Linux was not executed locally; the existing CI
matrix includes Linux and macOS. The successful real-agent run predates the final
cancellation/diagnostic/context hardening, which is covered by deterministic tests;
real inference was not repeatedly consumed for those changes.
