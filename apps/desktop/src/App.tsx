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
import type { Connection, Process, Run, Session } from "./lib/types";
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
import { Dialog } from "./components/ui/dialog";
import {
  Badge,
  Empty,
  ErrorNotice,
  Panel,
  StatusGlyph,
} from "./components/common";
import { NewRun } from "./components/new-run";
import { RunDetail } from "./components/run-detail";
import { SettingsView } from "./components/settings";
import { HumanActionsView } from "./components/human-action";
import { Activity, ProcessLogs } from "./components/activity";
import { SessionRow } from "./components/evidence";
type Page = "overview" | "runs" | "human" | "settings" | "processes";
const pages = [
  { id: "overview", label: "Overview", icon: LayoutGrid },
  { id: "human", label: "Human actions", icon: ShieldAlert },
  { id: "runs", label: "Runs", icon: GitBranch },
] as const;
export default function App() {
  const transport = useTransport(),
    client = useQueryClient(),
    appearance = useAppearance();
  const [page, setPage] = useState<Page>("overview"),
    [selected, setSelected] = useState<string | null>(null),
    [runTab, setRunTab] = useState<string | undefined>(),
    [newRun, setNewRun] = useState(false),
    [process, setProcess] = useState<string | null>(null),
    [humanFocus, setHumanFocus] = useState<string | null>(null),
    [rail, setRail] = useState<boolean | null>(null),
    [commands, setCommands] = useState(false);
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
    setPage(next);
    setSelected(null);
    setNewRun(false);
    setCommands(false);
  }, []);
  const select = useCallback((id: string, tab?: string) => {
    setPage("runs");
    setSelected(id);
    setRunTab(tab);
    setNewRun(false);
    setCommands(false);
  }, []);
  const human = useCallback((runId: string, actionId: string) => {
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
    ? "New run"
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
            setNewRun(true);
            setSelected(null);
          }}
          disabled={!healthy}
          title={!healthy ? "Reconnect to a healthy daemon" : "New run · ⌘N"}
        >
          <Plus size={15} />
          <span>New run</span>
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
                  New run
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
          className={`sheet-body ${page === "human" && !selected && !newRun ? "human-sheet" : ""}`}
          data-stale={!healthy && !!connection.data}
        >
          {newRun ? (
            <NewRun
              connected={healthy}
              activeRun={!!activeId}
              onClose={() => setNewRun(false)}
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
            setNewRun(true);
          }}
          healthy={healthy}
          reconnect={reconnect}
        />
      )}
      {process && (
        <Dialog
          wide
          title="Process output"
          description="Durable process events from the daemon"
          onClose={() => setProcess(null)}
        >
          <ProcessLogs id={process} />
        </Dialog>
      )}
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
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const actions = runs.flatMap((run) =>
    run.human_actions
      .filter((a) => canResolve(run) && pending(a))
      .map((action) => ({ run, action })),
  );
  const approvals = runs.filter((r) => can(r, "execute")),
    integrable = runs.filter((r) => can(r, "integrate")),
    continuable = runs.filter((r) => can(r, "continue"));
  const sessions: Session[] = active
    ? [
        active.planner,
        ...active.tasks.flatMap((t) =>
          t.attempts.flatMap((a) => [
            a.coder,
            ...(a.reviews.length
              ? a.reviews.map((r) => r.session)
              : [a.reviewer]),
          ]),
        ),
      ].filter((s): s is Session => !!s && !s.outcome)
    : [];
  return (
    <div className="overview-layout">
      <div className="overview-main">
        <Panel
          title="Working now"
          meta={
            active && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => select(active.id)}
              >
                Open run
                <ArrowRight size={13} />
              </Button>
            )
          }
        >
          {active ? (
            <div className="working-now">
              <div className="working-header">
                <div>
                  <div className="run-title-row">
                    <h2>{title(active)}</h2>
                    <Badge status={active.status} />
                  </div>
                  <p className="meta">
                    {basename(active.repository)} · {active.requested_base} @{" "}
                    {short(active.base_commit)}
                  </p>
                </div>
                <div className="working-duration">
                  <span className="meta">Since approval</span>
                  <strong>
                    {active.approved_at
                      ? duration((now - Date.parse(active.approved_at)) / 1000)
                      : "Not approved"}
                  </strong>
                </div>
              </div>
              {sessions.length ? (
                sessions.map((s) => (
                  <SessionRow key={s.id} session={s} onProcess={onProcess} />
                ))
              ) : (
                <p className="quiet-row">
                  No active agent sessions. Inspect the run for its next
                  required action.
                </p>
              )}
              <div className="working-footer">
                <span>
                  {sessions.length} agents running ·{" "}
                  {active.tasks.filter((t) => t.status === "COMPLETED").length}/
                  {active.tasks.length} tasks completed
                </span>
                <span>Updated {clock(active.updated_at)}</span>
              </div>
            </div>
          ) : (
            <Empty
              title={activeId ? "Loading active run…" : "Nothing is running"}
            >
              <span>Start a run or approve one that's ready.</span>
              <Button variant="secondary" disabled={!healthy} onClick={onNew}>
                New run
              </Button>
            </Empty>
          )}
        </Panel>
        {(active ?? runs[0]) && (
          <Panel
            title="Meaningful activity"
            meta={
              <Button
                variant="ghost"
                size="sm"
                onClick={() => select((active ?? runs[0]).id, "execution")}
              >
                Full journal
                <ArrowRight size={13} />
              </Button>
            }
          >
            <Activity
              id={(active ?? runs[0]).id}
              compact
              showErrors={healthy}
            />
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
    { label: "New run", action: newRun, disabled: !healthy },
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
