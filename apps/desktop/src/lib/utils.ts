import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";
import type { HumanAction, Run, RunAction, Task } from "./types";
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
export function title(run: Run) {
  return (
    run.specification
      .split("\n")
      .find((s) => s.trim())
      ?.replace(/^#+\s*/, "")
      .slice(0, 100) || run.id
  );
}
export function short(id: string) {
  return id.slice(0, 8);
}
export function date(value: string) {
  return new Date(value).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
export function pending(action: HumanAction) {
  return action.status !== "RESOLVED";
}
export function canResolve(run: Run) {
  return ["AWAITING_APPROVAL", "RUNNING", "BLOCKED"].includes(run.status);
}
// Presentation eligibility only. The Rust bridge rechecks revision; the daemon decides admission.
export function can(run: Run, action: RunAction) {
  switch (action) {
    case "execute":
      return (
        !run.delegation &&
        run.status === "AWAITING_APPROVAL" &&
        run.preflight?.ready === true &&
        !run.human_actions.some((a) => a.requirement.blocking && pending(a))
      );
    case "preflight":
      return (
        ["AWAITING_APPROVAL", "BLOCKED"].includes(run.status) && !!run.plan
      );
    case "integrate":
      return !run.delegation && run.status === "VERIFIED";
    case "cancel":
      return [
        "PLANNING",
        "AWAITING_APPROVAL",
        "RUNNING",
        "VERIFIED",
        "INTEGRATING",
        "BLOCKED",
      ].includes(run.status);
    case "continue":
      return (
        !run.delegation &&
        ["RUNNING", "BLOCKED"].includes(run.status) &&
        !!run.approved_at &&
        run.tasks.some((t) => {
          const actions = run.human_actions.filter(
            (a) =>
              a.requirement.kind === "runtime" &&
              a.requirement.affected_tasks.includes(t.plan.id),
          );
          return (
            t.status === "BLOCKED_HUMAN" &&
            t.attempts.length < run.settings.execution.max_attempts &&
            actions.length > 0 &&
            actions.every(
              (a) =>
                a.continuable &&
                !pending(a) &&
                a.resolution &&
                a.resolution.effect !== "unknown",
            )
          );
        })
      );
  }
}
export function canApprove(run: Run, action: HumanAction) {
  const permission = action.requirement.permission;
  if (
    !permission ||
    run.settings.permissions.rules[permission.operation] !== "REQUIRE_APPROVAL"
  )
    return false;
  return (
    (permission.operation === "workspace_write" &&
      permission.scope === `run ${run.id} assigned task worktrees`) ||
    (permission.operation === "run_checks" &&
      permission.scope ===
        `run ${run.id} immutable configured verification commands`)
  );
}
export function blockedAncestors(
  task: Task,
  tasks: Task[],
  visited = new Set<string>(),
): string[] {
  const found = new Set<string>();
  for (const id of task.plan.dependencies) {
    if (visited.has(id)) continue;
    visited.add(id);
    const dependency = tasks.find((t) => t.plan.id === id);
    if (!dependency) {
      found.add(`${id} (missing)`);
      continue;
    }
    if (
      [
        "BLOCKED",
        "BLOCKED_HUMAN",
        "FAILED",
        "INTERRUPTED",
        "CANCELLED",
      ].includes(dependency.status)
    )
      found.add(id);
    for (const ancestor of blockedAncestors(dependency, tasks, visited))
      found.add(ancestor);
  }
  return [...found];
}
export function errorMessage(error: unknown) {
  if (typeof error === "object" && error && "message" in error)
    return String(error.message);
  return typeof error === "string"
    ? error
    : "The request failed. Refresh daemon state and try again.";
}
export interface CursorPage<T> {
  items: T[];
  cursor: number;
  hasMore: boolean;
}
export function appendEvents<T extends { sequence: number }>(
  previous: CursorPage<T> | undefined,
  incoming: T[],
): CursorPage<T> {
  const cursor = previous?.cursor ?? 0;
  const fresh = incoming
    .filter((e) => e.sequence > cursor)
    .sort((a, b) => a.sequence - b.sequence);
  const unique = [...new Map(fresh.map((e) => [e.sequence, e])).values()];
  return {
    items: [...(previous?.items ?? []), ...unique].slice(-500),
    cursor: unique.at(-1)?.sequence ?? cursor,
    hasMore: incoming.length === 100,
  };
}
