import { useCallback, useEffect, useRef, useState } from "react";
import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  ArrowLeft,
  ArrowRight,
  ChevronDown,
  Command,
  GitBranch,
  LayoutGrid,
  Plus,
  Search,
  Settings,
  ShieldAlert,
  TerminalSquare,
  WifiOff,
} from "lucide-react";
import { useTransport } from "./lib/transport";
import { useAppearance } from "./lib/appearance";
import type { Connection, Process, Run } from "./lib/types";
import {
  can,
  canResolve,
  date,
  errorMessage,
  pending,
  short,
  title,
} from "./lib/utils";
import { basename, clock, duration } from "./lib/presentation";
import { Button } from "./components/ui/button";
import {
  Badge,
  Empty,
  ErrorNotice,
  Panel,
  StatusGlyph,
} from "./components/common";
import { BuildFeature } from "./components/build-feature";
import { NewRun } from "./components/new-run";
import { RunDetail } from "./components/run-detail";
import { SettingsView } from "./components/settings";
import { HumanActionsView } from "./components/human-action";
import { AgentProgress } from "./components/agent-progress";
import { Journal } from "./components/journal";
import { Timeline } from "./components/execution";
import { LogDock } from "./components/log-dock";
import { useExecution, useNow, elapsed, timeLabel } from "./lib/execution";

