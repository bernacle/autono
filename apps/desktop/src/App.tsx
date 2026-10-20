import { useEffect, useState } from "react";
import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  Activity as ActivityIcon,
  ArrowRight,
  Box,
  Command,
  Cpu,
  GitBranch,
  LayoutGrid,
  ListTodo,
  Moon,
  Plus,
  RefreshCw,
  Settings,
  ShieldAlert,
  Sun,
  TerminalSquare,
  Wifi,
  WifiOff,
} from "lucide-react";
import { useTransport } from "./lib/transport";
import type { Connection, Process, Run } from "./lib/types";
import { date, pending, short, title } from "./lib/utils";
import { Button } from "./components/ui/button";
import { Dialog } from "./components/ui/dialog";
import { Badge, Empty, ErrorNotice, Panel } from "./components/common";
import { NewRun } from "./components/new-run";
import { RunDetail } from "./components/run-detail";
import { SettingsView } from "./components/settings";
import { HumanActionCard } from "./components/human-action";
import { Activity, ProcessLogs } from "./components/activity";
type Page = "overview" | "runs" | "human" | "settings";
export default function App() {
  const transport = useTransport(),
    client = useQueryClient();
  const [page, setPage] = useState<Page>("overview"),
    [selected, setSelected] = useState<string | null>(null),
    [newRun, setNewRun] = useState(false),
    [process, setProcess] = useState<string | null>(null);
  const [theme, setTheme] = useState(
    () =>
      localStorage.getItem("autono-theme") ??
      (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"),
  );
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("autono-theme", theme);
  }, [theme]);
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
  const active = activeQuery.data ?? loaded.find((r) => r.id === activeId);
  const runs =
    active && !loaded.some((r) => r.id === active.id)
      ? [active, ...loaded]
      : loaded;
  const actions = runs.flatMap((run) =>
    run.human_actions.filter(pending).map((action) => ({ run, action })),
  );
  const connected = connection.isSuccess && !connection.isError;
  const healthy =
    connected &&
    connection.data.host.persistence.healthy &&
    !connection.data.engineering.persistence_error &&
    connection.data.host.database === "available";
  const navigate = (next: Page) => {
    setPage(next);
    setSelected(null);
  };
  const select = (id: string) => {
    setPage("runs");
    setSelected(id);
  };
  const reconnect = () => {
    void client.invalidateQueries();
  };
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-icon">
            <Command size={21} />
          </span>
          <div>
            autono<span>MISSION CONTROL</span>
          </div>
          <span className="version">v0</span>
        </div>
        <div className="sidebar-section">WORKSPACE</div>
        <nav aria-label="Main navigation">
          {(
            [
              { id: "overview", label: "Overview", icon: LayoutGrid },
              { id: "runs", label: "Runs", icon: GitBranch },
              { id: "human", label: "Human Actions", icon: ShieldAlert },
              { id: "settings", label: "Settings", icon: Settings },
            ] as const
          ).map((item) => (
            <button
              key={item.id}
              className={page === item.id ? "selected" : ""}
              onClick={() => navigate(item.id)}
            >
              <item.icon size={17} />
              {item.label}
              {item.id === "human" && actions.length > 0 && (
                <span className="nav-count">{actions.length}</span>
              )}
            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <div className="local-note">
            <Box size={16} />
            <span>
              Local first
              <br />
              <small>Daemon owns execution</small>
            </span>
          </div>
          <div className="sidebar-connection">
            <span className={`status-dot ${healthy ? "online" : ""}`} />
            <span>
              {healthy
                ? "Daemon connected"
                : connected
                  ? "Daemon degraded"
                  : "Daemon unavailable"}
            </span>
          </div>
          <button
            className="theme-toggle"
            onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
          >
            {theme === "dark" ? <Sun size={15} /> : <Moon size={15} />}{" "}
            {theme === "dark" ? "Light" : "Dark"} appearance
          </button>
        </div>
      </aside>
      <div className="main-shell">
        <header className="topbar">
          <span>
            <span className="status-dot online" />
            LOCAL ENVIRONMENT
          </span>
          <div>
            <span className="muted">
              {connected ? connection.data.address : "Connection required"}
            </span>
            <Button
              size="icon"
              variant="ghost"
              aria-label="Reconnect daemon"
              onClick={reconnect}
            >
              <RefreshCw size={15} />
            </Button>
            <Button
              size="sm"
              onClick={() => setNewRun(true)}
              disabled={!healthy}
            >
              <Plus size={15} />
              New run
            </Button>
          </div>
        </header>
        <main>
          {!connected && (
            <div className="disconnection" role="alert">
              <WifiOff size={20} />
              <div>
                <strong>Daemon connection unavailable</strong>
                <p>
                  {connection.isPending
                    ? "Checking local connection metadata…"
                    : "Start the existing Autono daemon separately. This window reconnects automatically; running agents stay with the daemon."}
                </p>
                {connection.isError && (
                  <p>
                    {typeof connection.error === "object" &&
                    connection.error &&
                    "message" in connection.error
                      ? String(connection.error.message)
                      : "Check local connection metadata and owner-only permissions."}
                  </p>
                )}
              </div>
              <Button variant="secondary" size="sm" onClick={reconnect}>
                Reconnect
              </Button>
            </div>
          )}
          {connected && !healthy && (
            <div className="notice notice-error">
              Daemon persistence or engineering health is degraded. Inspect
              settings and daemon logs before taking action.{" "}
              {connection.data.engineering.persistence_error}
            </div>
          )}
          {selected ? (
            <RunDetail
              key={selected}
              id={selected}
              connected={healthy}
              onBack={() => setSelected(null)}
            />
          ) : page === "settings" ? (
            <SettingsView
              connection={connected ? connection.data : undefined}
            />
          ) : (
            <>
              <div className="page-heading">
                <div>
                  <div className="eyebrow">
                    {page === "overview"
                      ? "Engineering workspace"
                      : page === "human"
                        ? "Developer attention"
                        : "Durable engineering"}
                  </div>
                  <h1>
                    {page === "overview"
                      ? "Overview"
                      : page === "human"
                        ? "Human Actions"
                        : "Runs"}
                  </h1>
                  <p>
                    {page === "overview"
                      ? "Supervise the work. Keep the decisions."
                      : page === "human"
                        ? "Inspect evidence, permission scope and effects before resolving."
                        : "Plans, execution and evidence owned by your local daemon."}
                  </p>
                </div>
                {page === "overview" && (
                  <span className="environment-pill">
                    <Wifi size={13} />
                    {healthy ? "Connected" : connected ? "Degraded" : "Offline"}
                  </span>
                )}
              </div>
              {connected && runsQuery.isError && (
                <ErrorNotice
                  error={runsQuery.error}
                  retry={() => void runsQuery.refetch()}
                />
              )}
              {page === "overview" && (
                <>
                  <div className="overview-metrics">
                    <Metric
                      icon={Cpu}
                      label="Executing processes"
                      value={
                        connected
                          ? connection.data.host.admission.executing_runs
                          : "—"
                      }
                      detail={
                        connected
                          ? `${connection.data.host.admission.max_active_runs} process capacity`
                          : "Daemon unavailable"
                      }
                    />
                    <Metric
                      icon={ListTodo}
                      label="Active run tasks"
                      value={
                        active
                          ? `${active.tasks.filter((t) => t.status === "COMPLETED").length}/${active.tasks.length}`
                          : "—"
                      }
                      detail={
                        active
                          ? `${active.tasks.filter((t) => t.status === "RUNNING").length} running · ${active.tasks.filter((t) => ["BLOCKED", "BLOCKED_HUMAN"].includes(t.status)).length} blocked`
                          : "No active engineering run"
                      }
                    />
                    <Metric
                      icon={ShieldAlert}
                      label="Human actions"
                      value={connected ? actions.length : "—"}
                      detail={`Across ${runs.length} loaded runs`}
                    />
                  </div>
                  <div className="two-columns overview-columns">
                    <Panel
                      title="Active engineering run"
                      meta={<ActivityIcon size={15} />}
                    >
                      {active ? (
                        <div className="active-run">
                          <div className="row-between">
                            <code>{short(active.id)}</code>
                            <Badge status={active.status} />
                          </div>
                          <h2>{title(active)}</h2>
                          <p>{active.summary || active.repository}</p>
                          <div className="mini-task-list">
                            {active.tasks.slice(0, 6).map((task) => (
                              <div key={task.plan.id}>
                                <code>{task.plan.id}</code>
                                <span>{task.plan.goal}</span>
                                <Badge status={task.status} />
                              </div>
                            ))}
                          </div>
                          <Button
                            variant="secondary"
                            onClick={() => select(active.id)}
                          >
                            Inspect run
                            <ArrowRight size={15} />
                          </Button>
                        </div>
                      ) : (
                        <Empty
                          title={
                            activeId
                              ? "Loading active run…"
                              : "No active engineering run"
                          }
                        >
                          Create a plan to begin. Opening Autono never starts
                          agents.
                        </Empty>
                      )}
                      {activeQuery.isError && (
                        <ErrorNotice error={activeQuery.error} />
                      )}
                    </Panel>
                    <Panel
                      title="Needs your attention"
                      meta={<span className="count">{actions.length}</span>}
                    >
                      {actions.length ? (
                        <div className="attention-list">
                          {actions.slice(0, 4).map(({ run, action }) => (
                            <button
                              key={`${run.id}-${action.id}`}
                              onClick={() => {
                                setPage("human");
                                setSelected(null);
                              }}
                            >
                              <span className="attention-dot" />
                              <div>
                                <strong>
                                  {action.requirement.requested_action}
                                </strong>
                                <small>
                                  {short(run.id)} · {action.requirement.kind} ·{" "}
                                  {action.requirement.affected_tasks.join(
                                    ", ",
                                  ) || "whole run"}
                                </small>
                              </div>
                              <ArrowRight size={15} />
                            </button>
                          ))}
                        </div>
                      ) : (
                        <Empty title="No pending actions in loaded runs">
                          New blockers appear here as the daemon records them.
                        </Empty>
                      )}
                    </Panel>
                  </div>
                </>
              )}
              {page === "human" ? (
                <>
                  <p className="coverage-note">
                    Showing unresolved actions across {runs.length} loaded runs.
                    Load older runs below to inspect their actions.
                  </p>
                  <div className="human-actions">
                    {actions.map(({ run, action }) => (
                      <div key={`${run.id}-${action.id}`}>
                        <button
                          className="run-link"
                          onClick={() => select(run.id)}
                        >
                          {title(run)} <ArrowRight size={13} />
                        </button>
                        <HumanActionCard
                          run={run}
                          action={action}
                          connected={healthy && !runsQuery.isError}
                        />
                      </div>
                    ))}
                    {!actions.length && (
                      <Empty title="No unresolved human actions">
                        Resolved decisions and audit history remain on each run.
                      </Empty>
                    )}
                  </div>
                </>
              ) : (
                <Panel
                  title={
                    page === "overview" ? "Recent runs" : "Engineering runs"
                  }
                  meta={<span className="muted">{runs.length} loaded</span>}
                >
                  <RunList
                    runs={page === "overview" ? runs.slice(0, 6) : runs}
                    select={select}
                  />
                </Panel>
              )}
              {runsQuery.hasNextPage && (
                <Button
                  variant="secondary"
                  disabled={runsQuery.isFetchingNextPage}
                  onClick={() => void runsQuery.fetchNextPage()}
                >
                  Load older runs
                </Button>
              )}
              {page === "overview" && (
                <>
                  <Panel
                    title="Recent processes"
                    meta={
                      <span className="muted">
                        Latest 50 · daemon reports total executing above
                      </span>
                    }
                  >
                    {connected && processes.isError && (
                      <ErrorNotice error={processes.error} />
                    )}
                    <div className="process-list">
                      {processes.data?.slice(0, 8).map((p) => (
                        <button key={p.id} onClick={() => setProcess(p.id)}>
                          <TerminalSquare size={16} />
                          <div>
                            <strong>
                              {p.program ?? "Process"}{" "}
                              <code>{short(p.id)}</code>
                            </strong>
                            <small>{p.cwd}</small>
                          </div>
                          <span className="muted">
                            {p.pid ? `PID ${p.pid}` : "No PID"}
                          </span>
                          <Badge status={p.runtime?.phase ?? p.status} />
                          <ArrowRight size={14} />
                        </button>
                      ))}
                      {!processes.data?.length && (
                        <Empty title="No recorded processes" />
                      )}
                    </div>
                  </Panel>
                  {(active ?? runs[0]) && (
                    <Panel
                      title="Recent meaningful events"
                      meta={
                        <span className="muted">
                          {short((active ?? runs[0]).id)}
                        </span>
                      }
                    >
                      <Activity id={(active ?? runs[0]).id} compact />
                    </Panel>
                  )}
                </>
              )}
            </>
          )}
        </main>
        <footer>
          <span>
            <span className={`status-dot ${healthy ? "online" : ""}`} />
            {connected
              ? "Authenticated local daemon"
              : "Reconnecting to local daemon"}
          </span>
          <span>Closing this window leaves runs with the daemon.</span>
        </footer>
      </div>
      {newRun && (
        <NewRun
          connected={healthy}
          onClose={() => setNewRun(false)}
          onCreated={select}
        />
      )}{" "}
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
function Metric({
  icon: Icon,
  label,
  value,
  detail,
}: {
  icon: typeof Cpu;
  label: string;
  value: string | number;
  detail: string;
}) {
  return (
    <div className="metric">
      <div>
        <span>{label}</span>
        <Icon size={16} />
      </div>
      <strong>{value}</strong>
      <small>{detail}</small>
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
    <div className="run-list">
      {runs.map((run) => (
        <button key={run.id} onClick={() => select(run.id)}>
          <GitBranch size={17} />
          <div>
            <strong>{title(run)}</strong>
            <small>
              {run.repository.split("/").at(-1)} <span>·</span> {short(run.id)}{" "}
              <span>·</span>{" "}
              {run.tasks.filter((t) => t.status === "COMPLETED").length}/
              {run.tasks.length} tasks
            </small>
          </div>
          <Badge status={run.status} />
          <time>{date(run.updated_at)}</time>
          <ArrowRight size={14} />
        </button>
      ))}
      {!runs.length && (
        <Empty title="No engineering runs yet">
          Select an existing repository and submit a feature specification to
          create a plan.
        </Empty>
      )}
    </div>
  );
}
