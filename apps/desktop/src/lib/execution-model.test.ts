import { describe, expect, it } from "vitest";
import failed from "../test/fixtures/failed-journal.json";
import observable from "../test/fixtures/observability-journal.json";
import captured from "../test/fixtures/completed-journal.json";
import { run, task } from "../test/fixtures";
import type { EngineeringEvent, Run, Session } from "./types";
import {
  appendHistory,
  deriveExecution,
  emptyHistory,
  parseFact,
  unionDuration,
} from "./execution-model";
const snapshot = captured.run as unknown as Run,
  journal = captured.events as EngineeringEvent[];
const ids = new Set(snapshot.tasks.map((t) => t.plan.id));
function event(
  sequence: number,
  kind: string,
  detail = "",
  seconds = sequence,
): EngineeringEvent {
  return {
    sequence,
    run_id: snapshot.id,
    timestamp: new Date(
      Date.parse(snapshot.created_at) + seconds * 1000,
    ).toISOString(),
    kind,
    detail,
  };
}
function history(events: EngineeringEvent[], known = ids) {
  return appendHistory(
    undefined,
    snapshot.id,
    known,
    events,
    events.length + 1,
  );
}
const uuid = (n: number) =>
  `00000000-0000-4000-8000-${n.toString().padStart(12, "0")}`;