type Page = "build" | "overview" | "runs" | "human" | "settings" | "processes";
const pages = [
  { id: "build", label: "Build", icon: Plus },
  { id: "overview", label: "Overview", icon: LayoutGrid },
  { id: "human", label: "Human actions", icon: ShieldAlert },
  { id: "runs", label: "Runs", icon: GitBranch },
] as const;
export default function App() {
  const body = useRef<HTMLElement>(null);
  const transport = useTransport(),
    client = useQueryClient(),
    appearance = useAppearance();
  const [page, setPage] = useState<Page>("build"),
    [selected, setSelected] = useState<string | null>(null),
    [runTab, setRunTab] = useState<string | undefined>(),
    [newRun, setNewRun] = useState(false),
    [supervised, setSupervised] = useState(false),
    [process, setProcess] = useState<string | null>(null),
    [humanFocus, setHumanFocus] = useState<string | null>(null),
    [rail, setRail] = useState<boolean | null>(null),
    [commands, setCommands] = useState(false);
  useEffect(() => {
    if (body.current) body.current.scrollTop = 0;
  }, [page, selected, newRun]);
  const connection = useQuery({
    queryKey: ["connection"],
    queryFn: () => transport.request<Connection>({ type: "status" }),
    refetchInterval: 3000,
    retry: false,
  });
  const runsQuery = useInfiniteQuery({
    queryKey: ["runs"],
    queryFn: ({ pageParam }) =>
      transport.request<Run[]>({ type: "runs", offset: pageParam }),
    initialPageParam: 0,
    getNextPageParam: (last, pages) =>
      last.length === 20 ? pages.length * 20 : undefined,
    refetchInterval: 3500,
    retry: false,
  });
  const processes = useQuery({
    queryKey: ["processes"],
    queryFn: () =>
      transport.request<Process[]>({ type: "processes", offset: 0 }),
    refetchInterval: 3000,
    retry: false,
  });
  const activeId = connection.data?.engineering.active_run;
  const activeQuery = useQuery({
    queryKey: ["run", activeId],
    queryFn: () => transport.request<Run>({ type: "run", id: activeId! }),
    enabled: !!activeId,
    refetchInterval: 2500,
    retry: false,
  });
  const loaded = [
    ...new Map(
      (runsQuery.data?.pages.flat() ?? []).map((r) => [r.id, r]),
    ).values(),
  ];
  const active = activeId
    ? (activeQuery.data ?? loaded.find((r) => r.id === activeId))
    : undefined;
  const runs = active
    ? [active, ...loaded.filter((r) => r.id !== active.id)]
    : loaded;
  const actions = runs.flatMap((run) =>
    run.human_actions
      .filter((a) => canResolve(run) && pending(a))
      .map((action) => ({ run, action })),
  );
  const connected = !!connection.data && !connection.isError;
  const healthy =
    connected &&
    connection.data!.host.persistence.healthy &&
    !connection.data!.engineering.persistence_error &&
    connection.data!.host.database === "available";
  const navigate = useCallback((next: Page) => {
    setProcess(null);
    setPage(next);
    setSelected(null);
    setNewRun(false);
    setCommands(false);
  }, []);
  const select = useCallback((id: string, tab?: string) => {
    setProcess(null);
    setPage("runs");
    setSelected(id);
    setRunTab(tab);
    setNewRun(false);
    setCommands(false);
  }, []);
  const human = useCallback((runId: string, actionId: string) => {
    setProcess(null);
    setPage("human");
    setSelected(null);
    setNewRun(false);
    setHumanFocus(`${runId}/${actionId}`);
  }, []);
  const reconnect = useCallback(() => {
    void client.invalidateQueries();
  }, [client]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setCommands((v) => !v);
        return;
      }
      if (mod && e.key.toLowerCase() === "r") {
        e.preventDefault();
        reconnect();
        return;
      }
      if (mod && e.key.toLowerCase() === "n") {
        e.preventDefault();
        if (healthy) {
          setSupervised(false);
          setNewRun(true);
          setSelected(null);
          setCommands(false);
        }
        return;
      }
      if (mod && ["1", "2", "3"].includes(e.key)) {
        e.preventDefault();
        navigate(e.key === "1" ? "overview" : e.key === "2" ? "human" : "runs");
        return;
      }
      if (mod && e.key === "\\") {
        e.preventDefault();
        setRail((v) => (v === null ? window.innerWidth >= 1100 : !v));
        return;
      }
      if (mod && e.shiftKey && e.key.toLowerCase() === "c" && selected) {
        e.preventDefault();
        void navigator.clipboard.writeText(selected).catch(() => {});
        return;
      }
      if (e.key === "Escape" && commands) {
        e.preventDefault();
        setCommands(false);
      }
    };
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [healthy, navigate, reconnect, selected, commands]);
  const inProgress = runs.filter((r) =>
    [
      "PLANNING",
      "AWAITING_APPROVAL",
      "RUNNING",
      "BLOCKED",
      "INTEGRATING",
    ].includes(r.status),
  );
  const pageLabel = newRun
    ? supervised
      ? "Supervised mode"
      : "Build"
    : selected
      ? "Runs"
      : page === "human"
        ? "Human actions"
        : page === "processes"
          ? "Processes"
          : page[0].toUpperCase() + page.slice(1);
  const lastUpdated = connection.dataUpdatedAt
    ? clock(connection.dataUpdatedAt)
    : "—";
  return (
    <div
      className="app-shell"
      data-sidebar={rail === null ? "auto" : rail ? "rail" : "full"}
    >
      <div className="app-ground" aria-hidden="true" />
      <aside className="sidebar">
        <div className="window-drag" data-tauri-drag-region />
        <div className="brand">
          <span className="brand-icon">
            <Command size={18} />
          </span>
          <strong>autono</strong>
          <button
            aria-label="Quick navigation"
            className="shortcut-button"
            onClick={() => setCommands((v) => !v)}
          >
            ⌘K
          </button>
        </div>
        <Button
          className="sidebar-new"
          variant="secondary"
          onClick={() => {
            setSupervised(false);
            setNewRun(true);
            setSelected(null);
          }}
          disabled={!healthy}
          title={
            !healthy ? "Reconnect to a healthy daemon" : "Build feature · ⌘N"
          }
        >
          <Plus size={15} />
          <span>Build feature</span>
          <kbd>⌘N</kbd>
        </Button>
        <nav aria-label="Main navigation">
          {pages.map((item) => (
            <button
              key={item.id}
              title={item.label}
              aria-label={item.label}
              aria-current={page === item.id && !newRun ? "page" : undefined}
              onClick={() => navigate(item.id)}
            >
              <item.icon size={15} />
              <span>{item.label}</span>
              {item.id === "human" && actions.length > 0 && (
                <span className="nav-count">{actions.length}</span>
              )}
            </button>
          ))}
        </nav>
        <div className="sidebar-section">In progress</div>
        <nav className="in-progress-nav" aria-label="In progress runs">
          {inProgress.slice(0, 5).map((run) => (
            <button
              title={title(run)}
              key={run.id}
              onClick={() => select(run.id)}
              aria-current={selected === run.id ? "page" : undefined}
            >
              <StatusGlyph status={run.status} />
              <span>{title(run)}</span>
              <small>
                {run.tasks.filter((t) => t.status === "COMPLETED").length}/
                {run.tasks.length}
              </small>
            </button>
          ))}
          {!inProgress.length && (
            <p className="sidebar-empty">No active runs</p>
          )}
        </nav>
        <div className="sidebar-section">Daemon</div>
        <nav aria-label="Daemon navigation">
          <button
            title="Processes"
            aria-label="Processes"
            onClick={() => navigate("processes")}
            aria-current={page === "processes" ? "page" : undefined}
          >
            <TerminalSquare size={15} />
            <span>Processes</span>
          </button>
          <button
            title="Settings"
            aria-label="Settings"
            onClick={() => navigate("settings")}
            aria-current={page === "settings" ? "page" : undefined}
          >
            <Settings size={15} />
            <span>Settings</span>
          </button>
        </nav>
        <div className="sidebar-bottom">
          <details className="daemon-popover">
            <summary
              title={
                healthy
                  ? "Daemon connected"
                  : connected
                    ? "Daemon degraded"
                    : "Daemon unavailable"
              }
            >
              <span
                className={`status-dot ${healthy ? "online" : connected ? "degraded" : "offline"}`}
              />
              <span>
                {healthy
                  ? "Daemon connected"
                  : connected
                    ? "Daemon degraded"
                    : "Daemon unavailable"}
                <small>
                  {connection.data?.address.split(":").at(-1)
                    ? `:${connection.data.address.split(":").at(-1)}`
                    : "Offline"}
                </small>
              </span>
              <ChevronDown size={12} />
            </summary>
            <div className="popover daemon-details">
              <strong>
                {healthy
                  ? "Connected daemon"
                  : connected
                    ? "Degraded daemon"
                    : "Daemon unavailable"}
              </strong>
              <dl>
                <dt>Address</dt>
                <dd>{connection.data?.address ?? "Unavailable"}</dd>
                <dt>Process</dt>
                <dd>
                  {connection.data
                    ? `PID ${connection.data.host.pid} · API v${connection.data.host.api_version}`
                    : "Unknown"}
                </dd>
                <dt>Persistence</dt>
                <dd>
                  {connected
                    ? connection.data!.host.persistence.healthy
                      ? "Healthy"
                      : "Degraded"
                    : "Unknown"}
                </dd>
                <dt>Stay awake</dt>
                <dd>
                  {connected
                    ? connection.data!.host.power.held
                      ? "Held"
                      : "Not held"
                    : "Unknown"}
                </dd>
              </dl>
              <Button variant="secondary" size="sm" onClick={reconnect}>
                Reconnect
              </Button>
              <p>Closing this window leaves runs with the daemon.</p>
            </div>
          </details>
        </div>
      </aside>
      <section className="sheet" aria-label="Mission control content">
        <header className="sheet-header" data-tauri-drag-region>
          <div className="sheet-breadcrumb">
            {selected && (
              <Button
                variant="ghost"
                size="icon"
                aria-label="Back to runs"
                onClick={() => setSelected(null)}
              >
                <ArrowLeft size={14} />
              </Button>
            )}
            <span>{pageLabel}</span>
            {selected && (
              <>
                <span className="muted">/</span>
                <code>{short(selected)}</code>
              </>
            )}
          </div>
          <div className="sheet-header-actions">
            {page === "human" && !selected && !newRun && (
              <span className="meta">Resolving never starts agents</span>
            )}
            {!selected &&
              !newRun &&
              page !== "human" &&
              page !== "settings" && (
                <Button
                  size="sm"
                  onClick={() => setNewRun(true)}
                  disabled={!healthy}
                  aria-describedby={!healthy ? "connection-state" : undefined}
                >
                  <Plus size={13} />
                  Build feature
                </Button>
              )}
          </div>
        </header>
        {!healthy && (
          <div
            className={`connection-banner ${connected ? "degraded" : "disconnected"}`}
            id="connection-state"
            role="alert"
          >
            <WifiOff size={16} />
            <div>
              <strong>
                {connected
                  ? "Daemon degraded"
                  : "Daemon connection unavailable"}
              </strong>
              <p>
                {connected
                  ? (connection.data!.engineering.persistence_error ??
                    `Database: ${connection.data!.host.database}; persistence ${connection.data!.host.persistence.healthy ? "healthy" : "unhealthy"}`)
                  : connection.isPending
                    ? "Checking local connection metadata…"
                    : errorMessage(connection.error)}
              </p>
              {connection.data && (
                <span className="meta">
                  Last updated {lastUpdated} · saved state is read-only
                </span>
              )}
            </div>
            <Button variant="secondary" size="sm" onClick={reconnect}>
              Reconnect
            </Button>
          </div>
        )}
        <main
          ref={body}
          className={`sheet-body ${page === "human" && !selected && !newRun ? "human-sheet" : ""}`}
          data-stale={!healthy && !!connection.data}
        >
          {newRun && supervised ? (
            <NewRun
              connected={healthy}
              activeRun={!!activeId}
              onClose={() => {
                setNewRun(false);
                setSupervised(false);
              }}
              onCreated={select}
            />
          ) : selected ? (
            <RunDetail
              key={selected}
              id={selected}
              connected={healthy}
              onBack={() => setSelected(null)}
              onHuman={human}
              initialTab={runTab}
            />
          ) : newRun || page === "build" ? (
            <BuildFeature
              connected={healthy}
              active={active}
              recent={runs.find((r) => !!r.delegation)}
              onCreated={(id) => {
                setNewRun(false);
                setSupervised(false);
                select(id);
              }}
              onSupervised={() => {
                setSupervised(false);
                setNewRun(true);
                setSupervised(true);
              }}
            />
          ) : page === "settings" ? (
            <SettingsView
              connection={connected ? connection.data : undefined}
              appearance={appearance}
            />
          ) : page === "human" ? (
            <HumanActionsView
              key={humanFocus ?? "queue"}
              runs={runs}
              connected={healthy && !runsQuery.isError}
              initial={humanFocus}
              onRun={select}
              onOlder={
                runsQuery.hasNextPage
                  ? () => void runsQuery.fetchNextPage()
                  : undefined
              }
              loadingOlder={runsQuery.isFetchingNextPage}
            />
          ) : (
            <div className="page-content">
              {healthy && page === "overview" && processes.isError && (
                <ErrorNotice
                  error={processes.error}
                  retry={() => void processes.refetch()}
                />
              )}
              {healthy && runsQuery.isError && (
                <ErrorNotice
                  error={runsQuery.error}
                  retry={() => void runsQuery.refetch()}
                />
              )}
              {page === "overview" ? (
                <Overview
                  active={active}
                  activeId={activeId}
                  runs={runs}
                  processes={processes.data ?? []}
                  connection={connected ? connection.data : undefined}
                  healthy={healthy}
                  select={select}
                  onHuman={human}
                  onNew={() => setNewRun(true)}
                  onProcess={setProcess}
                />
              ) : page === "processes" ? (
                <Panel
                  title="Recent daemon processes"
                  meta={<span className="meta">Latest 50</span>}
                >
                  {healthy && processes.isError && (
                    <ErrorNotice
                      error={processes.error}
                      retry={() => void processes.refetch()}
                    />
                  )}
                  <ProcessList
                    processes={processes.data ?? []}
                    onProcess={setProcess}
                  />
                </Panel>
              ) : (
                <Panel
                  title="Engineering runs"
                  meta={<span className="meta">{runs.length} shown</span>}
                >
                  <RunList runs={runs} select={select} />
                </Panel>
              )}
              {healthy && activeQuery.isError && page === "overview" && (
                <ErrorNotice
                  error={activeQuery.error}
                  retry={() => void activeQuery.refetch()}
                />
              )}
              {page !== "processes" && (
                <div className="pagination-note">
                  <span className="meta">
                    Showing {runs.length} recent runs
                    {runsQuery.hasNextPage
                      ? " · attention queue covers these runs"
                      : ""}
                  </span>
                  {runsQuery.hasNextPage && (
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={runsQuery.isFetchingNextPage}
                      onClick={() => void runsQuery.fetchNextPage()}
                    >
                      Load older
                    </Button>
                  )}
                </div>
              )}
            </div>
          )}
        </main>
      </section>
      <div className="sr-only" role="status" aria-live="polite">
        {actions.length} human actions need attention.{" "}
        {active
          ? `${title(active)}: ${active.status.toLowerCase().replaceAll("_", " ")}.`
          : "No active engineering run."}
      </div>
      {commands && (
        <QuickCommands
          onClose={() => setCommands(false)}
          navigate={navigate}
          newRun={() => {
            setCommands(false);
            setSupervised(false);
            setNewRun(true);
          }}
          healthy={healthy}
          reconnect={reconnect}
        />
      )}
      {process && <LogDock id={process} onClose={() => setProcess(null)} />}
    </div>
  );
}
function Overview({
  active,
  activeId,
  runs,
  processes,
  connection,
  healthy,
  select,
  onHuman,
  onNew,
  onProcess,
}: {
  active: Run | undefined;
  activeId: string | null | undefined;
  runs: Run[];
  processes: Process[];
  connection: Connection | undefined;
  healthy: boolean;
  select: (id: string, tab?: string) => void;
  onHuman: (runId: string, actionId: string) => void;
  onNew: () => void;
  onProcess: (id: string) => void;
}) {
  const working =
    active ??
    runs.find((r) =>
      ["BLOCKED", "VERIFIED", "AWAITING_APPROVAL"].includes(r.status),
    );
  const now = useNow();
  const observed = working ?? runs[0],
    execution = useExecution(observed);
  const actions = runs.flatMap((run) =>
    run.human_actions
      .filter((a) => canResolve(run) && pending(a))
      .map((action) => ({ run, action })),
  );
  const approvals = runs.filter((r) => can(r, "execute")),
    integrable = runs.filter((r) => can(r, "integrate")),
    continuable = runs.filter((r) => can(r, "continue"));
  const sessions =
    execution.model?.segments.filter((s) => s.live && s.session) ?? [];
  return (
    <div className="overview-layout">
      <div className="overview-main">
        <Panel
          title={
            working?.status === "BLOCKED"
              ? "Awaiting attention"
              : working?.status === "VERIFIED"
                ? "Ready to integrate"
                : "Working now"
          }
          meta={
            working && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => select(working.id)}
              >
                Open run
                <ArrowRight size={13} />
              </Button>
            )
          }
        >
          {working ? (
            <div className="working-now">
              <div className="working-header">
                <div>
                  <div className="run-title-row">
                    <h2>{title(working)}</h2>
                    <Badge status={working.status} />
                  </div>
                  <p className="meta">
                    {basename(working.repository)} · {working.requested_base} @{" "}
                    {short(working.base_commit)}
                  </p>
                </div>
                <div className="working-duration">
                  <span className="meta">Since approval</span>
                  <strong>
                    {working.approved_at
                      ? duration((now - Date.parse(working.approved_at)) / 1000)
                      : "Not approved"}
                  </strong>
                </div>
              </div>
              {execution.model && (
                <Timeline
                  mini
                  run={working}
                  model={execution.model}
                  selected={null}
                  onTask={() => select(working.id, "execution")}
                  onPhase={() => select(working.id, "execution")}
                  onGraph={() => select(working.id, "tasks")}
                  connected={healthy && !execution.query.isError}
                />
              )}
              <div className="overview-agents">
                {sessions.length ? (
                  sessions.map((s) => (
                    <AgentProgress
                      key={s.id}
                      segment={s}
                      onProcess={onProcess}
                      connected={healthy}
                    />
                  ))
                ) : (
                  <p className="quiet-row">
                    No agent start is currently open in the journal.
                  </p>
                )}
              </div>
              <div className="overview-progress">
                <span>
                  {
                    working.tasks.filter((t) =>
                      ["BLOCKED", "BLOCKED_HUMAN"].includes(t.status),
                    ).length
                  }{" "}
                  blocked tasks
                </span>
                <span>
                  Last recorded progress:{" "}
                  {execution.model?.lastProgress
                    ? `${timeLabel(execution.model.lastProgress)} · ${elapsed(now - execution.model.lastProgress)} ago`
                    : "None recorded"}
                </span>
              </div>
              <div className="working-footer">
                <span>
                  {sessions.length} recorded live agents ·{" "}
                  {working.tasks.filter((t) => t.status === "COMPLETED").length}
                  /{working.tasks.length} tasks completed
                </span>
                <span>Updated {clock(working.updated_at)}</span>
              </div>
            </div>
          ) : (
            <Empty
              title={activeId ? "Loading active run…" : "Nothing is running"}
            >
              <span>Start a run or approve one that's ready.</span>
              <Button variant="secondary" disabled={!healthy} onClick={onNew}>
                Build feature
              </Button>
            </Empty>
          )}
        </Panel>
        {observed && (
          <Panel
            title="Meaningful activity"
            meta={
              <Button
                variant="ghost"
                size="sm"
                onClick={() => select(observed.id, "execution")}
              >
                Full journal
                <ArrowRight size={13} />
              </Button>
            }
          >
            {execution.query.isError && healthy && (
              <ErrorNotice
                error={execution.query.error}
                retry={() => void execution.query.refetch()}
              />
            )}
            {observed && (
              <Journal
                run={observed}
                events={execution.query.data?.recent ?? []}
                cursor={execution.query.data?.cursor ?? 0}
                compact
              />
            )}
          </Panel>
        )}
        <Panel
          title="Recent runs"
          meta={<span className="meta">Most recent</span>}
        >
          <RunList runs={runs.slice(0, 6)} select={select} />
        </Panel>
        {processes.some(
          (p) => p.runtime?.phase === "RUNNING" || p.status === "RUNNING",
        ) && (
          <Panel title="Running daemon processes">
            <ProcessList
              processes={processes
                .filter(
                  (p) =>
                    p.runtime?.phase === "RUNNING" || p.status === "RUNNING",
                )
                .slice(0, 8)}
              onProcess={onProcess}
            />
          </Panel>
        )}
      </div>
      <aside className="overview-rail">
        <Panel
          title="Needs you"
          meta={
            <span className="count">
              {actions.length +
                approvals.length +
                integrable.length +
                continuable.length}
            </span>
          }
        >
          <div className="attention-list">
            {actions.map(({ run, action }) => (
              <button
                key={`${run.id}-${action.id}`}
                onClick={() => onHuman(run.id, action.id)}
              >
                <div>
                  <span className="warning-text meta">
                    {action.requirement.kind.replaceAll("_", " ")} ·{" "}
                    {action.requirement.blocking ? "Blocking" : "Advisory"}
                  </span>
                  <strong>{action.requirement.requested_action}</strong>
                  <small>
                    {title(run)} ·{" "}
                    {action.requirement.affected_tasks.join(", ") ||
                      "Whole run"}
                  </small>
                </div>
                <ArrowRight size={13} />
              </button>
            ))}
            {[
              ...approvals.map((run) => ({ run, label: "Execution approval" })),
              ...integrable.map((run) => ({
                run,
                label: "Ready to integrate",
              })),
              ...continuable.map((run) => ({
                run,
                label: "Ready to continue",
              })),
            ].map(({ run, label }) => (
              <button key={`${run.id}-${label}`} onClick={() => select(run.id)}>
                <div>
                  <span className="meta">{label}</span>
                  <strong>{title(run)}</strong>
                  {label === "Execution approval" &&
                    activeId &&
                    activeId !== run.id && (
                      <small>Can start after the active run finishes</small>
                    )}
                </div>
                <ArrowRight size={13} />
              </button>
            ))}
            {!actions.length &&
              !approvals.length &&
              !integrable.length &&
              !continuable.length && (
                <p className="quiet-row">No decisions waiting in these runs.</p>
              )}
          </div>
        </Panel>
        <Panel title="Daemon">
          <dl className="daemon-facts">
            <dt>Health</dt>
            <dd>
              {healthy ? "Connected" : connection ? "Degraded" : "Unavailable"}
            </dd>
            <dt>Processes</dt>
            <dd>
              {connection
                ? `${connection.host.admission.executing_runs} of ${connection.host.admission.max_active_runs}`
                : "—"}
            </dd>
            <dt>Stay awake</dt>
            <dd>
              {connection
                ? connection.host.power.held
                  ? "Held"
                  : "Not held"
                : "Unknown"}
            </dd>
          </dl>
        </Panel>
      </aside>
    </div>
  );
}
function RunList({
  runs,
  select,
}: {
  runs: Run[];
  select: (id: string) => void;
}) {
  return (
    <div
      className="run-list"
      onKeyDown={(e) => {
        if (!["j", "k", "ArrowDown", "ArrowUp"].includes(e.key)) return;
        const buttons = [...e.currentTarget.querySelectorAll("button")],
          index = buttons.indexOf(document.activeElement as HTMLButtonElement);
        if (index < 0) return;
        e.preventDefault();
        buttons[
          Math.max(
            0,
            Math.min(
              buttons.length - 1,
              index + (["j", "ArrowDown"].includes(e.key) ? 1 : -1),
            ),
          )
        ]?.focus();
      }}
    >
      {runs.map((run) => (
        <button key={run.id} onClick={() => select(run.id)}>
          <StatusGlyph status={run.status} />
          <div>
            <strong>{title(run)}</strong>
            <small>
              {basename(run.repository)} · {short(run.id)}
            </small>
          </div>
          <span className="mono muted">
            {run.tasks.filter((t) => t.status === "COMPLETED").length}/
            {run.tasks.length}
          </span>
          <Badge status={run.status} />
          <time>{date(run.updated_at)}</time>
          <ArrowRight size={13} />
        </button>
      ))}
      {!runs.length && (
        <Empty title="No engineering runs yet">
          Select a repository and submit a feature specification.
        </Empty>
      )}
    </div>
  );
}
function ProcessList({
  processes,
  onProcess,
}: {
  processes: Process[];
  onProcess: (id: string) => void;
}) {
  return (
    <div className="process-list">
      {processes.map((p) => (
        <button key={p.id} onClick={() => onProcess(p.id)}>
          <TerminalSquare size={15} />
          <div>
            <strong>
              {p.program ?? "Process"} <code>{short(p.id)}</code>
            </strong>
            <small>{p.cwd}</small>
          </div>
          <span className="mono meta">{p.pid ? `PID ${p.pid}` : "No PID"}</span>
          <Badge status={p.runtime?.phase ?? p.status} />
          <ArrowRight size={13} />
        </button>
      ))}
      {!processes.length && <Empty title="No recorded processes" />}
    </div>
  );
}
function QuickCommands({
  onClose,
  navigate,
  newRun,
  healthy,
  reconnect,
}: {
  onClose: () => void;
  navigate: (page: Page) => void;
  newRun: () => void;
  healthy: boolean;
  reconnect: () => void;
}) {
  const [search, setSearch] = useState(""),
    input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    input.current?.focus();
    return () => previous?.focus();
  }, []);
  const items = [
    ...pages.map((p) => ({
      label: p.label,
      action: () => navigate(p.id),
      disabled: false,
    })),
    { label: "Settings", action: () => navigate("settings"), disabled: false },
    {
      label: "Processes",
      action: () => navigate("processes"),
      disabled: false,
    },
    { label: "Build feature", action: newRun, disabled: !healthy },
    {
      label: "Reconnect daemon",
      action: () => {
        reconnect();
        onClose();
      },
      disabled: false,
    },
  ].filter((i) => i.label.toLowerCase().includes(search.toLowerCase()));
  return (
    <div className="quick-navigation-wrap">
      <button
        tabIndex={-1}
        className="quick-scrim"
        aria-label="Close quick navigation"
        onClick={onClose}
      />
      <section
        className="popover quick-navigation"
        role="dialog"
        aria-modal="true"
        aria-label="Quick navigation"
        onKeyDown={(e) => {
          const nodes = [
            ...e.currentTarget.querySelectorAll<HTMLElement>(
              "input,button:not(:disabled)",
            ),
          ];
          const current = nodes.indexOf(document.activeElement as HTMLElement);
          if (
            e.key === "Tab" &&
            ((e.shiftKey && current === 0) ||
              (!e.shiftKey && current === nodes.length - 1))
          ) {
            e.preventDefault();
            nodes[e.shiftKey ? nodes.length - 1 : 0]?.focus();
          }
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            nodes[
              Math.max(
                0,
                Math.min(
                  nodes.length - 1,
                  current + (e.key === "ArrowDown" ? 1 : -1),
                ),
              )
            ]?.focus();
          }
        }}
      >
        <label className="quick-search">
          <Search size={16} />
          <input
            ref={input}
            aria-label="Find a page or safe command"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                items.find((item) => !item.disabled)?.action();
              }
            }}
            placeholder="Go to a page or safe command…"
          />
        </label>
        <div>
          {items.map((i) => (
            <button key={i.label} disabled={i.disabled} onClick={i.action}>
              {i.label}
              {i.disabled && <small>Daemon unavailable</small>}
            </button>
          ))}
        </div>
      </section>
    </div>
  );
}
