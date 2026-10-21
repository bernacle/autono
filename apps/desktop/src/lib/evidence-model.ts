import type { Review, Run, Task, TaskPlan, Verification } from "./types";

export type Attempt = Task["attempts"][number];
/** Mirrors ReviewResult::accepts(), including a whitespace-only blocker. */
export function reviewAccepts(review: Review) {
  return (
    review.accepted &&
    review.blocker.length === 0 &&
    review.criteria.every((c) => c.satisfied) &&
    !review.findings.some((f) =>
      ["critical", "high", "medium"].includes(f.severity),
    )
  );
}
export function reviewRecords(attempt: Attempt) {
  return attempt.reviews.length
    ? attempt.reviews.map((r, index) => ({ ...r, ordinal: index + 1 }))
    : attempt.review || attempt.reviewer
      ? [
          {
            ordinal: 1,
            session: attempt.reviewer,
            review: attempt.review,
            diagnostic: null,
          },
        ]
      : [];
}
/** ID-based reviews never fall back to labels, even when an ID is missing/wrong. */
export function criterionEvidence(
  review: Review | null | undefined,
  task: TaskPlan,
  index: number,
) {
  if (!review) return undefined;
  const legacy = review.criteria.every((c) => !c.criterion_id);
  if (
    legacy &&
    task.acceptance_criteria.filter(
      (c) => c === task.acceptance_criteria[index],
    ).length !== 1
  )
    return undefined;
  const matches = review.criteria.filter((c) =>
    legacy
      ? c.criterion === task.acceptance_criteria[index]
      : c.criterion_id === `${task.id}-C${index + 1}`,
  );
  return matches.length === 1 ? matches[0] : undefined;
}
export type CheckOutcome =
  | "passed"
  | "test_failure"
  | "timeout"
  | "cancelled"
  | "infrastructure_failure"
  | "unknown";
export const checkLabels: Record<CheckOutcome, string> = {
  passed: "Passed",
  test_failure: "Test failed",
  timeout: "Timed out",
  cancelled: "Cancelled",
  infrastructure_failure: "Infrastructure failure",
  unknown: "Outcome unavailable",
};
/** False alone supplies no test verdict. Legacy successful records retain store evidence checks. */
export function verificationOutcome(result: Verification): CheckOutcome {
  if (result.outcome === "passed" || result.outcome === null) {
    return result.passed && result.exit_code === 0 && result.output_complete
      ? "passed"
      : "unknown";
  }
  if (result.passed) return "unknown";
  if (result.outcome === "test_failure")
    return result.exit_code !== null && result.output_complete
      ? "test_failure"
      : "unknown";
  if (
    ["timeout", "cancelled", "infrastructure_failure"].includes(result.outcome)
  )
    return result.outcome as CheckOutcome;
  return "unknown";
}
export function verificationRecords(run: Run) {
  return [
    ...run.tasks.flatMap((t) =>
      t.attempts.flatMap((a) =>
        a.verification.map((result) => ({
          task: t.plan.id as string | undefined,
          attempt: a.number as number | undefined,
          result,
        })),
      ),
    ),
    ...run.integration_verification.map((result) => ({
      task: undefined,
      attempt: undefined,
      result,
    })),
  ];
}
export function checkEventOutcome(
  run: Run,
  event: {
    process?: string;
    task?: string;
    attempt?: number;
    key?: string;
    accepted?: boolean;
  },
): CheckOutcome {
  const matches = verificationRecords(run).filter(
    (v) => v.result.process_run_id === event.process,
  );
  // passed=true is the daemon's successful process verdict; false has several meanings.
  if (!matches.length) return event.accepted === true ? "passed" : "unknown";
  if (matches.length !== 1) return "unknown";
  const v = matches[0];
  if (
    v.task !== event.task ||
    v.attempt !== event.attempt ||
    (event.key && event.key !== v.result.process_key) ||
    (event.accepted !== undefined && v.result.passed !== event.accepted)
  )
    return "unknown";
  return verificationOutcome(v.result);
}
export function verificationCounts(results: Verification[]) {
  const counts: Record<CheckOutcome, number> = {
    passed: 0,
    test_failure: 0,
    timeout: 0,
    cancelled: 0,
    infrastructure_failure: 0,
    unknown: 0,
  };
  for (const result of results) counts[verificationOutcome(result)]++;
  return counts;
}
export interface CheckSummary {
  state: CheckOutcome | "pending" | "partial" | "unavailable";
  complete: boolean;
  recorded: number;
  expected: number;
  label: string;
}
export function summarizeChecks(
  results: Verification[],
  commands: string[][],
  pending: boolean,
): CheckSummary {
  const base = {
    complete: false,
    recorded: results.length,
    expected: commands.length,
  };
  const unavailable = {
    ...base,
    state: "unavailable" as const,
    label: "Checks unavailable",
  };
  // The store compares exact cardinality and ordered argv, not a set of command names.
  if (
    !commands.length ||
    commands.some((command) =>
      command.some((arg) => arg.includes("[redacted]") || arg === "***"),
    ) ||
    results.length > commands.length ||
    results.some(
      (v, i) => JSON.stringify(v.command) !== JSON.stringify(commands[i]),
    ) ||
    new Set(results.map((v) => v.process_run_id)).size !== results.length ||
    new Set(results.map((v) => v.process_key)).size !== results.length
  )
    return unavailable;
  if (!results.length)
    return pending
      ? {
          ...base,
          state: "pending",
          label: `Checks pending (0/${commands.length})`,
        }
      : unavailable;
  const outcomes = results.map(verificationOutcome),
    problems = [
      ...new Set(outcomes.filter((o) => o !== "passed" && o !== "unknown")),
    ],
    complete = results.length === commands.length;
  if (outcomes.includes("unknown"))
    return { ...unavailable, label: "Check outcome unavailable" };
  if (problems.length)
    return {
      ...base,
      complete,
      state: problems[0],
      label: problems.map((o) => checkLabels[o]).join(" · "),
    };
  if (!complete)
    return {
      ...base,
      state: "partial",
      label: `Checks ${pending ? "partial" : "incomplete"} (${results.length}/${commands.length})`,
    };
  return { ...base, complete: true, state: "passed", label: "Checks passed" };
}
export function attemptChecks(run: Run, task: Task, attempt: Attempt) {
  return summarizeChecks(
    attempt.verification,
    run.settings.verification.commands,
    task.status === "RUNNING" &&
      task.attempts.at(-1)?.number === attempt.number &&
      !["COMPLETED", "FAILED", "CANCELLED", "INTERRUPTED"].includes(run.status),
  );
}
