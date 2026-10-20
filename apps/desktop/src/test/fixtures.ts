import type { HumanAction, Run, Task } from "../lib/types";
export function task(
  id: string,
  dependencies: string[] = [],
  status: Task["status"] = "WAITING",
): Task {
  return {
    plan: {
      id,
      dependencies,
      goal: `Implement ${id}`,
      description: `Task ${id}`,
      acceptance_criteria: ["Works"],
      verification_expectations: ["Tests pass"],
      requirement_ids: ["r"],
      role: "coder",
    },
    status,
    attempts: [],
    workspace: null,
    artifact_commit: null,
  };
}
export function action(): HumanAction {
  return {
    id: "H-permission",
    status: "PENDING",
    continuable: false,
    requirement: {
      kind: "permission",
      reason: "Requires developer authorization",
      requested_action: "Approve configured checks",
      affected_tasks: ["A"],
      blocking: true,
      evidence: "Exact daemon policy",
      confidence: 100,
      permission: {
        operation: "run_checks",
        scope:
          "run 00000000-0000-4000-8000-000000000001 immutable configured verification commands",
      },
    },
    resolution: null,
    created_at: "2026-10-20T12:00:00Z",
    updated_at: "2026-10-20T12:00:00Z",
    audit: [],
  };
}
export function run(overrides: Partial<Run> = {}): Run {
  return {
    id: "00000000-0000-4000-8000-000000000001",
    revision: 7,
    status: "AWAITING_APPROVAL",
    created_at: "2026-10-20T12:00:00Z",
    updated_at: "2026-10-20T12:00:00Z",
    repository: "/tmp/fixture",
    requested_base: "main",
    base_commit: "abc12345",
    specification: "# Build a safe feature\nOriginal requirements",
    settings: {
      agents: {
        planner: { provider: "claude", model: null, timeout_seconds: 600 },
        coder: { provider: "codex", model: null, timeout_seconds: 600 },
        reviewer: { provider: "claude", model: null, timeout_seconds: 600 },
      },
      execution: { max_parallel: 2, max_attempts: 3, max_review_cycles: 2 },
      verification: {
        commands: [["test", "-f", "feature.txt"]],
        timeout_seconds: 10,
      },
      permissions: { rules: { run_checks: "REQUIRE_APPROVAL" } },
    },
    approved_at: null,
    planner: null,
    planning_workspace: {
      id: "ws",
      path: "/tmp/worktree",
      branch: "autono/plan",
      base_commit: "abc12345",
      head_commit: null,
    },
    plan: {
      requirements: [{ id: "r", text: "Feature works" }],
      milestones: [
        { id: "m", goal: "Deliver feature", task_ids: ["A", "B", "D"] },
      ],
      tasks: [],
    },
    tasks: [
      task("A", [], "COMPLETED"),
      task("B", ["A"], "BLOCKED_HUMAN"),
      task("D", ["B"]),
    ],
    preflight: {
      checked_at: "2026-10-20T12:00:00Z",
      ready: true,
      checks: [
        {
          id: "repository",
          ready: true,
          blocking: true,
          evidence: "Pinned repository accessible",
        },
        {
          id: "authentication",
          ready: null,
          blocking: false,
          evidence: "Unknown without inspecting credentials",
        },
      ],
      action_ids: [],
      limitation: "Preflight cannot discover future blockers",
    },
    human_actions: [],
    blockers: [],
    integration_workspace: null,
    integration_verification: [],
    summary: "Plan ready",
    ...overrides,
  };
}
