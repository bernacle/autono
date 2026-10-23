import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PartialHandoff } from "../components/partial-handoff";
import { PreflightChecklist } from "../components/preflight";
import type { RunHandoff } from "../lib/types";
const report: RunHandoff = {
  outcome: "partially_completed",
  summary:
    "4 of 5 tasks independently verified locally. External integration verification was not performed.",
  tasks: [
    {
      task_id: "A",
      delivery: "independently_verified_locally",
      implementation_recorded: true,
      checks_passed: true,
      artifact_commit: "abc",
    },
    {
      task_id: "D",
      delivery: "blocked_awaiting_prerequisite",
      implementation_recorded: false,
      checks_passed: false,
      artifact_commit: null,
    },
  ],
  needs_you: [
    {
      action_id: "H-service",
      action_version: 1,
      category: "deferrable_manual_dependency",
      affected_tasks: ["D"],
      waiting_descendants: ["F"],
      requested_action:
        "Configure test account externally; provide no credentials",
    },
  ],
  combined_checks_passed: false,
  completion_requirements_satisfied: false,
  internal_decision_ids: [],
};
describe("A4 partial handoff", () => {
  it("shows retained local work and the exact unresolved dependency without success", () => {
    render(<PartialHandoff report={report} />);
    expect(screen.getByText("Partially completed")).toBeInTheDocument();
    expect(
      screen.getByText(/External integration verification was not performed/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/D: blocked awaiting prerequisite/),
    ).toBeInTheDocument();
    expect(screen.getByText(/waiting descendants: F/)).toBeInTheDocument();
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
  });
  it("supports legacy runs without a new receipt", () => {
    const { container } = render(<PartialHandoff />);
    expect(container).toBeEmptyDOMElement();
  });
  it("distinguishes full readiness from task-local independent readiness", () => {
    render(
      <PreflightChecklist
        report={{
          ready: false,
          independent_work_ready: true,
          checked_at: "2026-10-22T00:00:00Z",
          checks: [],
          action_ids: ["H-service"],
          limitation: "External access still required",
        }}
      />,
    );
    expect(
      screen.getByText("Independent authorized work can proceed"),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Ready for explicit approval"),
    ).not.toBeInTheDocument();
  });
});
