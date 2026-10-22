# M5 A3 independent review corrections

Scope: the three findings against `b10c9666fed45a77f5fa369c95cb1d2ddc0ba7db`
and the separately reported A2 expiry receipt. No new delegation grants, provider
tools, inference loops, A4 features or remote-effect retries are introduced.

## Reproductions

The independent review fixtures were available under `/tmp/autono-a3-review`.
Running their unchanged `review_decisions.rs` against the reviewed HEAD reproduced
all three failures (3/3): superseded stable replay verified legacy, a failed
attempt's invented `b.txt` convention qualified as trusted support, and the
explicit stable prohibition passed Category A. The corrected versions reuse those
provider responses in the existing shell-only delegation harness. No paid
inference is used.

## Revision attribution (P1)

The old ID replay shortcut checked payload equality and skipped active-state
validation. Separately, every implementation/check/review/completion event searched
for *all currently active* decisions affecting the task. Together these paths
attributed stable's third-attempt artifact to legacy's second revision.

A replay must now match an active revision. Superseded replay fails closed before
its edits are applied. Each attempt freezes the decision IDs supplied to its coder
(`decision_context`). Response assessment may replace these only with that
attempt's own explicitly linked revisions. The daemon commits the final
`decision_ids` and `consumed` audit facts with the decision assessments before
applying edits. Journal serialization and SQLite enforce immutable bindings.

All subsequent events target the bound IDs. Successful implementation, checks,
independent review and preservation reference the same immutable candidate tree.
The fresh independent reviewer must account for every consumed ID exactly once
using `decision_reviews: [{decision_id, implemented, evidence}]`. Missing,
duplicate or different IDs fail validation under the existing bounded review
budget. `implemented: false` prevents acceptance even if feature criteria pass.
A decision's verified predicate requires the ordered chain on the same revision,
task, attempt and tree for every affected task. CLI and native predicates agree.

The exact stable → legacy → stale stable case now stops before third-attempt
edits/review. Both failed attempts and supersession remain in history; legacy has
no third-attempt implementation or verification events. A separate fixture omits
the stale proposal and implements stable while consuming legacy: its reviewer
accepts feature criteria but explicitly denies legacy implementation. The daemon
records `review_rejected`, exhausts the original attempt budget and never records
`task_verified` for either revision.

## Evidence source (P1)

The old policy read the mutable task worktree and stamped its content hash with
the original base commit. A failed attempt could therefore manufacture its own
justification.

Only original specification bytes or regular Git blobs from the frozen original
base qualify as trusted decision evidence now. The daemon resolves cited paths
with literal pathspecs, verifies exact path, regular-blob mode and size, and reads
the object directly. Every path must already be in the A2 exact allowlist; Git
ownership, deadline and cancellation checks remain in force. No new discovery or
file access is granted. Provenance explicitly identifies `specification` or
`frozen_base`; frozen evidence records the commit, blob ID and SHA-256 of those
actual immutable bytes, without claiming a mutable workspace origin.

Workspace bytes remain candidate context. A missing or changed frozen-base
citation is unresolved; the proposal retains its claim but the trusted evidence
array omits it. Failed/unreviewed artifacts cannot establish a convention. This
policy also conservatively excludes completed dependency artifacts from convention
eligibility: support for their distinct verified origin is deferred. The poisoned
`b.txt` regression retains the failed file and candidate quote, records Category B
with no trusted provenance, and applies none of the later proposed edits.

## Explicit requirements (P2)

The old lexical conflict gate treated mentioning the chosen option as permission,
even when it occurred in a prohibition. That assumption is removed.

A competing option mentioned anywhere in the original specification preserves
Category B. A chosen-option mention requires a sole, simple positive instruction
from a small explicit grammar (`Use OPTION`, `Use OPTION helper names`, or
`Use OPTION as the default export format`). Other phrasing, quotations, negative
instructions, contrasting alternatives and contradictory specifications remain B.
If neither option is mentioned, actual frozen project support can still establish
a reversible default. This is a conservative lexical gate, not a natural-language
policy engine or semantic compatibility guarantee. Original requirements and
independent review remain authoritative. Tests retain both unambiguous positive
instructions and the CSV default scenario.

## Persistence and compatibility

No historical decision or event is rewritten. The existing additive migration
0007 and append-only SQLite tables store the new consumption events atomically
with the run/task projection and lifecycle journal. Attempt fields have additive
serde defaults. New provenance source/blob fields also default safely; old origin
claims read as `legacy_unverified` and cannot be newly consumed without a
validated replacement revision. Existing historical events lacking an explicit
consumption chain remain inspectable but do not certify the stronger verified
predicate. Existing Supervised schemas and workflows retain their original
behavior; an empty/absent decision-review list is compatible when no revisions
were consumed.

## Separate A2 expiry receipt

The `Failed` expectation on authorization expiry is correct. A small separate
correction makes both expiry timers recheck the actual absolute cutoff after a
relative wake, rather than emitting a cancellation signal early. The original
receipt assertions are unchanged. See [A2 expiry receipts](m5-delegation-expiry-receipts.md)
for code semantics, deterministic early-wake coverage, real workflow receipt
capture and the limitation that the exact original mismatch was intermittent and
not reproduced in the local full run.

## Validation

The unchanged reproductions passed 3/3 before fixes. Corrected deterministic
regressions cover stale replay, explicit negative requirements, manufactured
conventions, independent per-revision rejection, immutable attempt bindings,
append-only history, SQLite reopen, and ordered same-attempt/tree status predicates.
Final macOS arm64 validation passed:

- `RUST_TEST_THREADS=2 cargo test --workspace --locked`: 138 passed, zero failed,
  two intentional exclusions (the paid-agent suite and isolated helper).
- Frontend typecheck, lint, 106 tests in eight files, production build and module
  startup smoke passed.
- `cargo fmt --all --check`, strict workspace/all-target Clippy, debug and release
  workspace builds, Rust 1.89 MSRV check and workspace rustdoc passed.
- `npm run tauri -- build --bundles app` and a packaged native smoke passed. The
  authenticated bridge showed completed run `2b934e3a-9558-4697-801e-8fe36455cc52`,
  the verified CSV ledger, frozen-base hash, four consumption chains and independent
  revision reviews. Provider invocation logs remained unchanged.
- All four A2 verification/integration cancellation/expiry receipts matched their
  unchanged expectations. Expiry receipts finished just after the recorded cutoff
  with `Failed` and `authorization deadline exceeded`.

[Recorded fixture excerpts](m5-decisions-review-evidence.json) preserve the actual
IDs, proposal/audit history, provenance, attempt bindings, checks, review verdicts,
artifact trees and expiry receipts. Full local snapshot SHA-256 values are included
for traceability. The earlier acceptance artifact is retained unchanged.
[Validation commands/results](m5-decisions-review-validation.json) record local
checks; Linux/macOS CI results are linked from [PR #6 checks](https://github.com/bernacle/autono/pull/6/checks).

An initial focused run under concurrent compilation hit the existing 60-second
fixture wait. The corrected regressions then passed in isolation and in both full
workspace runs without increasing budgets, changing assertions or weakening scope.
The first full run had 137 tests; the final run adds the absolute-cutoff regression.
The SQLite fixture now uses the explicit positive requirement `Use feature`, so it
continues testing a genuinely eligible accepted decision under the stricter policy;
negative and contradictory specifications have dedicated classification regressions.
