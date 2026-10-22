import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { decisionVerified } from "../lib/decision-state";
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
function verifyTask(
  d: DecisionRecord,
  task: string,
  attempt: number,
  tree: string,
) {
  for (const kind of [
    "consumed",
    "implemented",
    "checks_passed",
    "review_accepted",
    "task_verified",
  ]) {
    d.audit.push({
      sequence: d.audit.length + 1,
      at: "2026-10-22",
      kind,
      task_id: task,
      attempt,
      detail: tree,
    });
  }
}
describe("decision inspection", () => {
  it("keeps old supervised views empty", () => {
    const { container } = render(<Decisions />);
    expect(container).toBeEmptyDOMElement();
  });
  it("requires independently preserved artifacts for every affected task", () => {
    const d = decision();
    verifyTask(d, "export", 1, "tree-a");
    const { rerender } = render(<Decisions ledger={[d]} />);
    expect(screen.getByText(/Verification incomplete/)).toBeInTheDocument();
    expect(screen.getByText(/JSON/)).toBeInTheDocument();
    expect(screen.getByText(/source-digest/)).toBeInTheDocument();
    verifyTask(d, "tests", 1, "tree-b");
    rerender(<Decisions ledger={[d]} />);
    expect(screen.getByText(/Independently verified/)).toBeInTheDocument();
    d.audit.push({
      sequence: d.audit.length + 1,
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
  it("requires an ordered chain on the same attempt and implementation tree", () => {
    const d = decision();
    verifyTask(d, "export", 2, "tree-a");
    verifyTask(d, "tests", 1, "tree-b");
    expect(decisionVerified(d)).toBe(true);
    const check = d.audit.find((e) => e.kind === "checks_passed")!;
    check.attempt = 1;
    expect(decisionVerified(d)).toBe(false);
    check.attempt = 2;
    check.detail = "other-tree";
    expect(decisionVerified(d)).toBe(false);
    check.detail = "tree-a";
    check.sequence = 1;
    expect(decisionVerified(d)).toBe(false);
  });
  it("does not certify historical task_verified events without revision consumption", () => {
    const d = decision();
    verifyTask(d, "export", 1, "tree-a");
    verifyTask(d, "tests", 1, "tree-b");
    d.audit = d.audit.filter((e) => e.kind !== "consumed");
    expect(decisionVerified(d)).toBe(false);
    render(<Decisions ledger={[d]} />);
    expect(screen.getByText(/Verification incomplete/)).toBeInTheDocument();
  });
});
