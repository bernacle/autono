import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import App from "../App";
import { DependencyGraph } from "../components/dependency-graph";
import { PreflightChecklist } from "../components/preflight";
import { ConfirmAction } from "../components/confirm-action";
import { ResolutionDialog } from "../components/human-action";
import { RunDetail } from "../components/run-detail";
import { NewRun } from "../components/new-run";
import { Activity } from "../components/activity";
import { TransportContext, type Transport } from "../lib/transport";
import { appendEvents, blockedAncestors, can } from "../lib/utils";
import { run, task, action } from "./fixtures";
function setup(ui: ReactNode, request = vi.fn().mockResolvedValue([])) {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchInterval: false, gcTime: 0 },
      mutations: { retry: false },
    },
  });
  const transport: Transport = {
    request,
    pickRepository: vi.fn().mockResolvedValue("/tmp/fixture"),
    inspectArtifact: vi
      .fn()
      .mockResolvedValue({ accessible: true, path: "/tmp/worktree" }),
  };
  const view = render(
    <QueryClientProvider client={client}>
      <TransportContext.Provider value={transport}>
        {ui}
      </TransportContext.Provider>
    </QueryClientProvider>,
  );
  return { ...view, client, transport, request };
}
describe("operational run state", () => {
  it.each(["BLOCKED", "FAILED", "INTERRUPTED"] as const)(
    "renders %s distinctly without enabling execute",
    async (status) => {
      const r = run({ status });
      setup(
        <RunDetail id={r.id} connected onBack={vi.fn()} />,
        vi
          .fn()
          .mockImplementation((req) =>
            Promise.resolve(req.type === "events" ? [] : r),
          ),
      );
      expect(
        await screen.findByRole("heading", { name: "Build a safe feature" }),
      ).toBeInTheDocument();
      expect(screen.getByText(status.toLowerCase())).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "Approve execution" }),
      ).toBeDisabled();
      if (status === "INTERRUPTED")
        expect(
          screen.getByText(/does not support resuming interrupted/),
        ).toBeInTheDocument();
    },
  );
  it("shows transitive blocked descendants without changing their WAITING state", () => {
    const tasks = [
      task("A", [], "COMPLETED"),
      task("B", ["A"], "BLOCKED_HUMAN"),
      task("D", ["B"]),
      task("F", ["D"]),
      task("C", [], "COMPLETED"),
    ];
    expect(blockedAncestors(tasks[3], tasks)).toEqual(["B"]);
    setup(<DependencyGraph tasks={tasks} />);
    expect(screen.getAllByText("Waiting on B")).toHaveLength(2);
    expect(screen.getAllByText("waiting")).toHaveLength(2);
    expect(screen.getByText("← B")).toBeInTheDocument();
  });
  it("handles unknown and failed readiness without inventing success", () => {
    const report = run().preflight!;
    report.ready = false;
    report.checks.push({
      id: "missing-tool",
      ready: false,
      blocking: true,
      evidence: "Executable missing",
    });
    setup(<PreflightChecklist report={report} />);
    expect(
      screen.getByText("Prerequisites need attention"),
    ).toBeInTheDocument();
    expect(screen.getByText("Unknown")).toBeInTheDocument();
    expect(screen.getByText("Missing")).toBeInTheDocument();
  });
  it("requires all runtime checkpoints before offering continue", () => {
    const r = run({ status: "BLOCKED", approved_at: "2026-10-20T12:00:00Z" }),
      a = action();
    a.requirement.kind = "runtime";
    a.requirement.permission = null;
    a.requirement.affected_tasks = ["B"];
    a.continuable = true;
    a.status = "RESOLVED";
    a.resolution = {
      request_id: "r",
      disposition: "resolve",
      note: "Inspected",
      effect: "not_performed",
      permission: null,
    };
    r.human_actions = [a];
    expect(can(r, "continue")).toBe(true);
    a.resolution.effect = "unknown";
    expect(can(r, "continue")).toBe(false);
    r.status = "INTERRUPTED";
    expect(can(r, "continue")).toBe(false);
  });
});
describe("explicit approvals and errors", () => {
  it("waits for confirmation and daemon response before reporting acceptance", async () => {
    let accept!: (value: unknown) => void;
    const request = vi.fn(
        () =>
          new Promise((resolve) => {
            accept = resolve;
          }),
      ),
      onAccepted = vi.fn();
    setup(
      <ConfirmAction
        run={run()}
        action="execute"
        onClose={vi.fn()}
        onAccepted={onAccepted}
      />,
      request,
    );
    expect(request).not.toHaveBeenCalled();
    expect(screen.getByText(/test/)).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Approve execution" }),
    );
    expect(onAccepted).not.toHaveBeenCalled();
    expect(request).toHaveBeenCalledWith({
      type: "action",
      id: run().id,
      revision: 7,
      action: "execute",
    });
    accept({});
    await waitFor(() => expect(onAccepted).toHaveBeenCalledOnce());
  });
  it("shows rejected and stale actions without a success message", async () => {
    const onAccepted = vi.fn(),
      request = vi
        .fn()
        .mockRejectedValue({
          code: "stale",
          message: "Run changed; refresh state",
        });
    setup(
      <ConfirmAction
        run={run()}
        action="execute"
        onClose={vi.fn()}
        onAccepted={onAccepted}
      />,
      request,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Approve execution" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("Run changed");
    expect(onAccepted).not.toHaveBeenCalled();
  });
  it("renders exact permission scope and submits it only after inspection", async () => {
    const a = action(),
      request = vi.fn().mockResolvedValue(run());
    setup(
      <ResolutionDialog run={run()} action={a} editable onClose={vi.fn()} />,
      request,
    );
    expect(
      screen.getByText(/immutable configured verification commands/),
    ).toBeInTheDocument();
    const submit = screen.getByRole("button", { name: "Confirm resolution" });
    expect(submit).toBeDisabled();
    await userEvent.type(
      screen.getByLabelText(/^Decision or nonsecret evidence/),
      "Reviewed configured checks",
    );
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(submit);
    await waitFor(() => expect(request).toHaveBeenCalled());
    expect(request.mock.calls[0][0].resolution.permission).toEqual(
      a.requirement.permission,
    );
  });
  it("keeps the same resolution receipt when an ambiguous response is retried", async () => {
    const request = vi
      .fn()
      .mockRejectedValue({ message: "Response interrupted" });
    setup(
      <ResolutionDialog
        run={run()}
        action={action()}
        editable
        onClose={vi.fn()}
      />,
      request,
    );
    await userEvent.type(
      screen.getByLabelText(/^Decision or nonsecret evidence/),
      "Inspected",
    );
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(
      screen.getByRole("button", { name: "Confirm resolution" }),
    );
    await screen.findByRole("alert");
    await userEvent.click(
      screen.getByRole("button", { name: "Confirm resolution" }),
    );
    await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
    expect(request.mock.calls[0][0]).toEqual(request.mock.calls[1][0]);
  });
  it("requires a native repository selection and explicit planner acknowledgement", async () => {
    const request = vi.fn().mockResolvedValue(run());
    const { transport } = setup(
      <NewRun connected onClose={vi.fn()} onCreated={vi.fn()} />,
      request,
    );
    expect(request).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Browse…" }));
    expect(transport.pickRepository).toHaveBeenCalledOnce();
    await userEvent.type(
      screen.getByLabelText(/^Feature specification/),
      "Build a feature",
    );
    expect(
      screen.getByRole("button", { name: "Submit planning" }),
    ).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(
      screen.getByRole("button", { name: "Submit planning" }),
    );
    await waitFor(() => expect(request).toHaveBeenCalled());
  });
});
describe("connection and durable events", () => {
  it("renders disconnection and disables new execution", async () => {
    setup(
      <App />,
      vi
        .fn()
        .mockRejectedValue({ code: "metadata", message: "Metadata expired" }),
    );
    expect(
      (await screen.findAllByText("Metadata expired")).length,
    ).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "New run" })).toBeDisabled();
  });
  it("deduplicates replay, accepts cursor gaps and bounds retained events", () => {
    const first = appendEvents(undefined, [{ sequence: 3 }, { sequence: 8 }]);
    const second = appendEvents(first, [
      { sequence: 8 },
      { sequence: 14 },
      { sequence: 14 },
    ]);
    expect(second.items.map((e) => e.sequence)).toEqual([3, 8, 14]);
    expect(second.cursor).toBe(14);
    const bounded = appendEvents(
      second,
      Array.from({ length: 800 }, (_, i) => ({ sequence: i + 20 })),
    );
    expect(bounded.items).toHaveLength(500);
    expect(bounded.cursor).toBe(819);
  });
  it("retains cursor across a failed poll and reconnects from the last confirmed event", async () => {
    const event = {
        sequence: 42,
        run_id: "run",
        timestamp: "2026-10-20T12:00:00Z",
        kind: "task_started",
        detail: "Task A started",
      },
      request = vi
        .fn()
        .mockResolvedValueOnce([event])
        .mockRejectedValueOnce({ message: "Daemon restarting" })
        .mockResolvedValueOnce([
          { ...event, sequence: 51, detail: "Task A completed" },
        ]);
    const { client } = setup(<Activity id="run" />, request);
    await screen.findByText("Task A started");
    await client.refetchQueries({ queryKey: ["events", "run"] });
    expect(await screen.findByText("Daemon restarting")).toBeInTheDocument();
    await client.refetchQueries({ queryKey: ["events", "run"] });
    await screen.findByText("Task A completed");
    expect(request.mock.calls[2][0]).toEqual({
      type: "events",
      id: "run",
      after: 42,
    });
  });
});
