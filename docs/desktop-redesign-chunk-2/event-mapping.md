# Execution event mapping and boundaries

The pure model in `apps/desktop/src/lib/execution-model.ts` is pinned to the current engineering store, engine and service implementations. Event detail strings are implementation conventions, not a newly declared protocol. No daemon orchestration, schema or endpoint changes are required.

| Recorded input | Timeline fact |
|---|---|
| `created`, `plan_ready`, `approved` | Planning, awaiting approval and executing run bands |
| `continued`, `execution_finished` | Further execution epochs; active continuation does not split an epoch, and a no-op execution checkpoint does not close it |
| Consecutive `task_state` events | Waiting, ready, running container, human pause and blocked intervals |
| `attempt_started` | Attempt identity and retry markers; no guessed attempt duration |
| `agent_started` / `agent_finished` | Coding or independent review interval, joined by session UUID to the run snapshot, never by parallel role alone |
| `agent_process` | Process reference only when the snapshot joins that process to the same session |
| `verification_started` / `verification_process` / `verification_finished` | Check interval and result, keyed by process key / process UUID with the recorded target and attempt; ambiguous joins are refused |
| `review_finished`, `review_invalid_or_failed` | Overall independent review verdict on the latest recorded attempt, including invalid reviews that retry without another coding attempt |
| `task_completed` | Verified artifact preservation marker; agent exit alone is never task completion |
| Human-action creation and audit | Decision markers from authoritative snapshot timestamps |
| `integration_requested`, `integration_merge`, `completed` | Integration band, recorded merges and local completion |
| Explicit cancellation / interruption / workflow stop | End of the execution epoch and observed in-flight phase; retained stop evidence remains inspectable |

Task-state rows follow the transaction's primary event in the store. A transaction can append several state rows. Each row retains that primary cause instead of accidentally attributing the second state change to the first state row.

The loader starts at zero, requests pages of 100 and follows from the last confirmed sequence. Sequence numbers are global: gaps are normal and do not create synthetic time or intervals. Replayed rows are deduplicated. Normalized timeline facts stop at 5,000 events with a visible limit notice. Only the latest 500 raw journal records remain cached; the UI initially renders 50 and can reveal older retained records. Inspecting a phase source can fetch that exact sanitized event using the existing cursor endpoint, even when it has left the journal tail.

A matching start without an end is live only after catch-up and snapshot corroboration. A reserved session without a recorded start is not an active agent. Unknown kinds, redacted identifiers, malformed detail strings and unmatched UUIDs create no inferred bars. Missing task-state boundaries are not filled from terminal snapshots. Backwards timestamps are not repaired into positive durations.

All displayed times are journal record times, with JavaScript millisecond precision. An agent session's recorded start can precede actual process start by admission time. The bridge additionally projects existing process-event timestamps onto sanitized complete output lines; it does not project environments, argv or raw provider result objects. Existing redaction is unchanged.

Three sanitized journal fixtures pin actual behavior: `completed-journal.json` captures Chunk 1's 155-event successful native run; `observability-journal.json` captures Chunk 2's 184-event run with a failed check, rejected review, invalid review retry on the same coding attempt, B's human pause, second execution epoch and completed local integration; `failed-journal.json` captures three failed verification attempts and a real global sequence gap from 8 to 19. Session result objects are removed from these fixtures to match the bridge's explicit projection. Hand-built tests additionally cover malformed/redacted identifiers, ambiguous process joins, interruptions, repeated events, truncation and backwards timestamps.

Each configured verification command retains its own sourced check interval and outcome instead of merging multiple commands into one opaque interval. The stat strip includes integration checks. Unknown result formats retain a neutral “result recorded” journal label; parsing failure never implies a failed or accepted verdict.
