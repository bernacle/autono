import { checkEventOutcome, checkLabels } from "./evidence-model";
import type { EngineeringEvent, Run } from "./types";
import { parseFact, sessionBindings } from "./execution-model";
import { humanize } from "./presentation";
export type JournalFilter =
  "All" | "Agents" | "Checks" | "Reviews" | "Human" | "Problems";
export function describeEvent(e: EngineeringEvent, run: Run) {
  const f = parseFact(e, new Set(run.tasks.map((t) => t.plan.id))),
    b = f?.session ? sessionBindings(run).get(f.session) : undefined;
  const task = f?.task ?? b?.task;
  const attempt = f?.attempt ?? b?.attempt;
  const checkOutcome =
    f?.kind === "verification_finished" ? checkEventOutcome(run, f) : undefined;
  const names: Record<string, string> = {
    created: "Planning requested",
    planning_workspace_ready: "Planning worktree prepared",
    plan_ready: `Plan ready · ${run.tasks.length} tasks`,
    preflight: "Readiness inspected",
    approved: "Execution explicitly approved",
    continued: "Resolved work explicitly continued",
    scheduler: "Dependency readiness checked",
    execution_finished: "Execution checkpoint recorded",
    integration_requested: "Local integration requested",
    integration_workspace_ready: "Integration worktree prepared",
    completed: "Local integration completed",
    cancelled: "Run cancelled",
    workflow_stopped: "Workflow stopped · inspect retained evidence",
    interrupted: "Run interrupted",
    human_resolution:
      "Human resolution recorded · execution requires separate approval",
    task_started: "Task execution started",
    workspace_reserved: "Task worktree reserved",
    workspace_ready: "Task worktree prepared",
    attempt_started: `Attempt ${f?.attempt ?? ""} started`,
    attempt_feedback: "Attempt feedback retained for retry",
    review_reserved: "Independent reviewer reserved",
    review_workspace_ready: "Isolated review worktree prepared",
    review_finished:
      f?.accepted === true
        ? "Independent review accepted"
        : f?.accepted === false
          ? "Independent review rejected"
          : "Independent review result recorded",
    review_invalid_or_failed: "Independent review unavailable or invalid",
    task_completed: "Verified task artifact preserved",
    task_stopped: "Task stopped · inspect evidence",
    verification_passed: "Task checks passed",
    integration_merge: "Verified task commit merged into integration worktree",
    agent_started: `${humanize(f?.role ?? "Agent")} session started`,
    agent_process: "Agent process recorded",
    agent_finished: f?.outcome
      ? `${humanize(f.role ?? "Agent")} session ${f.outcome === "succeeded" ? "finished" : "stopped"}`
      : "Agent session result recorded",
    verification_started: "Configured checks started",
    verification_process: "Check process recorded",
    verification_finished:
      checkOutcome && checkOutcome !== "unknown"
        ? `Configured check · ${checkLabels[checkOutcome].toLowerCase()}`
        : "Configured check result recorded · outcome unavailable",
    task_state: f?.state
      ? f.state === "BLOCKED_HUMAN"
        ? "Task paused for a human decision"
        : f.state === "WAITING"
          ? "Task waiting on dependencies"
          : `Task ${humanize(f.state)}`
      : "Task state recorded",
  };
  const human =
    ["human_resolution", "continued"].includes(e.kind) ||
    f?.state === "BLOCKED_HUMAN";
  const problem =
    [
      "workflow_stopped",
      "interrupted",
      "task_stopped",
      "review_invalid_or_failed",
    ].includes(e.kind) ||
    (e.kind === "agent_finished" &&
      !!f?.outcome &&
      f.outcome !== "succeeded") ||
    (e.kind === "review_finished" && f?.accepted === false) ||
    (checkOutcome !== undefined && checkOutcome !== "passed") ||
    ["FAILED", "INTERRUPTED", "BLOCKED", "BLOCKED_HUMAN"].includes(
      f?.state ?? "",
    );
  const group: JournalFilter = problem
    ? "Problems"
    : e.kind.startsWith("agent_")
      ? "Agents"
      : e.kind.startsWith("verification_")
        ? "Checks"
        : e.kind.startsWith("review_")
          ? "Reviews"
          : e.kind === "human_resolution" || e.kind === "continued"
            ? "Human"
            : "All";
  return {
    label: names[e.kind] ?? humanize(e.kind),
    task,
    attempt,
    human,
    group,
    problem,
    quiet: [
      "scheduler",
      "task_state",
      "workspace_reserved",
      "workspace_ready",
      "agent_process",
      "verification_process",
      "review_reserved",
      "review_workspace_ready",
      "planning_workspace_ready",
    ].includes(e.kind),
  };
}
