import { describe, expect, it } from "vitest";
import completed from "../test/fixtures/completed-journal.json";
import observable from "../test/fixtures/observability-journal.json";
import timeout from "../test/fixtures/timeout-journal.json";
import type { EngineeringEvent, Review, Run, Verification } from "./types";
import { appendHistory, deriveExecution } from "./execution-model";
import { describeEvent } from "./journal";
import {
  attemptChecks,
  checkEventOutcome,
  checkLabels,
  criterionEvidence,
  reviewAccepts,
  reviewRecords,
  summarizeChecks,
  verificationCounts,
  verificationOutcome,
  type CheckOutcome,
} from "./evidence-model";
const fixture = () => structuredClone(completed.run) as unknown as Run;
const review = () => fixture().tasks[0].attempts[0].reviews[0].review!;
const check = () => fixture().tasks[0].attempts[0].verification[0];
function result(outcome: CheckOutcome): Verification {
  return {
    ...check(),
    outcome: outcome === "unknown" ? null : outcome,
    passed: outcome === "passed",
    exit_code: outcome === "passed" ? 0 : outcome === "test_failure" ? 1 : null,
    output_complete: outcome === "passed" || outcome === "test_failure",
  };
}
describe("effective review evidence (Rust ReviewResult semantics)", () => {
  it.each(["critical", "high", "medium"])(
    "rejects provider acceptance with a %s finding",
    (severity) => {
      const r = review();
      r.findings = [
        {
          severity,
          file: "a.rs",
          line: 1,
          explanation: "Defect",
          recommended_fix: "Repair",
          confidence: 1,
        },
      ];
      expect(reviewAccepts(r)).toBe(false);
    },
  );
  it.each(["low", "info"])(
    "allows satisfied acceptance with a %s advisory",
    (severity) => {
      const r = review();
      r.findings = [
        {
          severity,
          file: "a.rs",
          line: 1,
          explanation: "Advisory",
          recommended_fix: "Consider",
          confidence: 1,
        },
      ];
      expect(reviewAccepts(r)).toBe(true);
    },
  );
  it("requires the flag, every criterion and an exactly empty blocker", () => {
    const r = review();
    expect(reviewAccepts(r)).toBe(true);
    expect(reviewAccepts({ ...r, accepted: false })).toBe(false);
    expect(
      reviewAccepts({
        ...r,
        criteria: [{ ...r.criteria[0], satisfied: false }],
      }),
    ).toBe(false);
    for (const blocker of ["Needs a decision", " ", "\n"])
      expect(reviewAccepts({ ...r, blocker })).toBe(false);
  });
  it("requires the independent implementation verdict for every decision revision", () => {
    const r = review();
    r.decision_reviews = [
      {
        decision_id: "D-legacy",
        implemented: false,
        evidence:
          "The candidate implements stable and does not implement legacy.",
      },
    ];
    expect(reviewAccepts(r)).toBe(false);
    r.decision_reviews[0].implemented = true;
    expect(reviewAccepts(r)).toBe(true);
  });
  it("retains null reviews, their original ordinals and session identity", () => {
    const a = (observable.run as unknown as Run).tasks.find(
        (t) => t.plan.id === "E",
      )!.attempts[0],
      records = reviewRecords(a);
    expect(records.map((r) => [r.ordinal, r.session?.id, !!r.review])).toEqual([
      [1, a.reviews[0].session.id, false],
      [2, a.reviews[1].session.id, true],
    ]);
    expect(records[0].diagnostic).toContain("invalid structured review");
    expect(
      reviewRecords({
        ...a,
        reviews: [],
        reviewer: a.reviews[1].session,
        review: a.reviews[1].review,
      })[0].ordinal,
    ).toBe(1);
  });
  it("binds canonical IDs before wording and never borrows evidence from another ID", () => {
    const task = fixture().tasks[0].plan;
    task.acceptance_criteria = ["Export", "Export CSV"];
    const r: Review = {
      ...review(),
      criteria: [
        {
          criterion_id: "A-C2",
          criterion: "Export",
          satisfied: false,
          evidence: "Only second criterion",
        },
        {
          criterion_id: "A-C1",
          criterion: "Paraphrased first criterion",
          satisfied: true,
          evidence: "Only first criterion",
        },
      ],
    };
    expect(criterionEvidence(r, task, 0)?.evidence).toBe(
      "Only first criterion",
    );
    expect(criterionEvidence(r, task, 1)?.evidence).toBe(
      "Only second criterion",
    );
    r.criteria[1].criterion_id = "wrong-C1";
    r.criteria[1].criterion = "Export";
    expect(criterionEvidence(r, task, 0)).toBeUndefined();
    r.criteria[1].criterion_id = "";
    expect(criterionEvidence(r, task, 0)).toBeUndefined();
  });
  it("permits exact text fallback only for wholly legacy, unambiguous evidence", () => {
    const task = fixture().tasks[0].plan,
      r = review();
    r.criteria[0].criterion_id = "";
    expect(criterionEvidence(r, task, 0)).toBe(r.criteria[0]);
    r.criteria.push({ ...r.criteria[0] });
    expect(criterionEvidence(r, task, 0)).toBeUndefined();
    r.criteria.pop();
    task.acceptance_criteria.push(task.acceptance_criteria[0]);
    expect(criterionEvidence(r, task, 0)).toBeUndefined();
  });
});
describe("configured verification completeness", () => {
  const commands = [["check"], ["check"]];
  function results() {
    return [0, 1].map((i) => ({
      ...result("passed"),
      command: ["check"],
      process_key: `key-${i}`,
      process_run_id: `process-${i}`,
    }));
  }
  it("requires every ordered occurrence, including repeated identical commands", () => {
    const all = results();
    expect(summarizeChecks(all, commands, true)).toMatchObject({
      state: "passed",
      complete: true,
    });
    expect(summarizeChecks(all.slice(0, 1), commands, true)).toMatchObject({
      state: "partial",
      complete: false,
      recorded: 1,
      expected: 2,
    });
    expect(summarizeChecks([all[0], all[0]], commands, true).state).toBe(
      "unavailable",
    );
    expect(summarizeChecks([...all, all[0]], commands, false).state).toBe(
      "unavailable",
    );
    expect(
      summarizeChecks([{ ...all[0], command: ["other"] }], commands, true)
        .state,
    ).toBe("unavailable");
    expect(summarizeChecks(all, [], false).state).toBe("unavailable");
    expect(
      summarizeChecks(
        [{ ...all[0], command: ["[redacted]"] }],
        [["[redacted]"]],
        false,
      ).state,
    ).toBe("unavailable");
  });
  it("distinguishes pending, terminal missing, partial, failed and untrusted results", () => {
    expect(summarizeChecks([], commands, true).state).toBe("pending");
    expect(summarizeChecks([], commands, false).state).toBe("unavailable");
    expect(
      summarizeChecks(results().slice(0, 1), commands, false).label,
    ).toContain("incomplete");
    expect(
      summarizeChecks(
        [
          {
            ...results()[0],
            passed: false,
            outcome: "test_failure",
            exit_code: 1,
          },
        ],
        commands,
        true,
      ).state,
    ).toBe("test_failure");
    for (const v of [
      { ...results()[0], output_complete: false },
      { ...results()[0], exit_code: null },
      { ...results()[0], outcome: "future_outcome" },
    ]) {
      expect(summarizeChecks([v], [["check"]], false).state).toBe(
        "unavailable",
      );
    }
  });
  it("does not mix different attempts or let terminal state imply missing checks passed", () => {
    const run = fixture(),
      task = run.tasks[0],
      first = task.attempts[0];
    run.settings.verification.commands = commands;
    first.verification = results().slice(0, 1);
    task.attempts.push({
      ...structuredClone(first),
      number: 2,
      verification: results(),
    });
    task.status = "RUNNING";
    run.status = "RUNNING";
    expect(attemptChecks(run, task, first).state).toBe("partial");
    expect(attemptChecks(run, task, task.attempts[1]).state).toBe("passed");
    for (const status of [
      "FAILED",
      "INTERRUPTED",
      "CANCELLED",
      "COMPLETED",
    ] as const) {
      run.status = status;
      task.attempts[1].verification = [];
      expect(attemptChecks(run, task, task.attempts[1]).state).toBe(
        "unavailable",
      );
    }
  });
});
describe("structured check outcomes across model, journal and summaries", () => {
  it("replays the native partial-then-timeout journal with no failed-test verdict", () => {
    const run = timeout.run as unknown as Run,
      events = timeout.events as EngineeringEvent[],
      model = deriveExecution(
        run,
        appendHistory(undefined, run.id, new Set(["A"]), events, 1000),
      );
    expect(model.unparsed).toBe(0);
    expect(model.incomplete).toBe(0);
    expect(
      model.segments.filter((s) => s.phase === "checks").map((s) => s.outcome),
    ).toEqual(["passed", "timeout"]);
    expect(
      verificationCounts(run.tasks[0].attempts[0].verification),
    ).toMatchObject({ passed: 1, timeout: 1, test_failure: 0 });
    expect(
      attemptChecks(run, run.tasks[0], run.tasks[0].attempts[0]).state,
    ).toBe("timeout");
    const finish = events
      .filter((e) => e.kind === "verification_finished")
      .at(-1)!;
    expect(describeEvent(finish, run).label).toBe(
      "Configured check · timed out",
    );
    expect(
      model.segments.find(
        (s) => s.phase === "checks" && s.outcome === "timeout",
      )?.end,
    ).not.toBeNull();
  });

  it.each([
    "passed",
    "test_failure",
    "timeout",
    "cancelled",
    "infrastructure_failure",
    "unknown",
  ] as const)(
    "preserves %s without confusing it with a failed test",
    (outcome) => {
      const run = fixture(),
        v = result(outcome),
        a = run.tasks[0].attempts[0];
      a.verification = [v];
      const events = structuredClone(completed.events) as EngineeringEvent[],
        finish = events.find(
          (e) =>
            e.kind === "verification_finished" &&
            e.detail.includes(v.process_run_id),
        )!;
      finish.detail = finish.detail.replace(
        "passed=true",
        `passed=${v.passed}`,
      );
      const history = appendHistory(
          undefined,
          run.id,
          new Set(run.tasks.map((t) => t.plan.id)),
          events,
          1000,
        ),
        m = deriveExecution(run, history);
      expect(verificationOutcome(v)).toBe(outcome);
      expect(
        m.segments.find(
          (s) => s.phase === "checks" && s.process === v.process_run_id,
        )?.outcome,
      ).toBe(outcome);
      const description = describeEvent(finish, run);
      expect(description.label.toLowerCase()).toContain(
        checkLabels[outcome].toLowerCase(),
      );
      expect(verificationCounts([v])[outcome]).toBe(1);
      expect(verificationCounts([v]).test_failure).toBe(
        outcome === "test_failure" ? 1 : 0,
      );
      const summary = summarizeChecks(
        [v],
        run.settings.verification.commands,
        false,
      );
      expect(summary.state).toBe(
        outcome === "unknown" ? "unavailable" : outcome,
      );
    },
  );
  it("does not infer false-event outcomes from argv, exit text or a missing/ambiguous snapshot join", () => {
    const run = fixture(),
      v = run.tasks[0].attempts[0].verification[0],
      event = {
        process: v.process_run_id,
        task: "A",
        attempt: 1,
        accepted: false,
      };
    v.passed = false;
    v.outcome = null;
    v.exit_code = 1;
    v.summary = "status=Failed timeout exceeded test failed";
    expect(checkEventOutcome(run, event)).toBe("unknown");
    v.outcome = "timeout";
    expect(checkEventOutcome(run, event)).toBe("timeout");
    expect(checkEventOutcome(run, { ...event, attempt: 2 })).toBe("unknown");
    expect(checkEventOutcome(run, { ...event, key: "wrong-key" })).toBe(
      "unknown",
    );
    expect(checkEventOutcome(run, { ...event, accepted: true })).toBe(
      "unknown",
    );
    run.integration_verification.push({ ...v });
    expect(checkEventOutcome(run, event)).toBe("unknown");
    run.tasks[0].attempts[0].verification = [];
    run.integration_verification = [];
    expect(checkEventOutcome(run, event)).toBe("unknown");
    expect(checkEventOutcome(run, { ...event, accepted: true })).toBe("passed");
  });
  it("keeps validated legacy success but refuses contradictory structured success", () => {
    expect(verificationOutcome({ ...result("passed"), outcome: null })).toBe(
      "passed",
    );
    expect(
      verificationOutcome({ ...result("passed"), output_complete: false }),
    ).toBe("unknown");
    expect(verificationOutcome({ ...result("timeout"), passed: true })).toBe(
      "unknown",
    );
  });
});
