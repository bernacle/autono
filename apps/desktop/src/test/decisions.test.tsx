import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Decisions } from "../components/decisions";
import type { DecisionRecord } from "../lib/types";

function decision(): DecisionRecord {
  return {
    id: "D-csv",
    policy_basis: "Exact A2 allowlist; no new permissions",
    proposal: {
      question: "Which default export format?",
      chosen: "CSV",
      rationale: "Reuse the project convention.",
      alternatives: ["JSON"],
      affected_tasks: ["export", "tests"],
      assumptions: [],
      uncertainty: [],
    },
    evidence: [
      {
        file: "format.sh",
        content_sha256: "source-digest",
        excerpt: "Default export format is CSV.",
      },
    ],
    audit: [
      {
        sequence: 1,
        at: "2026-10-22",
        kind: "accepted",
        task_id: null,
        attempt: null,
        detail: "Local reversible choice",
      },
    ],
  };
}
describe("decision inspection", () => {
  it("keeps old supervised views empty", () => {
    const { container } = render(<Decisions />);
    expect(container).toBeEmptyDOMElement();
  });
  it("requires independently preserved artifacts for every affected task", () => {
    const d = decision();
    d.audit.push({
      sequence: 2,
      at: "2026-10-22",
      kind: "task_verified",
      task_id: "export",
      attempt: 1,
      detail: "tree-a",
    });
    const { rerender } = render(<Decisions ledger={[d]} />);
    expect(screen.getByText(/Verification incomplete/)).toBeInTheDocument();
    expect(screen.getByText(/JSON/)).toBeInTheDocument();
    expect(screen.getByText(/source-digest/)).toBeInTheDocument();
    d.audit.push({
      sequence: 3,
      at: "2026-10-22",
      kind: "task_verified",
      task_id: "tests",
      attempt: 1,
      detail: "tree-b",
    });
    rerender(<Decisions ledger={[d]} />);
    expect(screen.getByText(/Independently verified/)).toBeInTheDocument();
    d.audit.push({
      sequence: 4,
      at: "2026-10-22",
      kind: "superseded",
      task_id: null,
      attempt: null,
      detail: "D-next",
    });
    rerender(<Decisions ledger={[d]} />);
    expect(screen.getByText(/Superseded/)).toBeInTheDocument();
  });
  it("never presents an unresolved proposal as accepted", () => {
    const d = decision();
    d.audit[0].kind = "unresolved";
    render(<Decisions ledger={[d]} />);
    expect(screen.getByText(/Human action required/)).toBeInTheDocument();
  });
});
