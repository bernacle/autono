# BRU-342 — Independent PR #4 review repairs

These changes address the confirmed P1 and five P2 findings against `f7bf3bd9afccc95d7ad71536178424c4ecd3835d` on the existing `feat/bru-342-native-mission-control` branch. [PR #4](https://github.com/bernacle/autono/pull/4) remains Draft pending independent revalidation. No daemon orchestration, protocol semantics, migrations, approval payloads, credentials access, or M5 features changed. Pre-existing untracked `docs/design/` files were preserved.

The reviewer patch at `/tmp/autono-pr4-review-regressions.patch` was inspected and reused. All six frontend regressions and the Rust known-credential-key canary reproduced on the reviewed implementation before the fixes.

| Finding | Root cause | Fix |
|---|---|---|
| P1: JSON credential keys | The redactor recursively visited values but preserved property names, and decoded only object-shaped output lines. Control stripping or JSON serialization could also reconstruct a literal after the initial pass. | Rebuild every JSON object with sanitized keys and recursively sanitized values, including objects inside arrays and scalar JSON lines. Sensitive-field decisions use original keys. Key collisions discard both payloads into a safe marker. Normalize controls before another literal pass, check replacement markers against known literals, and guard serialized log text. Numeric/primitive canaries and nested provider transcript suppression are covered as well. |
| P2: effective review acceptance | Evidence and task rows treated the provider's `accepted` flag as the daemon verdict. | One `reviewAccepts()` helper mirrors `ReviewResult::accepts()`: flag true, every criterion satisfied, exactly empty blocker, and no critical/high/medium findings. Evidence, task indicators and snapshot-backed timeline review outcomes use it. A successful reviewer process stays neutral until its validated verdict event arrives. |
| P2: partial verification | `every(passed)` over the currently available results treated a successful prefix as the whole configured set. | Shared summaries require exact configured command cardinality and ordered argv, unique process IDs and process keys, complete output and successful process evidence. Repeated identical commands require separate results. Pending, partial, terminal incomplete, failed and unavailable states remain distinct; attempts are never combined. Redacted or contradictory command evidence cannot earn a complete-success indicator. |
| P2: verification classification | The event model mapped every `passed=false` record to a failed test. | Join the event's process, target task, attempt and process key to structured snapshot results. Preserve `passed`, `test_failure`, `timeout`, `cancelled`, `infrastructure_failure` and unknown outcomes. Timeline labels, journal, individual evidence, task/attempt summaries and aggregate counts share the same classification. A false event without a reliable join remains unknown; a recorded true event is an authoritative successful process verdict. |
| P2: criterion association | The inspector matched mutable criterion wording instead of immutable IDs. | Match `${task.id}-C${ordinal}` exactly. Only a wholly ID-less legacy review may fall back to an exact, unique label. Mixed IDs, unknown IDs and ambiguous duplicate labels receive no borrowed evidence. |
| P2: review ordinals | Filtering null reviews before numbering compressed subsequent ordinals and detached diagnostics from their sessions. | Preserve each original review record and ordinal before rendering. Invalid review 1 remains a visible unavailable record with its own diagnostic/session/log link; successful review 2 retains ordinal 2 and its original identity. Legacy single-review records keep their existing fallback. |

The UI changes reuse approved tokens and components. The only added phase styles distinguish test failures from infrastructure, timeout, cancellation and unavailable outcomes. Permissions, durable receipts, explicit continuation, cancellation and integration still use the existing daemon rules.

## Regression coverage

- Reviewer-supplied tests: rejected provider acceptance with a medium finding; paraphrased criterion evidence; partial checks; timeout classification; reviewer-process success before validation; original invalid-review numbering.
- Pure semantic tests: every blocking severity; low/info findings; false acceptance, unmet criteria and whitespace blockers; canonical IDs versus misleading/overlapping labels; genuine legacy and mixed-ID records; original review/session ordering.
- Verification tests: repeated commands, duplicated process evidence, mismatched/extra/redacted commands, missing output/exit evidence, multiple attempts, pending versus terminal missing results, unknown/future outcomes, contradictory joins and all five structured outcomes plus unknown.
- Shared classification tests assert the same outcome in the pure timeline, translated journal, per-attempt summaries and aggregate counts. Actual task-row DOM indicators also reject partial checks and ineffective review acceptance.
- Rust unit tests inspect decoded nested keys and values, arrays/scalars, Unicode JSON escapes, control normalization, key collisions, replacement-marker collisions, numeric canaries and serialization reconstruction. A nonempty safe marker prevents redaction from erasing a blocker into apparent acceptance.
- The authenticated bridge test runs a real supervised shell process with credential canaries in process environment values and nested/escaped JSON property names. It verifies unauthorized requests remain rejected, replay remains deterministic, decoded renderer-bound keys/values contain no canaries, safe evidence remains visible, and process environments/specs remain excluded.

## Real native smoke evidence

The optimized macOS arm64 Tauri app was inspected against disposable local daemons and shell-only agents. No paid Codex/Claude acceptance ran.

Run `76896ca5-11b9-42e2-ad7d-43b2ac744b58` configured two commands. The first succeeded and the second timed out after 30 seconds:

- Native revision 23 showed `Checks partial (1/2)` in both the ledger and detailed attempt evidence, without a full-set success indicator.
- The authoritative run reached BLOCKED revision 27 with outcomes `[passed, timeout]`, zero verified tasks and an unresolved explicit human action. Continue remained disabled.
- Native Execution showed `1 passed · 0 test failed · 1 timed out`; its timeline phase and journal both identified the timeout.
- The captured 31-event sanitized journal is now a pure-model regression fixture; it yields one passed check and one timeout, zero unparsed events and zero incomplete intervals.

The existing completed fixture `60c19237-51ff-4096-97e1-5fa5d1609003` was inspected in the native Verification tab. Task E retained unavailable review 1 with diagnostic/provider session `eb969641-06c4-4fd9-a309-141ac86d0bd8`, followed by accepted review 2/provider session `3ea9deb3-6b73-4359-8271-82d823000c2e`. Both records remained readable at 1440×900 and 960×680. Only the owned smoke-test application/daemons were stopped afterward.

[Native screenshots](desktop-review-fixes/screenshots/README.md) · [Native evidence](desktop-review-fixes/native-evidence.json) · [Image dimensions and hashes](desktop-review-fixes/screenshots.json).

## Validation

| Check | Result |
|---|---|
| Frontend tests | 90 passed across seven files |
| TypeScript and frontend lint | Passed, zero lint warnings |
| Production frontend + emitted-module startup smoke | Passed; startup requires no network fetch |
| Native Tauri release | Passed macOS arm64 build, approximately 14.84 MiB; actual native smoke inspected |
| Rust bridge tests | Six unit tests and one authenticated integration test passed |
| Workspace formatting / Clippy with warnings denied | Passed |
| Workspace tests | 95 passed, two ignored (paid-agent acceptance and isolated helper) |
| Workspace build and docs | Passed |
| Complete diff | Inspected; no daemon/protocol implementation or design-reference changes |

CI is checked on the pushed commit and reported in the PR/final handoff. The independent reviewer must validate these fixes before the PR is marked ready or merged.

## Remaining uncertainty

Old or redacted records may lack sufficient evidence to classify a check or validate the full configured set. The UI deliberately reports unavailable/unknown/incomplete in these cases. A newly recorded finish can briefly precede the corresponding snapshot refresh; its false outcome stays unknown until a reliable structured join is available. Exact process verdicts still come from the existing daemon, and the UI grants no authorization.

Native smoke covered macOS arm64 and shell-only agents; other native platforms and paid providers were not exercised. Redaction is a conservative local safeguard for known literals and credential patterns, not a guarantee of detecting arbitrary encoded or previously unknown secrets. Independent review is still pending.
