import { it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BuildFeature } from "../components/build-feature";
import { FeatureSummary } from "../components/feature-summary";
import { TransportContext, type Transport } from "../lib/transport";
import type { DelegationContract } from "../lib/types";
import { can } from "../lib/utils";
import { run } from "./fixtures";
const contract: DelegationContract = {
  version: 2,
  request: {
    submission: {
      id: "build-id",
      repository: "/tmp/fixture",
      base: "HEAD",
      specification: "CSV export",
    },
    source_scope: {
      version: 1,
      read_subtrees: ["src", "tests"],
      write_subtrees: ["src", "tests"],
      allow_create: true,
    },
    writable_files: [],
    max_runtime_seconds: 1800,
    local_integration: true,
  },
  base_commit: "a".repeat(40),
  worktree_root: "/tmp/home/engineering/build-id",
  settings: run().settings,
  verification_sandbox: "macos-seatbelt-v1",
  source_inventory: ["src/reporting.ts"],
  limitations: ["No arbitrary tools; no spending cap"],
};
function setup(ui: React.ReactNode, request = vi.fn()) {
  const transport: Transport = {
    request,
    pickRepository: vi.fn().mockResolvedValue("/tmp/fixture"),
    inspectArtifact: vi.fn(),
  };
  render(
    <QueryClientProvider
      client={
        new QueryClient({
          defaultOptions: {
            queries: { retry: false },
            mutations: { retry: false },
          },
        })
      }
    >
      <TransportContext.Provider value={transport}>
        {ui}
      </TransportContext.Provider>
    </QueryClientProvider>,
  );
  return transport;
}
it("inspects source scope then sends one exact consent without orchestration calls", async () => {
  localStorage.clear();
  const created = vi.fn(),
    request = vi
      .fn()
      .mockImplementation((req) =>
        Promise.resolve(req.type === "propose_delegation" ? contract : run()),
      );
  setup(
    <BuildFeature connected onCreated={created} onSupervised={vi.fn()} />,
    request,
  );
  const user = userEvent.setup();
  expect(
    screen.getByRole("heading", { name: "What do you want to build?" }),
  ).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Change repository" }));
  await user.type(
    screen.getByRole("textbox", { name: "Feature request" }),
    "CSV export",
  );
  await user.click(screen.getByRole("button", { name: "Build feature" }));
  await screen.findByRole("dialog", { name: "Authorize this feature" });
  expect(request).toHaveBeenCalledTimes(1);
  expect(request.mock.calls[0][0].request.writable_files).toEqual([]);
  expect(request.mock.calls[0][0].request.source_scope.write_subtrees).toEqual([
    "src",
    "tests",
  ]);
  expect(screen.getByText(/No spending cap\./)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Authorize and build" }));
  await waitFor(() => expect(created).toHaveBeenCalled());
  expect(request.mock.calls.map((c) => c[0].type)).toEqual([
    "propose_delegation",
    "authorize_delegation",
  ]);
  expect(request.mock.calls[1][0].displayed_contract).toEqual(contract);
});
it("reuses proposal identity after an ambiguous inspection response", async () => {
  localStorage.clear();
  const request = vi.fn().mockRejectedValue(new Error("Response interrupted"));
  setup(
    <BuildFeature connected onCreated={vi.fn()} onSupervised={vi.fn()} />,
    request,
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Change repository" }));
  await user.type(
    screen.getByRole("textbox", { name: "Feature request" }),
    "CSV export",
  );
  await user.click(screen.getByRole("button", { name: "Build feature" }));
  await screen.findByText("Response interrupted");
  await user.click(screen.getByRole("button", { name: "Build feature" }));
  await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
  expect(request.mock.calls[0][0]).toEqual(request.mock.calls[1][0]);
});
it("keeps incomplete evidence truthful and never offers routine delegated approvals", () => {
  const r = run({
    status: "COMPLETED",
    delegation: { contract, authorized_at: "now", expires_at: "later" },
    handoff: {
      outcome: "evidence_incomplete",
      summary: "Missing review evidence",
      tasks: [],
      needs_you: [],
      combined_checks_passed: false,
      completion_requirements_satisfied: false,
      internal_decision_ids: [],
    },
  });
  setup(
    <FeatureSummary
      run={r}
      connected
      onDetails={vi.fn()}
      onHuman={vi.fn()}
      onCancel={vi.fn()}
      onRefresh={vi.fn()}
    />,
    vi.fn().mockResolvedValue([]),
  );
  expect(screen.queryByText("Ready for review")).not.toBeInTheDocument();
  expect(screen.getByText("Missing review evidence")).toBeInTheDocument();
  for (const status of ["AWAITING_APPROVAL", "VERIFIED", "BLOCKED"] as const)
    for (const action of ["execute", "integrate", "continue"] as const)
      expect(can({ ...r, status }, action)).toBe(false);
});

it.each(["conflict", "disconnected"])(
  "refreshes definitive %s proposals while preserving uncertain consent identity",
  async (code) => {
    localStorage.clear();
    const request = vi.fn().mockImplementation((req) =>
      req.type === "propose_delegation"
        ? Promise.resolve({ ...contract, request: req.request })
        : Promise.reject({
            code,
            message:
              code === "conflict"
                ? "Base changed; create a new proposal"
                : "Response interrupted",
          }),
    );
    setup(
      <BuildFeature connected onCreated={vi.fn()} onSupervised={vi.fn()} />,
      request,
    );
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Change repository" }));
    await user.type(
      screen.getByRole("textbox", { name: "Feature request" }),
      "CSV export",
    );
    await user.click(screen.getByRole("button", { name: "Build feature" }));
    await screen.findByRole("dialog", { name: "Authorize this feature" });
    await user.click(
      screen.getByRole("button", { name: "Authorize and build" }),
    );
    await screen.findByText(
      code === "conflict"
        ? "Base changed; create a new proposal"
        : "Response interrupted",
    );
    await user.click(screen.getByRole("button", { name: "Back" }));
    await user.click(screen.getByRole("button", { name: "Build feature" }));
    await screen.findByRole("dialog", { name: "Authorize this feature" });
    const proposals = request.mock.calls.filter(
      (c) => c[0].type === "propose_delegation",
    );
    expect(proposals).toHaveLength(2);
    if (code === "conflict")
      expect(proposals[1][0].request.submission.id).not.toBe(
        proposals[0][0].request.submission.id,
      );
    else {
      expect(proposals[1][0].request.submission.id).toBe(
        proposals[0][0].request.submission.id,
      );
      expect(
        JSON.parse(localStorage.getItem("autono.build.pending")!)
          .authorization_attempted,
      ).toBe(true);
    }
  },
);

it("shows essential preplanning holds as blocked and offers the daemon's supported refresh", () => {
  const r = run({
    status: "AWAITING_APPROVAL",
    plan: null,
    planner: null,
    delegation: { contract, authorized_at: "now", expires_at: "later" },
    handoff: {
      outcome: "blocked",
      summary: "Essential provider prerequisite missing",
      tasks: [],
      needs_you: [
        {
          action_id: "H-provider",
          action_version: 1,
          category: "essential_before_start",
          affected_tasks: [],
          waiting_descendants: [],
          requested_action: "Configure the supported provider externally",
        },
      ],
      combined_checks_passed: false,
      completion_requirements_satisfied: false,
      internal_decision_ids: [],
    },
  });
  setup(
    <FeatureSummary
      run={r}
      connected
      onDetails={vi.fn()}
      onHuman={vi.fn()}
      onCancel={vi.fn()}
      onRefresh={vi.fn()}
    />,
    vi.fn().mockResolvedValue([]),
  );
  expect(
    screen.getByRole("heading", {
      name: "Needs your input before work can continue",
    }),
  ).toBeInTheDocument();
  expect(screen.queryByText("Building your feature")).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Check repaired prerequisites" }),
  ).toBeEnabled();
  expect(can({ ...r, delegation: null }, "preflight")).toBe(false);
  expect(can({ ...r, status: "INTERRUPTED" }, "preflight")).toBe(false);
});
