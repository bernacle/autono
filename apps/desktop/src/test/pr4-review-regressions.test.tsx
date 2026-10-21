import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import captured from "./fixtures/completed-journal.json";
import blockerCases from "./fixtures/review-blocker-cases.json";
import { reviewAccepts } from "../lib/evidence-model";
import type { Run, EngineeringEvent } from "../lib/types";
import { appendHistory, deriveExecution } from "../lib/execution-model";
import { TaskEvidence } from "../components/evidence";
import { TasksView, TaskInspector } from "../components/tasks";
import { VerificationLedger } from "../components/execution";

function fixture() {
  return structuredClone(captured) as unknown as {
    run: Run;
    events: EngineeringEvent[];
  };
}
function model(run: Run, events: EngineeringEvent[]) {
  return deriveExecution(
    run,
    appendHistory(
      undefined,
      run.id,
      new Set(run.tasks.map((t) => t.plan.id)),
      events,
      1000,
    ),
  );
}
function html(node: ReactNode) {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      {node}
    </QueryClientProvider>,
  );
}
describe("PR 4 adversarial regressions", () => {
  it.each(blockerCases)(
    "preserves the bridge verdict in every review presentation: $name",
    (testCase) => {
      const { run } = fixture(),
        task = run.tasks[0],
        attempt = task.attempts[0];
      const review = {
        ...attempt.reviews[0].review!,
        accepted: true,
        findings: [],
        blocker: testCase.projected,
      };
      expect(review.criteria.every((c) => c.satisfied)).toBe(true);
      expect(reviewAccepts({ ...review, blocker: testCase.original })).toBe(
        testCase.accepts,
      );
      expect(reviewAccepts(review)).toBe(testCase.accepts);
      attempt.review = review;
      attempt.reviews[0].review = review;
      run.tasks = [task];
      for (const view of [
        <TaskEvidence run={run} onProcess={vi.fn()} />,
        <TaskInspector
          task={task}
          run={run}
          onClose={vi.fn()}
          onProcess={vi.fn()}
          onHuman={vi.fn()}
        />,
      ]) {
        const container = document.createElement("div");
        container.innerHTML = html(view);
        const badge = container.querySelector(".review .badge")!;
        expect(badge.textContent).toBe(
          testCase.accepts ? "Review accepted" : "Rejected",
        );
        expect(badge.querySelector(".glyph-success") !== null).toBe(
          testCase.accepts,
        );
        expect(container.textContent).toContain(
          "Fixture artifact and deterministic check inspected",
        );
        expect(container.textContent).not.toContain("\u001b");
      }
      const container = document.createElement("div");
      container.innerHTML = html(
        <TasksView
          run={run}
          onProcess={vi.fn()}
          onHuman={vi.fn()}
          onExecution={vi.fn()}
        />,
      );
      expect(
        container.querySelector(
          '[data-task="A"] [title="Review"] .success-text',
        ) !== null,
      ).toBe(testCase.accepts);
      expect(
        container.querySelector(
          '[data-task="A"] [title="Review"] .danger-text',
        ) !== null,
      ).toBe(!testCase.accepts);
    },
  );
  it("does not label a provider accepted review with a blocking finding as accepted", () => {
    const { run } = fixture();
    const task = run.tasks[0],
      a = task.attempts[0];
    const review = {
      ...a.reviews[0].review!,
      accepted: true,
      findings: [
        {
          severity: "medium",
          file: "a.rs",
          line: 1,
          explanation: "Blocking defect",
          recommended_fix: "Fix it",
          confidence: 1,
        },
      ],
    };
    a.review = review;
    a.reviews[0].review = review;
    run.tasks = [task];
    task.status = "FAILED";
    run.status = "FAILED";
    expect(html(<TaskEvidence run={run} onProcess={vi.fn()} />)).not.toContain(
      "Review accepted",
    );
  });
  it("uses effective verdicts and complete check sets in the actual task-row indicators", () => {
    const { run } = fixture(),
      task = run.tasks[0],
      a = task.attempts[0];
    run.tasks = [task];
    task.status = "RUNNING";
    run.status = "RUNNING";
    a.reviews[0].review!.accepted = true;
    a.reviews[0].review!.blocker = "Needs a decision";
    run.settings.verification.commands.push(["second-check"]);
    const container = document.createElement("div");
    container.innerHTML = html(
      <TasksView
        run={run}
        onProcess={vi.fn()}
        onHuman={vi.fn()}
        onExecution={vi.fn()}
      />,
    );
    const row = container.querySelector('[data-task="A"]')!;
    expect(row.querySelector('[title="Checks"] .success-text')).toBeNull();
    expect(
      row.querySelector('[title="Checks"] [aria-label="Checks partial (1/2)"]'),
    ).not.toBeNull();
    expect(row.querySelector('[title="Review"] .success-text')).toBeNull();
    expect(row.querySelector('[title="Review"] .danger-text')).not.toBeNull();
  });
  it("joins paraphrased criterion evidence by its stable criterion id", () => {
    const { run } = fixture();
    const task = run.tasks[0],
      a = task.attempts[0];
    a.reviews[0].review!.criteria[0] = {
      criterion_id: `${task.plan.id}-C1`,
      criterion: "A valid paraphrase",
      satisfied: true,
      evidence: "Unique stable criterion evidence",
    };
    const rendered = html(
      <TaskInspector
        task={task}
        run={run}
        onClose={vi.fn()}
        onProcess={vi.fn()}
        onHuman={vi.fn()}
      />,
    );
    const list = rendered.match(
      /<ul class="criteria-list">([\s\S]*?)<\/ul>/,
    )![1];
    expect(list).toContain("Unique stable criterion evidence");
  });
  it("does not claim aggregate check success with one of two commands still pending", () => {
    const { run } = fixture();
    const task = run.tasks[0];
    run.tasks = [task];
    run.status = "RUNNING";
    task.status = "RUNNING";
    run.settings.verification.commands.push(["second-check"]);
    task.attempts[0].verified_tree = null;
    task.attempts[0].preserved_tree = null;
    expect(
      html(
        <VerificationLedger run={run} onTask={vi.fn()} onProcess={vi.fn()} />,
      ),
    ).not.toContain("✓ Checks passed");
  });
  it("does not turn a timed-out check into a failed test verdict", () => {
    const { run, events } = fixture();
    const v = run.tasks[0].attempts[0].verification[0];
    v.passed = false;
    v.outcome = "timeout";
    const finish = events.find(
      (e) =>
        e.kind === "verification_finished" &&
        e.detail.includes(v.process_run_id),
    )!;
    finish.detail = finish.detail.replace("passed=true", "passed=false");
    expect(
      model(run, events).segments.find(
        (s) => s.process === v.process_run_id && s.phase === "checks",
      )!.outcome,
    ).not.toBe("failed");
  });
  it("keeps a successful reviewer session neutral until the validated review verdict arrives", () => {
    const { run, events } = fixture();
    const session = run.tasks[0].attempts[0].reviews[0].session;
    const end = events.find(
      (e) => e.kind === "agent_finished" && e.detail.includes(session.id),
    )!;
    run.status = "RUNNING";
    run.tasks[0].status = "RUNNING";
    run.tasks[0].attempts[0].reviews[0].review = null;
    run.tasks[0].attempts[0].review = null;
    const m = model(
      run,
      events.filter((e) => e.sequence <= end.sequence),
    );
    expect(m.segments.find((s) => s.session === session.id)!.outcome).toBe(
      "recorded",
    );
  });
});
