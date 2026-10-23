import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import App from "../App";
import { DependencyGraph } from "../components/dependency-graph";
import { PreflightChecklist } from "../components/preflight";
import { ConfirmAction } from "../components/confirm-action";
import {
  HumanActionsView,
  ResolutionDialog,
  ResolutionPanel,
} from "../components/human-action";
import { TasksView } from "../components/tasks";
import { Lifecycle } from "../components/lifecycle";
import { lifecycleIndex, primaryAction } from "../lib/presentation";
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
  const wrap = (content: ReactNode) => (
    <QueryClientProvider client={client}>
      <TransportContext.Provider value={transport}>
        {content}
      </TransportContext.Provider>
    </QueryClientProvider>
  );
  const view = render(wrap(ui));
  return {
    ...view,
    rerender: (content: ReactNode) => view.rerender(wrap(content)),
    client,
    transport,
    request,
  };
}
describe("operational run state", () => {
  it("keeps terminal human records inspectable without requesting an invalid resolution", () => {
    const r = run({ status: "CANCELLED", human_actions: [action()] });
    setup(
      <HumanActionsView
        runs={[r]}
        connected
        initial={`${r.id}/${r.human_actions[0].id}`}
        onRun={vi.fn()}
      />,
    );
    expect(
      screen.getByText(/No actions requiring resolution/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        /cancelled run · the daemon does not accept new resolutions/i,
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("run_checks")).toBeInTheDocument();
    expect(
      screen.queryByRole("radio", { name: "Approve exact permission" }),
    ).not.toBeInTheDocument();
  });
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
      expect(
        screen.getAllByText(status[0] + status.slice(1).toLowerCase()).length,
      ).toBeGreaterThan(0);
      if (status === "BLOCKED")
        expect(
          screen.getByRole("button", { name: /Approve execution/ }),
        ).toBeDisabled();
      else
        expect(
          screen.queryByRole("button", { name: /Approve execution/ }),
        ).not.toBeInTheDocument();
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
    expect(screen.getAllByText("Waiting")).toHaveLength(2);
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
      request = vi.fn().mockRejectedValue({
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
    expect(screen.getByTestId("exact-scope")).toHaveTextContent(
      a.requirement.permission!.scope,
    );
    expect(
      screen.getByRole("button", { name: "Choose a decision" }),
    ).toBeDisabled();
    await userEvent.click(
      screen.getByRole("radio", { name: "Approve exact permission" }),
    );
    await userEvent.type(
      screen.getByLabelText(/^Decision or nonsecret evidence/),
      "Reviewed configured checks",
    );
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(
      screen.getByRole("button", { name: "Approve run_checks" }),
    );
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
    await userEvent.click(
      screen.getByRole("radio", { name: "Approve exact permission" }),
    );
    await userEvent.type(
      screen.getByLabelText(/^Decision or nonsecret evidence/),
      "Inspected",
    );
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(
      screen.getByRole("button", { name: "Approve run_checks" }),
    );
    await screen.findByRole("alert");
    await userEvent.click(
      screen.getByRole("button", { name: "Approve run_checks" }),
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
    expect(request.mock.calls.some(([req]) => req.type === "plan")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Browse…" }));
    expect(transport.pickRepository).toHaveBeenCalledOnce();
    await userEvent.type(
      screen.getByLabelText(/^Feature specification/),
      "Build a feature",
    );
    expect(
      screen.getByRole("button", { name: "Start planning" }),
    ).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(
      screen.getByRole("button", { name: "Start planning" }),
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
    expect(await screen.findAllByText("Metadata expired")).toHaveLength(1);
    expect(
      screen.queryByText("Request could not be confirmed"),
    ).not.toBeInTheDocument();
    expect(
      screen
        .getAllByRole("button", { name: /New run/ })
        .every((b) => b.hasAttribute("disabled")),
    ).toBe(true);
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

describe("Chunk 1 safe decisions", () => {
  async function fillPermission(note = "Inspected exact operation and scope") {
    await userEvent.click(
      screen.getByRole("radio", { name: "Approve exact permission" }),
    );
    await userEvent.type(
      screen.getByLabelText(/^Decision or nonsecret evidence/),
      note,
    );
    await userEvent.click(screen.getByRole("checkbox"));
  }
  it("starts without a decision, preserves verbatim scope, and names missing steps", async () => {
    const r = run(),
      a = action(),
      request = vi.fn().mockResolvedValue(r);
    setup(<ResolutionPanel run={r} action={a} editable />, request);
    expect(
      within(screen.getByRole("radiogroup", { name: "Decision" }))
        .getAllByRole("radio")
        .every((radio) => !(radio as HTMLInputElement).checked),
    ).toBe(true);
    expect(screen.getByRole("radio", { name: "Not inspected" })).toBeChecked();
    expect(screen.getByTestId("exact-operation").textContent).toBe(
      a.requirement.permission!.operation,
    );
    expect(screen.getByTestId("exact-scope").textContent).toBe(
      a.requirement.permission!.scope,
    );
    expect(
      screen.getByRole("button", { name: "Choose a decision" }),
    ).toBeDisabled();
    await userEvent.keyboard("{Meta>}{Enter}{/Meta}");
    expect(request).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole("radio", { name: "Approve exact permission" }),
    );
    expect(screen.getByRole("button", { name: "Add a note" })).toBeDisabled();
    await userEvent.type(
      screen.getByLabelText(/^Decision or nonsecret evidence/),
      "Reviewed",
    );
    expect(
      screen.getByRole("button", { name: "Confirm inspection" }),
    ).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(
      screen.getByRole("button", { name: "Approve run_checks" }),
    );
    await waitFor(() => expect(request).toHaveBeenCalledOnce());
    expect(request.mock.calls[0][0]).toEqual({
      type: "resolve",
      id: r.id,
      revision: r.revision,
      action_id: a.id,
      resolution: {
        expected_action_version: a.audit.length,
        request_id: expect.any(String),
        disposition: "approve",
        note: "Reviewed",
        effect: "unknown",
        permission: a.requirement.permission,
      },
    });
    expect(request.mock.calls.every(([req]) => req.type === "resolve")).toBe(
      true,
    );
  });
  it("clears inspection when the disposition changes and cannot approve unsupported permissions", async () => {
    const a = action();
    a.requirement.permission!.operation = "network_access";
    setup(<ResolutionPanel run={run()} action={a} editable />);
    expect(
      screen.getByRole("radio", { name: "Approve exact permission" }),
    ).toBeDisabled();
    await userEvent.click(screen.getByRole("radio", { name: "Reject" }));
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(screen.getByRole("radio", { name: "Defer" }));
    expect(screen.getByRole("checkbox")).not.toBeChecked();
  });
  it("requires an inspected effect for a continuable runtime checkpoint", async () => {
    const a = action();
    a.requirement.kind = "runtime";
    a.requirement.permission = null;
    a.continuable = true;
    const request = vi.fn().mockResolvedValue(run());
    setup(<ResolutionPanel run={run()} action={a} editable />, request);
    await userEvent.click(
      screen.getByRole("radio", { name: "Resolve with evidence" }),
    );
    await userEvent.type(
      screen.getByLabelText(/^Decision or nonsecret evidence/),
      "Confirmed no external operation occurred",
    );
    await userEvent.click(screen.getByRole("checkbox"));
    expect(
      screen.getByRole("button", { name: "Inspect effect state" }),
    ).toBeDisabled();
    await userEvent.click(screen.getByRole("radio", { name: "Not yet" }));
    expect(screen.getByRole("checkbox")).not.toBeChecked();
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "Resolve" }));
    await waitFor(() => expect(request).toHaveBeenCalledOnce());
    expect(request.mock.calls[0][0].resolution).toMatchObject({
      disposition: "resolve",
      effect: "not_performed",
      permission: null,
    });
  });
  it("reuses identical receipt IDs across panel remounts after an ambiguous response", async () => {
    const r = run(),
      a = action();
    a.id = "remount-receipt";
    const request = vi
      .fn()
      .mockRejectedValue({ message: "Response interrupted" });
    const view = setup(
      <ResolutionPanel key="first" run={r} action={a} editable />,
      request,
    );
    await fillPermission("Receipt remount test");
    await userEvent.click(
      screen.getByRole("button", { name: "Approve run_checks" }),
    );
    await screen.findByRole("alert");
    view.rerender(<ResolutionPanel key="second" run={r} action={a} editable />);
    await fillPermission("Receipt remount test");
    await userEvent.click(
      screen.getByRole("button", { name: "Approve run_checks" }),
    );
    await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
    expect(request.mock.calls[1][0]).toEqual(request.mock.calls[0][0]);
  });
  it("pins the inspected revision and rejects a changed permission until explicit reinspection", async () => {
    const r = run(),
      a = action(),
      request = vi.fn().mockResolvedValue(r);
    const view = setup(
      <ResolutionPanel run={r} action={a} editable />,
      request,
    );
    await fillPermission();
    view.rerender(
      <ResolutionPanel run={{ ...r, revision: 8 }} action={a} editable />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Approve run_checks" }),
    );
    await waitFor(() => expect(request).toHaveBeenCalledOnce());
    expect(request.mock.calls[0][0].revision).toBe(7);
    view.unmount();
    const next = setup(
      <ResolutionPanel run={r} action={a} editable />,
      request,
    );
    await fillPermission();
    const changed = structuredClone(a);
    changed.requirement.permission!.scope += " changed";
    next.rerender(
      <ResolutionPanel run={{ ...r, revision: 9 }} action={changed} editable />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("This action changed");
    expect(screen.getByTestId("exact-scope").textContent).toBe(
      a.requirement.permission!.scope,
    );
    expect(
      screen.getByRole("button", { name: "Approve run_checks" }),
    ).toBeDisabled();
    await userEvent.click(
      screen.getByRole("button", { name: "Review the new version" }),
    );
    expect(screen.getByTestId("exact-scope").textContent).toBe(
      changed.requirement.permission!.scope,
    );
    expect(
      screen.getByRole("button", { name: "Choose a decision" }),
    ).toBeDisabled();
    expect(screen.getByRole("radio", { name: "Not inspected" })).toBeChecked();
  });
});

describe("Chunk 1 navigation and lifecycle", () => {
  it("places blocked runs by approval and integration evidence, and omits terminal future steps", () => {
    const r = run({ status: "BLOCKED" });
    expect(lifecycleIndex(r)).toBe(1);
    r.approved_at = r.created_at;
    expect(lifecycleIndex(r)).toBe(2);
    r.integration_workspace = r.planning_workspace;
    expect(lifecycleIndex(r)).toBe(4);
    expect(primaryAction(r)).toBe("integrate");
    const view = setup(<Lifecycle run={{ ...r, status: "INTERRUPTED" }} />);
    expect(
      view.container.querySelector('[aria-current="step"]'),
    ).toHaveTextContent("Executing");
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
  });
  it("follows DAG dependency edges with arrows and selects a task without mutating daemon state", async () => {
    const tasks = [
        task("A"),
        task("B", ["A"], "BLOCKED_HUMAN"),
        task("D", ["B"]),
      ],
      onSelect = vi.fn();
    setup(<DependencyGraph tasks={tasks} onSelect={onSelect} />);
    screen.getByRole("button", { name: /^A:/ }).focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(screen.getByRole("button", { name: /^B:/ })).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    expect(onSelect).toHaveBeenCalledWith("B");
    await userEvent.keyboard("{ArrowRight}");
    expect(screen.getByRole("button", { name: /^D:/ })).toHaveFocus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(screen.getByRole("button", { name: /^B:/ })).toHaveFocus();
  });
  it("opens and closes the task inspector by keyboard and restores row focus", async () => {
    setup(
      <TasksView
        run={run()}
        onProcess={vi.fn()}
        onHuman={vi.fn()}
        onExecution={vi.fn()}
      />,
    );
    const rows = screen
      .getAllByRole("button")
      .filter((b) => b.hasAttribute("data-task"));
    rows[0].focus();
    await userEvent.keyboard("j{Enter}");
    expect(
      screen.getByRole("heading", { name: "B · Implement B" }),
    ).toHaveFocus();
    await userEvent.keyboard("{Escape}");
    expect(rows[1]).toHaveFocus();
    expect(
      screen.queryByRole("heading", { name: "B · Implement B" }),
    ).not.toBeInTheDocument();
  });
  it("starts dark without persisting a default and edits only local appearance", async () => {
    localStorage.clear();
    const request = vi
      .fn()
      .mockRejectedValue({ message: "Daemon unavailable" });
    setup(<App />, request);
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem("autono-theme")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(screen.getByRole("button", { name: "Appearance" }));
    await userEvent.click(screen.getByRole("radio", { name: "Light" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(localStorage.getItem("autono-theme")).toBe("light");
    await userEvent.click(
      screen.getByRole("switch", { name: /Background texture/ }),
    );
    expect(document.documentElement.dataset.texture).toBe("off");
    expect(
      request.mock.calls.every(([r]) =>
        ["settings", "status", "runs", "processes"].includes(r.type),
      ),
    ).toBe(true);
    localStorage.clear();
  });
});

describe("human action stale revision recovery", () => {
  it("requires explicit reinspection after a stale response and preserves the identical receipt", async () => {
    const r = run(),
      a = action();
    a.id = "revision-retry";
    const request = vi
      .fn()
      .mockRejectedValueOnce({
        message: "Run changed; inspect current action before resolving",
      })
      .mockResolvedValue(r);
    const view = setup(
      <ResolutionPanel run={r} action={a} editable />,
      request,
    );
    const fill = async () => {
      await userEvent.click(
        screen.getByRole("radio", { name: "Approve exact permission" }),
      );
      await userEvent.type(
        screen.getByLabelText(/^Decision or nonsecret evidence/),
        "Revision retry evidence",
      );
      await userEvent.click(screen.getByRole("checkbox"));
    };
    await fill();
    await userEvent.click(
      screen.getByRole("button", { name: "Approve run_checks" }),
    );
    await screen.findByRole("alert");
    view.rerender(
      <ResolutionPanel run={{ ...r, revision: 8 }} action={a} editable />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Review current revision" }),
    );
    expect(
      screen.getByRole("button", { name: "Choose a decision" }),
    ).toBeDisabled();
    expect(screen.getByRole("checkbox")).not.toBeChecked();
    await fill();
    await userEvent.click(
      screen.getByRole("button", { name: "Approve run_checks" }),
    );
    await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
    expect(request.mock.calls.map(([req]) => req.revision)).toEqual([7, 8]);
    expect(request.mock.calls[1][0].resolution).toEqual(
      request.mock.calls[0][0].resolution,
    );
  });
});
