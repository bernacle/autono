import type { Run, RunAction, Task } from "./types";
import { can, pending } from "./utils";

export const humanize = (value: string) =>
  value.replaceAll("_", " ").toLowerCase();
export const basename = (path: string) =>
  path.split("/").filter(Boolean).at(-1) ?? path;
export function duration(seconds: number) {
  const s = Math.max(0, Math.floor(seconds));
  return s >= 3600
    ? `${Math.floor(s / 3600)}:${String(Math.floor(s / 60) % 60).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`
    : `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}
export function clock(value: string | number) {
  return new Date(value).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}
export const actionLabels: Record<RunAction, string> = {
  execute: "Approve execution…",
  continue: "Continue…",
  integrate: "Integrate…",
  cancel: "Cancel run…",
  preflight: "Refresh preflight",
};
export function primaryAction(run: Run): RunAction | null {
  for (const action of ["execute", "continue", "integrate"] as const)
    if (can(run, action)) return action;
  if (run.status === "AWAITING_APPROVAL") return "execute";
  if (run.status === "BLOCKED")
    return !run.approved_at
      ? "execute"
      : run.integration_workspace
        ? "integrate"
        : "continue";
  return null;
}
export function actionReason(run: Run, action: RunAction, connected: boolean) {
  if (!connected) return "Reconnect to a healthy daemon before taking action.";
  if (can(run, action)) return "";
  if (action === "execute") {
    if (run.human_actions.some((a) => pending(a) && a.requirement.blocking))
      return "Resolve blocking human actions first.";
    if (!run.preflight?.ready) return "Preflight prerequisites need attention.";
    return "The daemon must return this run to awaiting approval.";
  }
  if (action === "continue")
    return "Inspect and resolve eligible runtime checkpoints before continuing.";
  if (action === "integrate")
    return "The run must be verified before integration.";
  return "This action is unavailable in the current run state.";
}
export function lifecycleIndex(run: Run) {
  if (run.status === "COMPLETED") return 5;
  if (
    run.status === "INTEGRATING" ||
    (run.status === "BLOCKED" && run.integration_workspace)
  )
    return 4;
  if (run.status === "VERIFIED") return 3;
  if (run.approved_at) return 2;
  if (run.status === "AWAITING_APPROVAL" || run.status === "BLOCKED") return 1;
  return 0;
}
const blockerLabels: Record<string, string> = {
  human_action: "Needs a human decision",
  effect_reconciliation: "Confirm what the last invocation did",
  review_blocker: "Reviewer raised a blocker",
  integration_conflict: "Merge conflict during integration",
  dependency_merge_conflict: "Conflict merging dependency commits",
  verification_infrastructure: "Checks could not run",
  task_failure: "Task failed",
  git_ownership_changed: "Repository changed outside Autono",
};
export const blockerLabel = (kind: string) =>
  blockerLabels[kind] ?? humanize(kind);
export function taskDepths(tasks: Task[]) {
  const depths = new Map<string, number>();
  function depth(id: string, path = new Set<string>()): number {
    if (depths.has(id)) return depths.get(id)!;
    if (path.has(id)) return 0;
    const task = tasks.find((t) => t.plan.id === id);
    if (!task) return 0;
    path.add(id);
    const value = task.plan.dependencies.length
      ? 1 +
        Math.max(...task.plan.dependencies.map((d) => depth(d, new Set(path))))
      : 0;
    depths.set(id, value);
    return value;
  }
  tasks.forEach((t) => depth(t.plan.id));
  return depths;
}
export function orderedTasks(run: Run) {
  const depths = taskDepths(run.tasks);
  const milestone = (id: string) => {
    const index =
      run.plan?.milestones.findIndex((m) => m.task_ids.includes(id)) ?? -1;
    return index < 0 ? Number.MAX_SAFE_INTEGER : index;
  };
  return [...run.tasks].sort(
    (a, b) =>
      milestone(a.plan.id) - milestone(b.plan.id) ||
      (depths.get(a.plan.id) ?? 0) - (depths.get(b.plan.id) ?? 0) ||
      a.plan.id.localeCompare(b.plan.id),
  );
}
export function latestReview(task: Task) {
  const attempt = task.attempts.at(-1);
  return attempt?.reviews.length
    ? attempt.reviews.at(-1)?.review
    : attempt?.review;
}
