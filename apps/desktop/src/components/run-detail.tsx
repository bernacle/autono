import { useCallback, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import * as Tabs from "@radix-ui/react-tabs";
import {
  ArrowLeft,
  ArrowRight,
  Folder,
  GitBranch,
  Play,
  RefreshCw,
} from "lucide-react";
import type { Run, RunAction } from "../lib/types";
import { useTransport } from "../lib/transport";
import { can, canResolve, date, pending, short, title } from "../lib/utils";
import {
  actionLabels,
  actionReason,
  basename,
  blockerLabel,
  primaryAction,
} from "../lib/presentation";
import { Badge, Empty, ErrorNotice, Panel } from "./common";
import { Button } from "./ui/button";
import { useExecution } from "../lib/execution";
import { ExecutionView, VerificationLedger } from "./execution";
import { LogDock } from "./log-dock";
import { Menu } from "./ui/menu";
import { TasksView } from "./tasks";
import { Lifecycle } from "./lifecycle";
import { PreflightChecklist } from "./preflight";
import { HumanActionCard } from "./human-action";
import { ConfirmAction } from "./confirm-action";
import { SessionRow, TaskEvidence, IntegrationEvidence } from "./evidence";
import { RunGuide } from "./new-run";
export function RunDetail({
  id,
  connected,
  onBack,
  onHuman,
  initialTab,
}: {
  id: string;
  connected: boolean;
  onBack: () => void;
  onHuman?: (runId: string, actionId: string) => void;
  initialTab?: string;
}) {
  const transport = useTransport(),
    client = useQueryClient();
  const query = useQuery({
    queryKey: ["run", id],
    queryFn: () => transport.request<Run>({ type: "run", id }),
    refetchInterval: 2500,
    retry: false,
  });
  const execution = useExecution(query.data);
  const [selectedTask, setSelectedTask] = useState<string | null>(() =>
    new URLSearchParams(window.location.hash.slice(1)).get("run") === id
      ? new URLSearchParams(window.location.hash.slice(1)).get("task")
      : null,
  );
  const [selectedPhase, setSelectedPhase] = useState<string | null>(null);
  const [taskView, setTaskView] = useState<"list" | "graph">("list");
  const selectTask = useCallback(
    (task: string | null) => {
      setSelectedTask(task);
      setSelectedPhase(null);
      const hash = new URLSearchParams();
      if (task) {
        hash.set("run", id);
        hash.set("task", task);
      }
      const url = new URL(window.location.href);
      url.hash = hash.toString();
      // Preserve the exact custom-scheme path (WKWebView may have an empty path).
      window.history.replaceState(null, "", url.href);
    },
    [id],
  );
  const [confirmation, setConfirmation] = useState<{
      run: Run;
      action: RunAction;
    } | null>(null),
    [notice, setNotice] = useState(""),
    [process, setProcess] = useState<string | null>(null),
    [chosenTab, setTab] = useState<string | null>(initialTab ?? null);
  const preflight = useMutation({
    mutationFn: () =>
      transport.request({
        type: "action",
        id,
        revision: query.data!.revision,
        action: "preflight",
      }),
    retry: false,
    onSuccess: async () => {
      await client.invalidateQueries({
        predicate: (q) => !["events", "logs"].includes(String(q.queryKey[0])),
      });
      setNotice("Daemon refreshed preflight.");
    },
    onError: () => {
      void query.refetch();
    },
  });
  if (!query.data)
    return (
      <div className="page-content">
        {query.isError ? (
          <ErrorNotice error={query.error} retry={() => void query.refetch()} />
        ) : (
          <Empty title="Loading engineering run…" />
        )}
        <Button variant="ghost" onClick={onBack}>
          <ArrowLeft size={14} />
          All runs
        </Button>
      </div>
    );
  const run = query.data,
    enabled = connected && !query.isError,
    next = primaryAction(run),
    reason = next ? actionReason(run, next, enabled) : "";
  const ask = (action: RunAction) => {
    setProcess(null);
    setNotice("");
    setConfirmation({ run, action });
  };
  const tab =
    chosenTab ?? (run.status === "AWAITING_APPROVAL" ? "preflight" : "tasks");
  const human = run.human_actions.filter(pending),
    needsHuman = canResolve(run) && human.length > 0;
  const openHuman = (actionId: string) =>
    onHuman ? onHuman(id, actionId) : setTab("preflight");
  const callout = needsHuman
    ? {
        headline: "A human decision is needed",
        text: `${human.length} unresolved ${human.length === 1 ? "action" : "actions"}. Inspect the exact request before resolving.`,
        button: "Review human actions",
        action: () => openHuman(human[0].id),
        tone: "warning",
      }
    : run.status === "BLOCKED"
      ? {
          headline: "This run is blocked",
          text: "Inspect the recorded blockers and preserved evidence before taking action.",
          button: "Inspect blockers",
          action: () => setTab("preflight"),
          tone: "neutral",
        }
      : run.status === "FAILED"
        ? {
            headline: "This run failed",
            text: "Preserved verification and review evidence explains where execution stopped.",
            button: "Inspect evidence",
            action: () => setTab("verification"),
            tone: "error",
          }
        : run.status === "INTERRUPTED"
          ? {
              headline: "This run was interrupted",
              text: "The daemon does not support resuming interrupted workflows. Inspect preserved evidence and create a new plan.",
              button: "Inspect workspaces",
              action: () => setTab("integration"),
              tone: "neutral",
            }
          : null;
  const menu = [
    {
      label: "Refresh preflight",
      disabled: !enabled || !can(run, "preflight") || preflight.isPending,
      onSelect: () => preflight.mutate(),
    },
    {
      label: "Copy run ID",
      onSelect: () => {
        void navigator.clipboard
          .writeText(id)
          .catch(() =>
            setNotice("Copy unavailable. The full run ID is shown in Plan."),
          );
      },
    },
    { label: "Open original specification", onSelect: () => setTab("plan") },
    {
      label: "Cancel run…",
      disabled: !enabled || !can(run, "cancel"),
      destructive: true,
      onSelect: () => ask("cancel"),
    },
  ];
  return (
    <div className="run-detail">
      <div className="run-header">
        <div className="run-heading">
          <div className="run-title-row">
            <h1>{title(run)}</h1>
            <Badge status={run.status} />
          </div>
          <div className="run-meta">
            <span title={run.repository}>
              <Folder size={13} />
              {basename(run.repository)}
            </span>
            <span title={run.base_commit}>
              <GitBranch size={13} />
              {run.requested_base} @ {short(run.base_commit)}
            </span>
            <span>revision {run.revision}</span>
            <span>
              {run.approved_at
                ? `Approved ${date(run.approved_at)}`
                : "Not approved"}
            </span>
            <span>Updated {date(run.updated_at)}</span>
          </div>
        </div>
        <div className="run-header-actions">
          <Menu items={menu} />
          {next && (
            <Button
              aria-describedby={reason ? "run-action-reason" : undefined}
              disabled={!!reason}
              onClick={() => ask(next)}
            >
              <Play size={13} />
              {actionLabels[next]}
            </Button>
          )}
        </div>
        {reason && (
          <p id="run-action-reason" className="primary-reason">
            {reason}
          </p>
        )}
        <Lifecycle run={run} />
      </div>
      {connected && query.isError && (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      )}
      {notice && (
        <div className="notice" role="status">
          {notice}
        </div>
      )}
      {preflight.isError && (
        <ErrorNotice error={preflight.error} retry={() => preflight.mutate()} />
      )}
      {callout && (
        <div className={`state-callout notice notice-${callout.tone}`}>
          <div>
            <strong>{callout.headline}</strong>
            <p>{callout.text}</p>
          </div>
          <Button variant="secondary" onClick={callout.action}>
            {callout.button}
            <ArrowRight size={13} />
          </Button>
        </div>
      )}
      <Tabs.Root value={tab} onValueChange={setTab} className="run-tabs">
        <Tabs.List className="tabs" aria-label="Run details">
          {[
            ["tasks", "Tasks"],
            ["execution", "Execution"],
            ["verification", "Verification"],
            ["plan", "Plan"],
            ["preflight", "Preflight"],
            ["integration", "Integration"],
          ].map(([value, label]) => (
            <Tabs.Trigger key={value} value={value}>
              {label}
              {value === "tasks" && <span>{run.tasks.length}</span>}
              {value === "preflight" && needsHuman && (
                <span className="warning-text">{human.length}</span>
              )}
            </Tabs.Trigger>
          ))}
        </Tabs.List>
        <Tabs.Content value="tasks" className="tasks-tab">
          <TasksView
            key={taskView}
            run={run}
            selected={selectedTask}
            onSelect={selectTask}
            initialView={taskView}
            logDockOpen={!!process}
            onProcess={setProcess}
            onHuman={openHuman}
            onExecution={() => setTab("execution")}
          />
          {run.status === "PLANNING" && (
            <Panel title="Planner session">
              <SessionRow session={run.planner} onProcess={setProcess} />
            </Panel>
          )}
        </Tabs.Content>
        <Tabs.Content value="execution" className="tab-content">
          {execution.query.isError && (
            <ErrorNotice
              error={execution.query.error}
              retry={() => void execution.query.refetch()}
            />
          )}
          <ExecutionView
            run={run}
            model={execution.model}
            history={execution.query.data}
            selected={selectedTask}
            phaseId={selectedPhase}
            onSelect={selectTask}
            onPhase={setSelectedPhase}
            onProcess={setProcess}
            onGraph={() => {
              setTaskView("graph");
              setTab("tasks");
            }}
            onHuman={openHuman}
            connected={enabled && !execution.query.isError}
            logDockOpen={!!process}
          />
        </Tabs.Content>
        <Tabs.Content value="verification" className="tab-content">
          <VerificationLedger
            run={run}
            onTask={(task) => {
              selectTask(task);
              setTab("tasks");
            }}
            onProcess={setProcess}
          />
          <TaskEvidence run={run} onProcess={setProcess} />
        </Tabs.Content>
        <Tabs.Content value="plan" className="tab-content">
          <Panel title="Original specification">
            <pre className="specification">{run.specification}</pre>
          </Panel>
          {run.plan && (
            <>
              <Panel title="Requirements">
                <div className="requirements">
                  {run.plan.requirements.map((r) => (
                    <p key={r.id}>
                      <code>{r.id}</code>
                      {r.text}
                    </p>
                  ))}
                </div>
              </Panel>
              <Panel title="Milestones">
                <div className="milestones">
                  {run.plan.milestones.map((m) => (
                    <div key={m.id}>
                      <code>{m.id}</code>
                      <strong>{m.goal}</strong>
                      <span>{m.task_ids.join(" · ")}</span>
                    </div>
                  ))}
                </div>
              </Panel>
            </>
          )}
          <details className="audit">
            <summary>Run identity and daemon summary</summary>
            <p>{id}</p>
            <p>{run.summary}</p>
          </details>
        </Tabs.Content>
        <Tabs.Content value="preflight" className="tab-content">
          <div
            className={`readiness-layout ${run.status === "AWAITING_APPROVAL" ? "guided" : ""}`}
          >
            {run.status === "AWAITING_APPROVAL" && <RunGuide readiness />}
            <div className="readiness-main">
              <h2>Readiness</h2>
              <p className="muted">
                Review the plan, resolve prerequisites, then explicitly approve
                execution.
              </p>
              <Panel
                title="Plan"
                meta={
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setTab("plan")}
                  >
                    Inspect plan
                    <ArrowRight size={13} />
                  </Button>
                }
              >
                <p className="task-content">
                  {run.plan
                    ? `${run.tasks.length} tasks · ${run.plan.milestones.length} milestones · ${run.plan.requirements.length} requirements`
                    : "The planner has not published a validated plan."}
                </p>
              </Panel>
              <Panel
                title="Preflight"
                meta={
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={
                      !enabled || !can(run, "preflight") || preflight.isPending
                    }
                    onClick={() => preflight.mutate()}
                  >
                    <RefreshCw size={13} />
                    Refresh preflight
                  </Button>
                }
              >
                <PreflightChecklist report={run.preflight} />
              </Panel>
              <Panel
                title="Human actions"
                meta={
                  <span className="meta">Resolving never starts agents</span>
                }
              >
                <div className="human-actions">
                  {run.human_actions.map((a) => (
                    <HumanActionCard
                      key={a.id}
                      run={run}
                      action={a}
                      connected={enabled}
                      onInspect={onHuman ? () => openHuman(a.id) : undefined}
                    />
                  ))}
                  {!run.human_actions.length && (
                    <Empty title="No human actions" />
                  )}
                </div>
              </Panel>
              {run.blockers.length > 0 && (
                <Panel title="Recorded blockers">
                  <div className="blocker-list">
                    {run.blockers.map((b, i) => (
                      <div key={i}>
                        <strong>
                          {b.task_id && `${b.task_id} · `}
                          {blockerLabel(b.kind)}
                        </strong>
                        <p>{b.detail}</p>
                        <p>{b.action}</p>
                      </div>
                    ))}
                  </div>
                </Panel>
              )}
              <details className="audit">
                <summary>Daemon summary</summary>
                <p>{run.summary}</p>
              </details>
            </div>
            <aside className="readiness-rail">
              <h3>What approving authorizes</h3>
              <p>
                Coding agents, configured checks and independent reviews in
                assigned local worktrees.
              </p>
              <p className="meta">No merge to main, push or deployment.</p>
              <dl className="settings-dl">
                <dt>Parallel tasks</dt>
                <dd>{run.settings.execution.max_parallel}</dd>
                <dt>Attempts</dt>
                <dd>{run.settings.execution.max_attempts}</dd>
                <dt>Review cycles</dt>
                <dd>{run.settings.execution.max_review_cycles}</dd>
              </dl>
              <p className="meta">
                {actionReason(run, "execute", enabled) ||
                  "Ready for explicit approval. Use Approve execution in the run header."}
              </p>
            </aside>
          </div>
        </Tabs.Content>
        <Tabs.Content value="integration" className="tab-content">
          <IntegrationEvidence run={run} onProcess={setProcess} />
        </Tabs.Content>
      </Tabs.Root>
      {confirmation && (
        <ConfirmAction
          {...confirmation}
          connected={enabled}
          onClose={() => setConfirmation(null)}
          onAccepted={setNotice}
        />
      )}
      {process && <LogDock id={process} onClose={() => setProcess(null)} />}
    </div>
  );
}
