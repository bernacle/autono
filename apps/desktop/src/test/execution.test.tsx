import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { task as makeTask } from "./fixtures";
import captured from "./fixtures/completed-journal.json";
import { appendHistory, deriveExecution } from "../lib/execution-model";
import { useExecutionHistory } from "../lib/queries";
import { relatedTasks } from "../lib/execution";
import { describeEvent } from "../lib/journal";
import { ViewBoundary } from "../components/view-boundary";
import { PhaseSource } from "../components/phase-source";
import { TaskInspector } from "../components/tasks";
import { LogDock } from "../components/log-dock";
import { Journal } from "../components/journal";
import { ExecutionView, Timeline } from "../components/execution";
import { RunDetail } from "../components/run-detail";
import { TransportContext, type Transport } from "../lib/transport";
import type { EngineeringEvent, Run, Request } from "../lib/types";
const run = captured.run as unknown as Run,
  events = captured.events as EngineeringEvent[],
  ids = new Set(run.tasks.map((t) => t.plan.id));
const history = appendHistory(undefined, run.id, ids, events, 1000),
  model = deriveExecution(run, history);
function setup(ui: ReactNode, request = vi.fn().mockResolvedValue([])) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  const transport: Transport = {
    request,
    pickRepository: vi.fn(),
    inspectArtifact: vi.fn(),
  };
  return {
    ...render(
      <QueryClientProvider client={client}>
        <TransportContext.Provider value={transport}>
          {ui}
        </TransportContext.Provider>
      </QueryClientProvider>,
    ),
    client,
    request,
  };
}
describe("execution interaction and history", { timeout: 20000 }, () => {
  it("draws only sourced intervals and uses the same phase ids in overview lanes", () => {
    const onPhase = vi.fn();
    const view = setup(
      <Timeline
        run={run}
        model={model}
        selected={null}
        onTask={vi.fn()}
        onPhase={onPhase}
        onGraph={vi.fn()}
      />,
    );
    for (const s of model.segments) {
      expect(s.sources.length).toBeGreaterThan(0);
      if (s.end !== null) expect(s.end).toBeGreaterThanOrEqual(s.start);
    }
    const idsFull = [...view.container.querySelectorAll("[data-phase]")].map(
      (n) => n.getAttribute("data-phase"),
    );
    view.unmount();
    const mini = setup(
      <Timeline
        mini
        run={run}
        model={model}
        selected={null}
        onTask={vi.fn()}
        onPhase={onPhase}
        onGraph={vi.fn()}
      />,
    );
    for (const n of mini.container.querySelectorAll("[data-phase]"))
      expect(idsFull).toContain(n.getAttribute("data-phase"));
  });
  it("supports roving rows, horizontal phase navigation, inspection, graph and follow shortcuts", async () => {
    const select = vi.fn(),
      graph = vi.fn(),
      phase = vi.fn(),
      u = userEvent.setup();
    setup(
      <Timeline
        run={run}
        model={model}
        selected="B"
        onTask={select}
        onPhase={phase}
        onGraph={graph}
      />,
    );
    const row = screen.getByRole("button", { name: "Run lifecycle" });
    row.focus();
    await u.keyboard("{ArrowDown}");
    expect(document.activeElement?.textContent).toContain("A");
    await u.keyboard("{ArrowRight}{Enter}");
    expect(phase).toHaveBeenCalled();
    await u.keyboard("g");
    expect(graph).toHaveBeenCalled();
    await u.keyboard("f");
    expect(screen.getByRole("button", { name: "Follow now" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });
  it("keeps graph and timeline task selection, source inspection and log references synchronized", async () => {
    const request = vi.fn((r: Request) =>
      Promise.resolve(
        r.type === "run"
          ? run
          : r.type === "events"
            ? events.filter((e) => e.sequence > r.after).slice(0, 100)
            : r.type === "process_events"
              ? { after: 1, has_more: false, lines: [] }
              : [],
      ),
    );
    setup(
      <RunDetail
        id={run.id}
        initialTab="execution"
        connected
        onBack={vi.fn()}
      />,
      request,
    );
    const phase = await screen.findByRole(
      "button",
      {
        name: /B, Coding, attempt 2/i,
      },
      { timeout: 10000 },
    );
    await userEvent.click(phase);
    expect(
      await screen.findByText(/Selected phase · attempt 2/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Process log dock" }),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Close log dock" }),
    );
    await userEvent.click(
      screen.getAllByRole("button", { name: /Show in graph/ })[0],
    );
    expect(
      await screen.findByRole("button", { name: /B: Implement report export/ }),
    ).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(
      screen.getByRole("button", { name: /C: Add validation helpers/ }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: /Show in timeline/ }),
    );
    expect(
      document.querySelector(".timeline-selection")?.textContent,
    ).toContain("Selected C · upstream none · downstream E");
    expect(window.location.hash).toContain("task=C");
    expect(
      screen.queryByText(/Selected phase · attempt 2/),
    ).not.toBeInTheDocument();
    expect(
      screen.getAllByRole("button", { name: /B, Coding, attempt 2/i })[0],
    ).toHaveAttribute("aria-pressed", "false");
    window.history.replaceState(null, "", "/");
  });
  it("never translates an unparsed result into a passed or failed verdict", () => {
    for (const kind of [
      "review_finished",
      "verification_finished",
      "agent_finished",
    ]) {
      const description = describeEvent(
        { ...events[0], kind, detail: "[redacted]" },
        run,
      );
      expect(description.label).toMatch(/result recorded/);
      expect(description.problem).toBe(false);
    }
  });
  it("translates known kinds, preserves original evidence and has a humanized unknown fallback", async () => {
    setup(
      <Journal
        run={run}
        events={[
          ...events,
          {
            ...events[0],
            sequence: 9999,
            kind: "future_record",
            detail: "opaque evidence",
          },
        ]}
        cursor={9999}
      />,
    );
    expect(screen.getByText("future record")).toBeInTheDocument();
    expect(screen.getByText("future_record #9999")).toBeInTheDocument();
    expect(screen.getByText("opaque evidence")).toBeInTheDocument();
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Journal filter" }),
      "Problems",
    );
    expect(screen.queryByText("future record")).not.toBeInTheDocument();
    expect(
      screen.getByText(/Task stopped · inspect evidence/),
    ).toBeInTheDocument();
  });
  it("retains rejected checks, retries and independent finding evidence without raw result fields", () => {
    const r = structuredClone(run),
      a = r.tasks[0].attempts[0];
    a.reviews[0].review!.findings = [
      {
        severity: "medium",
        file: "A.txt",
        line: 1,
        explanation: "Missing header",
        recommended_fix: "Add header",
        confidence: 0.9,
      },
    ];
    setup(
      <ExecutionView
        run={r}
        model={model}
        history={history}
        selected={null}
        phaseId={null}
        onSelect={vi.fn()}
        onPhase={vi.fn()}
        onProcess={vi.fn()}
        onGraph={vi.fn()}
        onHuman={vi.fn()}
        connected
      />,
    );
    expect(screen.getByText("Missing header")).toBeInTheDocument();
    expect(screen.getByText("A.txt:1")).toBeInTheDocument();
    expect(
      screen.getByText(/All agent sessions and process references/),
    ).toBeInTheDocument();
  });
  it("derives transitive selection relationships from the actual DAG", () => {
    const related = relatedTasks(run, "B");
    expect([...related.upstream]).toEqual(["A"]);
    expect([...related.downstream]).toEqual(["D"]);
  });
  it("replays every page, tolerates global gaps, and retains its cursor across a disconnect", async () => {
    let fail = false;
    const cursors: number[] = [];
    const request = vi.fn((r: Request) => {
      if (r.type !== "events") return Promise.resolve([]);
      cursors.push(r.after);
      if (fail) return Promise.reject(new Error("daemon unavailable"));
      return Promise.resolve(
        events.filter((e) => e.sequence > r.after).slice(0, 100),
      );
    });
    function Probe() {
      const q = useExecutionHistory(run);
      return (
        <>
          <span>{q.data?.cursor ?? 0}</span>
          <span>{q.data?.caughtUp ? "caught up" : "loading"}</span>
          <button onClick={() => void q.refetch()}>Refresh</button>
          {q.isError && <p>Disconnected · history retained</p>}
        </>
      );
    }
    setup(<Probe />, request);
    await screen.findByText("caught up");
    expect(
      screen.getByText(String(events.at(-1)!.sequence)),
    ).toBeInTheDocument();
    expect(cursors).toContain(events[99].sequence);
    fail = true;
    await userEvent.click(screen.getByText("Refresh"));
    await screen.findByText(/Disconnected/);
    expect(
      screen.getByText(String(events.at(-1)!.sequence)),
    ).toBeInTheDocument();
    fail = false;
    await userEvent.click(screen.getByText("Refresh"));
    await waitFor(() =>
      expect(screen.queryByText(/Disconnected/)).not.toBeInTheDocument(),
    );
    expect(cursors.at(-1)).toBe(events.at(-1)!.sequence);
  });
  it("fetches an older phase's exact source lazily through the existing sanitized cursor transport", async () => {
    const e = events.find((e) => e.kind === "agent_started")!;
    const request = vi.fn().mockResolvedValue([e]);
    setup(
      <PhaseSource
        runId={run.id}
        source={{ sequence: e.sequence, timestamp: e.timestamp, kind: e.kind }}
      />,
      request,
    );
    expect(request).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByText(new RegExp(`${e.kind} #${e.sequence}`)),
    );
    expect(await screen.findByText(e.detail)).toBeInTheDocument();
    expect(request).toHaveBeenCalledWith({
      type: "events",
      id: run.id,
      after: e.sequence - 1,
    });
  });
  it("includes the nonmodal log dock in the minimum-window inspector focus loop", async () => {
    const closeInspector = vi.fn(),
      closeLogs = vi.fn();
    setup(
      <>
        <TaskInspector
          run={run}
          task={run.tasks[0]}
          onClose={closeInspector}
          logDockOpen
          onProcess={vi.fn()}
          onHuman={vi.fn()}
        />
        <LogDock id="fixture-process" onClose={closeLogs} />
      </>,
      vi.fn().mockResolvedValue({ after: 0, has_more: false, lines: [] }),
    );
    expect(screen.getByRole("dialog")).toHaveAttribute(
      "aria-owns",
      "process-log-dock",
    );
    const replay = screen.getByRole("button", { name: "Replay logs" }),
      close = within(screen.getByRole("dialog")).getByRole("button", {
        name: "Close task inspector",
      });
    replay.focus();
    await userEvent.tab();
    expect(close).toHaveFocus();
    await userEvent.tab({ shift: true });
    expect(replay).toHaveFocus();
    await userEvent.keyboard("{Escape}");
    expect(closeLogs).toHaveBeenCalled();
    expect(closeInspector).not.toHaveBeenCalled();
  });
  it("keeps a render failure recoverable without invoking daemon commands", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    function Broken(): ReactNode {
      throw new Error("fixture rendering fault");
    }
    const reload = vi.fn(),
      view = setup(
        <ViewBoundary reload={reload}>
          <Broken />
        </ViewBoundary>,
      );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Desktop view could not render",
    );
    await userEvent.click(screen.getByRole("button", { name: "Reload view" }));
    expect(reload).toHaveBeenCalledOnce();
    expect(view.request).not.toHaveBeenCalled();
  });
  it("collapses large milestone groups and lets the developer reveal every task", async () => {
    const onTask = vi.fn();
    const tasks = Array.from({ length: 26 }, (_, i) =>
      makeTask(`T${i}`, i ? [`T${i - 1}`] : []),
    );
    const large = {
      ...run,
      tasks,
      plan: {
        ...run.plan!,
        milestones: [
          {
            id: "large",
            goal: "Growing milestone",
            task_ids: tasks.map((t) => t.plan.id),
          },
        ],
      },
    };
    setup(
      <Timeline
        run={large}
        model={{ ...model, segments: [], states: [], markers: [] }}
        selected={null}
        onTask={onTask}
        onPhase={vi.fn()}
        onGraph={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("button", { name: /Growing milestone/ }),
    ).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(
      screen.getByRole("button", { name: /Growing milestone/ }),
    );
    expect(screen.getByRole("button", { name: /T25/ })).toBeInTheDocument();
  });
  it("projects 5,000 normalized journal events within the operational budget", () => {
    const many = Array.from({ length: 5000 }, (_, i) => ({
      ...events[i % events.length],
      sequence: i + 1,
      timestamp: new Date(
        Date.parse(events[i % events.length].timestamp) +
          Math.floor(i / events.length) * 3600000,
      ).toISOString(),
    }));
    const h = appendHistory(undefined, run.id, ids, many, 6000);
    const timings = [];
    for (let i = 0; i < 5; i++) {
      const start = performance.now();
      deriveExecution(run, h);
      timings.push(performance.now() - start);
    }
    const median = timings.sort((a, b) => a - b)[2];
    console.info(`5,000-event projection median: ${median.toFixed(2)} ms`);
    expect(median).toBeLessThan(100);
  });
});