const session = (n: number, role = "coder"): Session => ({
  id: uuid(n),
  process_key: uuid(n + 100),
  process_run_id: uuid(n + 200),
  provider: "codex",
  role,
  provider_session_id: null,
  outcome: null,
});
function running() {
  const r = run({
    id: snapshot.id,
    status: "RUNNING",
    human_actions: [],
    tasks: [task("A", [], "RUNNING")],
  });
  r.tasks[0].attempts = [
    {
      number: 1,
      coder: session(1),
      reviewer: null,
      review: null,
      reviews: [],
      verification: [],
      snapshot: null,
      verified_tree: null,
      preserved_tree: null,
      detail: null,
    },
  ];
  return r;
}
describe("daemon journal projection", () => {
  it("maps the captured completed daemon run, two execution epochs and exact phase sources", () => {
    let h = emptyHistory();
    for (let i = 0; i < journal.length; i += 100)
      h = appendHistory(h, snapshot.id, ids, journal.slice(i, i + 100));
    const m = deriveExecution(snapshot, h);
    expect(m.unparsed).toBe(0);
    expect(m.incomplete).toBe(0);
    expect(m.segments.filter((s) => s.phase === "coding")).toHaveLength(6);
    expect(m.segments.filter((s) => s.phase === "review")).toHaveLength(5);
    expect(m.segments.filter((s) => s.phase === "checks")).toHaveLength(6);
    expect(m.segments.filter((s) => s.phase === "executing")).toHaveLength(2);
    expect(m.segments.some((s) => s.live)).toBe(false);
    expect(
      m.segments.find((s) => s.task === "B" && s.phase === "paused")?.end,
    ).not.toBeNull();
    expect(m.retries).toBe(1);
    expect(m.lastProgress).toBe(Date.parse(journal.at(-1)!.timestamp));
    for (const s of m.segments.filter((s) => s.session))
      expect(s.sources.some((f) => f.kind === "agent_started")).toBe(true);
  });
  it("maps the Chunk 2 native failure/retry/review/continuation journal without inventing history", () => {
    const r = observable.run as unknown as Run,
      events = observable.events as EngineeringEvent[];
    const h = appendHistory(
        undefined,
        r.id,
        new Set(r.tasks.map((t) => t.plan.id)),
        events,
        1000,
      ),
      m = deriveExecution(r, h);
    expect(m.unparsed).toBe(0);
    expect(m.incomplete).toBe(0);
    expect(m.segments.filter((s) => s.phase === "coding")).toHaveLength(8);
    expect(m.segments.filter((s) => s.phase === "review")).toHaveLength(7);
    expect(
      m.segments.filter((s) => s.phase === "review" && s.outcome === "failed"),
    ).toHaveLength(2);
    expect(m.segments.filter((s) => s.phase === "checks")).toHaveLength(8);
    expect(
      m.segments.filter((s) => s.phase === "checks" && s.outcome === "failed"),
    ).toHaveLength(1);
    expect(m.segments.filter((s) => s.phase === "executing")).toHaveLength(2);
    expect(m.segments.some((s) => s.live)).toBe(false);
    expect(m.retries).toBe(3);
    const eReviews = m.segments.filter(
      (s) => s.task === "E" && s.phase === "review",
    );
    expect(eReviews.map((s) => s.attempt)).toEqual([1, 1]);
    expect(eReviews.map((s) => s.review)).toEqual([1, 2]);
  });
  it("maps a real terminal failure with interleaved-run cursor gaps and no verified artifacts", () => {
    const r = failed.run as unknown as Run,
      events = failed.events as EngineeringEvent[],
      m = deriveExecution(
        r,
        appendHistory(undefined, r.id, new Set(["A"]), events, 1000),
      );
    expect(m.unparsed).toBe(0);
    expect(m.incomplete).toBe(0);
    expect(m.segments.some((s) => s.live)).toBe(false);
    expect(
      m.segments.filter((s) => s.phase === "checks" && s.outcome === "failed"),
    ).toHaveLength(3);
    expect(m.markers.filter((m) => m.kind === "completed")).toHaveLength(0);
    expect(
      events.some((e, i) => i > 0 && e.sequence > events[i - 1].sequence + 1),
    ).toBe(true);
  });
  it("uses sequence order, tolerates global cursor gaps, deduplicates replay and bounds the raw journal", () => {
    const events = Array.from({ length: 620 }, (_, i) =>
      event(i * 3 + 1, "scheduler"),
    );
    const h = appendHistory(
      history(events.slice(0, 310)),
      snapshot.id,
      ids,
      [...events.slice(300)].reverse(),
    );
    expect(h.cursor).toBe(events.at(-1)!.sequence);
    expect(h.facts).toHaveLength(620);
    expect(h.recent).toHaveLength(500);
    expect(h.facts[310].sequence).toBe(events[310].sequence);
  });
  it("caps timeline facts but continues the bounded cursor journal", () => {
    const h = history(
      Array.from({ length: 5100 }, (_, i) => event(i + 1, "scheduler")),
    );
    expect(h.limited).toBe(true);
    expect(h.facts).toHaveLength(5000);
    expect(h.cursor).toBe(5100);
    expect(h.recent).toHaveLength(500);
  });
  it("rejects malformed, unknown, redacted and unknown-task events without fabricating phases", () => {
    const events = [
      event(1, "task_state", "[REDACTED] -> Running"),
      event(2, "task_state", "Z: None -> Running"),
      event(3, "new_kind", "A"),
      event(
        4,
        "agent_started",
        `Coder Codex; session=${uuid(90)} process_key=${uuid(91)}`,
      ),
      event(5, "task_state", "A: None -> Running trailing"),
    ];
    const m = deriveExecution(running(), history(events));
    expect(m.unparsed).toBe(5);
    expect(m.segments).toEqual([]);
  });
  it("does not treat reserved sessions as active and correlates concurrent sessions by UUID", () => {
    const r = running();
    expect(deriveExecution(r, history([])).segments).toEqual([]);
    const e = event(
      1,
      "agent_started",
      `Coder Codex; session=${uuid(1)} process_key=${uuid(101)}`,
    );
    const m = deriveExecution(r, history([e]));
    expect(m.segments[0]).toMatchObject({
      task: "A",
      attempt: 1,
      phase: "coding",
      live: true,
    });
  });
  it("requires a start before a finish; never stretches incomplete history into a terminal run", () => {
    const r = running();
    r.status = "INTERRUPTED";
    const m = deriveExecution(
      r,
      history([
        event(
          1,
          "agent_started",
          `Coder Codex; session=${uuid(1)} process_key=${uuid(101)}`,
        ),
      ]),
    );
    expect(m.segments).toEqual([]);
    expect(m.incomplete).toBe(1);
    expect(
      deriveExecution(
        r,
        history([
          event(
            2,
            "agent_finished",
            `Coder session=${uuid(1)} outcome=succeeded`,
          ),
        ]),
      ).unparsed,
    ).toBe(1);
  });
  it("preserves check failures and joins a live check before a result exists", () => {
    const r = running(),
      start = event(
        1,
        "verification_started",
        `target=Some(("A", 1)) process_key=${uuid(6)} command=["sh","-c","false"]`,
      );
    expect(deriveExecution(r, history([start])).segments[0]).toMatchObject({
      phase: "checks",
      live: true,
      attempt: 1,
    });
    const m = deriveExecution(
      r,
      history([
        start,
        event(
          3,
          "verification_process",
          `target=Some(("A", 1)) process=${uuid(7)}`,
        ),
        event(
          4,
          "verification_finished",
          `target=Some(("A", 1)) process=${uuid(7)} passed=false`,
        ),
      ]),
    );
    expect(m.segments[0]).toMatchObject({
      outcome: "failed",
      process: uuid(7),
      live: false,
    });
  });
  it("refuses ambiguous parallel check joins", () => {
    const a = (n: number) =>
      event(
        n,
        "verification_started",
        `target=Some(("A", 1)) process_key=${uuid(n)} command=["true"]`,
      );
    const m = deriveExecution(
      running(),
      history([
        a(1),
        a(2),
        event(
          3,
          "verification_finished",
          `target=Some(("A", 1)) process=${uuid(9)} passed=true`,
        ),
      ]),
    );
    expect(m.unparsed).toBe(1);
    expect(m.segments.filter((s) => !s.live)).toEqual([]);
  });
  it.each(["review_finished", "review_invalid_or_failed"])(
    "uses overall verdict for %s, keeping review attempt/session attribution",
    (kind) => {
      const r = running();
      r.tasks[0].attempts[0].reviews = [
        {
          session: session(2, "reviewer"),
          workspace: null,
          review: null,
          diagnostic: null,
        },
      ];
      const m = deriveExecution(
        r,
        history([
          event(1, "attempt_started", "A: attempt 1"),
          event(
            2,
            "agent_started",
            `Reviewer Codex; session=${uuid(2)} process_key=${uuid(102)}`,
          ),
          event(
            3,
            "agent_finished",
            `Reviewer session=${uuid(2)} outcome=succeeded`,
          ),
          event(
            4,
            kind,
            kind === "review_finished"
              ? "A: accepted=false findings=1"
              : "A: invalid JSON",
          ),
        ]),
      );
      expect(m.segments[0]).toMatchObject({
        phase: "review",
        outcome: "failed",
        attempt: 1,
        review: 1,
      });
    },
  );
  it("attaches all state changes to the transaction cause and labels historical waits from history", () => {
    const h = history([
      event(1, "plan_ready"),
      event(2, "task_state", "A: None -> Waiting"),
      event(3, "task_state", "B: None -> Waiting"),
      event(4, "task_state", "A: Some(Waiting) -> Running"),
      event(5, "task_state", "A: Some(Running) -> BlockedHuman"),
      event(6, "task_state", "B: Some(Waiting) -> Ready"),
    ]);
    expect(h.facts[2].cause?.sequence).toBe(1);
    const r = run({
      id: snapshot.id,
      status: "RUNNING",
      tasks: [task("A", [], "BLOCKED_HUMAN"), task("B", ["A"], "READY")],
    });
    expect(
      deriveExecution(r, h).states.find(
        (s) => s.task === "B" && s.phase === "waiting",
      )?.waitingOn,
    ).toEqual(["A"]);
  });
  it("does not split execution for an active continuation or a no-op scheduler checkpoint", () => {
    const r = running();
    const m = deriveExecution(
      r,
      history([
        event(1, "approved"),
        event(2, "task_state", "A: None -> Running"),
        event(3, "continued"),
        event(4, "execution_finished"),
      ]),
    );
    expect(m.segments.filter((s) => s.phase === "executing")).toHaveLength(1);
    expect(m.segments.find((s) => s.phase === "executing")?.live).toBe(true);
  });
  it.each(["FAILED", "INTERRUPTED", "CANCELLED"] as const)(
    "closes recorded phases on explicit stop, preserving %s snapshot state",
    (status) => {
      const r = running();
      r.status = status;
      const m = deriveExecution(
        r,
        history([
          event(
            1,
            "agent_started",
            `Coder Codex; session=${uuid(1)} process_key=${uuid(101)}`,
          ),
          event(2, "workflow_stopped", "diagnostic"),
        ]),
      );
      expect(m.segments[0]).toMatchObject({
        end: Date.parse(event(2, "workflow_stopped").timestamp),
        live: false,
        outcome: "interrupted",
      });
    },
  );
  it("never repairs backwards timestamps into an invented interval", () => {
    const r = running();
    const m = deriveExecution(
      r,
      history([
        event(
          1,
          "agent_started",
          `Coder Codex; session=${uuid(1)} process_key=${uuid(101)}`,
          5,
        ),
        event(
          2,
          "agent_finished",
          `Coder session=${uuid(1)} outcome=succeeded`,
          2,
        ),
      ]),
    );
    expect(m.segments).toEqual([]);
    expect(m.incomplete).toBe(1);
  });
  it("unions overlapping wall-clock intervals without double-counting agents", () => {
    const s = deriveExecution(
      running(),
      history([
        event(
          1,
          "agent_started",
          `Coder Codex; session=${uuid(1)} process_key=${uuid(101)}`,
        ),
      ]),
    ).segments[0];
    expect(unionDuration([s, s], s.start + 5000)).toBe(5000);
  });
  it("pins bare, prefixed and integration formats", () => {
    for (const e of journal)
      expect(parseFact(e, ids), `${e.kind} #${e.sequence}`).not.toBeNull();
  });
});
