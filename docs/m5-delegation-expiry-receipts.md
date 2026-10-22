# A2 expiry receipts, investigated during A3 review fixes

The independent review recorded `Cancelled` where
`verification_and_local_integration_observe_cancellation_and_expiry` expected
`Failed`, on both A2 base `437acc4` and reviewed A3 `b10c966`. This is separate from
the three decision-policy defects. The original test expectation is retained.

## Authoritative semantics

[Supervisor](../crates/autono-core/src/supervisor.rs) owns process observation,
cleanup and immutable terminal receipts. Its prelaunch gate and running-process
classification distinguish:

| Observed boundary | Receipt | Reason |
| --- | --- | --- |
| Absolute authorization deadline reached | `Failed` | authorization deadline exceeded |
| User/workflow cancellation before expiry | `Cancelled` | cancelled by user / stopped before launch |
| Daemon shutdown | `Interrupted` | daemon shut down / stopped before launch |
| Direct child completion observed first | observed success/failure | process exit |

The engineering run has a separate state: expired delegation is `Blocked` for
inspection/new authority, user cancellation is `Cancelled`, and shutdown is
`Interrupted`. A later run state does not retrospectively change an earlier
process observation. Completed receipts and matching-key replay remain immutable.

The test is therefore correct to expect `Failed` for its still-running verification
or integration command stopped by expiry. Changing that assertion to `Cancelled`
would hide a wrong stop-reason attribution.

## Small correction

Both the supervisor deadline future and the engineering service's expiry watcher
previously converted the absolute cutoff to a single relative sleep. The service
then used a cancellation token, while the supervisor re-read UTC to determine
whether the signal meant expiry or cancellation. A relative wake before the
absolute clock reaches the cutoff (including a wall-clock adjustment) could
produce cancellation before the expiry guard became true.

A shared internal waiter now rechecks the unchanged absolute cutoff after every
wake. The service watcher and supervisor both use it. The deterministic unit test
`an_early_relative_wake_does_not_signal_absolute_expiry` supplies a clock still
before the cutoff after the first relative wake, then reaching it after the second.
It requires three clock observations; a one-shot implementation fails that check.
This is a private test seam, not a new configurable clock or authority source.

The immutable `expires_at`, stage/process admission checks, pre-spawn check,
cancellation signals, cleanup ownership, receipt classification and replay behavior
are unchanged. No process/effect retry or renewed authority is added.

## Evidence and limits

The first full local workspace run passed all four original combinations before
this timer correction, without changing their expected receipts. An isolated run
under concurrent compilation hit the fixture's wait timeout rather than the
reported receipt mismatch. Thus the exact reviewer `Cancelled` receipt was not
reproduced locally, and the available review logs do not establish that an early
relative wake was its sole cause. This correction closes the identified timer
edge; the independent reviewer should rerun the original failing environment.

Final validation captures the four real workflow receipts through
`AUTONO_EXPIRY_EVIDENCE`, including their absolute expiry, actual started/finished
timestamps, stop reason and run state. The existing assertions still require
`Failed` on expiry, `Cancelled` on explicit cancellation, no launch after cutoff,
retained completed tasks and no replay. Admission-contention, running-control,
shutdown and immutable-replay fixtures also remain enabled.

## Separate CI observation race

At `0acb23e`, all four pull-request jobs passed, while the duplicate push workflow's
macOS job failed in the CLI delegation fixture. This was a different assertion:
the preflight-expiry run summary still said `Plan validated` instead of containing
the durable runtime-budget reason. It was not a `Cancelled` process receipt.
See [the failed push run](https://github.com/bernacle/autono/actions/runs/39828896897)
and [the passing PR run](https://github.com/bernacle/autono/actions/runs/39828906599).

The fixture's `drained` helper read the run snapshot *before* observing controller
idle. `EngineeringService::run_owned` persists `record_failure` before clearing
`registry.active`, but the HTTP error response can reach the caller before that
publication finishes. A snapshot of the earlier `AwaitingApproval` checkpoint
followed by an idle health response could therefore return the stale snapshot.

The test helper now observes idle first, then reads the authoritative run. It also
surfaces a reported persistence error. A deterministic HTTP fixture publishes the
new failure checkpoint immediately before its idle response; the old helper
returns revision 1 and fails, while the corrected helper returns revision 2 and
the precise diagnostic. The real CLI fixture retains its original runtime budget,
expiry timing, summary assertion, exact preflight comparison, no-second-process
assertion and no-task-launch assertion. No runtime authorization or process code
changes for this test synchronization correction.
