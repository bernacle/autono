import { checkLabels, verificationCounts } from "../lib/evidence-model";
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { ArrowRight, Expand, Minimize, TerminalSquare } from "lucide-react";
import type { Run } from "../lib/types";
import type { ExecutionModel, History, Segment } from "../lib/execution-model";
import {
  intervalEnd,
  sourceLabel,
  unionDuration,
} from "../lib/execution-model";
import { elapsed, relatedTasks, timeLabel, useNow } from "../lib/execution";
import { orderedTasks } from "../lib/presentation";
import { Badge, Empty, Panel, StatusGlyph } from "./common";
import { Button } from "./ui/button";
import { Segmented } from "./ui/segmented";
import { Journal } from "./journal";
import { TaskInspector } from "./tasks";
import { PhaseSource } from "./phase-source";
import { CheckSummary, SessionRow } from "./evidence";
const labels: Record<Segment["phase"], string> = {
  planning: "Planning",
  awaiting: "Awaiting approval",
  executing: "Executing",
  verified: "Verified",
  integrating: "Integrating",
  waiting: "Dependency wait",
  ready: "Ready",
  running: "Task execution",
  paused: "Human pause",
  blocked: "Blocked",
  coding: "Coding",
  checks: "Checks",
  review: "Independent review",
};
export const phaseLabel = (s: Segment) =>
  `${labels[s.phase]}${s.phase === "checks" && s.outcome !== "recorded" && s.outcome !== "passed" && s.outcome !== "failed" && s.outcome !== "interrupted" ? ` · ${checkLabels[s.outcome].toLowerCase()}` : s.outcome === "failed" ? " · failed" : s.outcome === "interrupted" ? " · stopped" : ""}`;
export function PhaseBar({
  segment: s,
  from,
  to,
  now,
  selected,
  onSelect,
  tabIndex = -1,
}: {
  segment: Segment;
  from: number;
  to: number;
  now: number;
  selected?: boolean;
  onSelect?: (s: Segment) => void;
  tabIndex?: number;
}) {
  const end = intervalEnd(s, now),
    left = Math.max(from, s.start),
    right = Math.min(to, end);
  if (right < from || s.start > to || end < s.start) return null;
  const description = `${s.task ?? "Run"}, ${phaseLabel(s)}, ${s.attempt ? `attempt ${s.attempt}, ` : ""}${timeLabel(s.start)} to ${s.end === null ? (s.live ? "now" : "unknown") : timeLabel(s.end)}, ${s.live ? "open interval; live duration in inspector" : elapsed(end - s.start)}. ${s.waitingOn?.length ? `Waiting on ${s.waitingOn.join(", ")}. ` : ""}${s.sources.map(sourceLabel).join("; ")}`;
  return (
    <button
      type="button"
      data-phase={s.id}
      className={`phase-bar ${s.humanHeld ? "wait-human-held" : ""} phase-${s.phase} ${s.live ? "phase-live" : ""} phase-${s.outcome} ${selected ? "phase-selected" : ""}`}
      style={{
        left: `${(100 * (left - from)) / (to - from)}%`,
        width: `${Math.max(0.3, (100 * (right - left)) / (to - from))}%`,
      }}
      title={description}
      aria-label={description}
      aria-pressed={selected}
      tabIndex={tabIndex}
      onClick={() => onSelect?.(s)}
    >
      <span>
        {phaseLabel(s)}
        {s.attempt ? ` · ${s.attempt}` : ""}
      </span>
    </button>
  );
}
function ticks(from: number, to: number) {
  const span = to - from,
    steps = [
      1000, 2000, 5000, 10000, 30000, 60000, 120000, 240000, 600000, 1800000,
      3600000, 21600000, 86400000,
    ];
  const step = steps.find((v) => span / v <= 8) ?? 86400000;
  const out = [];
  for (let t = Math.ceil(from / step) * step; t < to; t += step) out.push(t);
  return out;
}
export function Timeline({
  run,
  model,
  selected,
  phaseId,
  onTask,
  onPhase,
  onGraph,
  mini = false,
  connected = true,
  onExpanded,
}: {
  run: Run;
  model: ExecutionModel;
  selected: string | null;
  phaseId?: string | null;
  onTask: (id: string) => void;
  onPhase: (s: Segment) => void;
  onGraph: () => void;
  mini?: boolean;
  connected?: boolean;
  onExpanded?: (expanded: boolean) => void;
}) {
  const clock = useNow(),
    [follow, setFollow] = useState(true),
    [frozen, setFrozen] = useState(clock),
    [expanded, setExpanded] = useState(false),
    [range, setRange] = useState<"whole" | "recent">(() =>
      Date.parse(run.updated_at) - Date.parse(run.created_at) > 3600000 &&
      ["RUNNING", "INTEGRATING"].includes(run.status)
        ? "recent"
        : "whole",
    ),
    [groupOverrides, setGroupOverrides] = useState<Map<string, boolean>>(
      new Map(),
    ),
    [focus, setFocus] = useState(
      mini ? (run.tasks[0]?.plan.id ?? "run") : "run",
    );
  const grid = useRef<HTMLDivElement>(null),
    hasLive = model.segments.some((s) => s.live),
    now =
      connected && hasLive
        ? follow
          ? clock
          : frozen
        : (model.lastEvent ?? Date.parse(run.updated_at));
  const end = Math.max(
      Date.parse(run.created_at) + 10000,
      now,
      follow ? (model.lastEvent ?? 0) : 0,
    ),
    from = range === "whole" ? Date.parse(run.created_at) : end - 900000,
    to = end + Math.max(1000, (end - from) * 0.1);
  const related = relatedTasks(run, selected),
    ordered = orderedTasks(run);
  const groups = (run.plan?.milestones ?? []).map((m) => ({
    ...m,
    tasks: ordered.filter((t) => m.task_ids.includes(t.plan.id)),
  }));
  const rest = ordered.filter(
    (t) => !groups.some((m) => m.task_ids.includes(t.plan.id)),
  );
  if (rest.length)
    groups.push({
      id: "other",
      goal: "Other tasks",
      task_ids: rest.map((t) => t.plan.id),
      tasks: rest,
    });
  useLayoutEffect(() => {
    if (expanded)
      grid.current
        ?.closest(".timeline")
        ?.scrollIntoView({ block: "start", behavior: "instant" });
  }, [expanded]);
  const toggleFollow = () => {
    setFrozen(now);
    setFollow((v) => !v);
  };
  const groupCollapsed = (id: string) =>
    groupOverrides.get(id) ?? run.tasks.length > 25;
  const focusRow = (row: HTMLElement, offset: number) => {
    const rows = [
      ...(grid.current?.querySelectorAll<HTMLElement>("[data-timeline-row]") ??
        []),
    ];
    rows[Math.max(0, Math.min(rows.length - 1, rows.indexOf(row) + offset))]
      ?.querySelector<HTMLElement>("[data-row-label]")
      ?.focus();
  };
  return (
    <section
      className={`timeline ${expanded ? "timeline-expanded" : ""} ${mini ? "timeline-mini" : ""}`}
      aria-label={mini ? "Task activity lanes" : "Execution timeline"}
    >
      {!mini && (
        <header className="timeline-toolbar">
          <strong>Execution timeline</strong>
          <Segmented
            label="Time window"
            value={range}
            onChange={setRange}
            options={[
              { value: "whole", label: "Whole run" },
              { value: "recent", label: "Last 15 min" },
            ]}
          />
          <div />
          <Button
            variant="ghost"
            size="sm"
            aria-pressed={follow}
            onClick={toggleFollow}
          >
            {follow ? "● Following now" : "Follow now"}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setExpanded((v) => !v);
              onExpanded?.(!expanded);
            }}
          >
            {expanded ? <Minimize size={13} /> : <Expand size={13} />}{" "}
            {expanded ? "Compact" : "Expand timeline"}
          </Button>
        </header>
      )}
      {selected && !mini && (
        <div className="timeline-selection">
          <span>
            Selected <code>{selected}</code> · upstream{" "}
            {Array.from(related.upstream).join(", ") || "none"} · downstream{" "}
            {Array.from(related.downstream).join(", ") || "none"}
          </span>
          <Button variant="ghost" size="sm" onClick={onGraph}>
            Show in graph <ArrowRight size={12} />
          </Button>
        </div>
      )}
      <div
        ref={grid}
        role="grid"
        aria-label="Recorded execution phases"
        className="timeline-grid"
        onKeyDown={(e) => {
          const target = e.target as HTMLElement,
            row = target.closest<HTMLElement>("[data-timeline-row]");
          if (!row || e.metaKey || e.ctrlKey || e.altKey) return;
          if (["ArrowDown", "ArrowUp"].includes(e.key)) {
            e.preventDefault();
            focusRow(row, e.key === "ArrowDown" ? 1 : -1);
          }
          if (["ArrowLeft", "ArrowRight"].includes(e.key)) {
            e.preventDefault();
            const cells = [
                ...row.querySelectorAll<HTMLElement>(
                  "[data-row-label],[data-phase]",
                ),
              ],
              i = cells.indexOf(target);
            cells[
              Math.max(
                0,
                Math.min(
                  cells.length - 1,
                  i + (e.key === "ArrowRight" ? 1 : -1),
                ),
              )
            ]?.focus();
          }
          if (e.key.toLowerCase() === "f") {
            e.preventDefault();
            toggleFollow();
          }
          if (e.key.toLowerCase() === "g") {
            e.preventDefault();
            onGraph();
          }
          if (e.key.toLowerCase() === "l") {
            const id = target.getAttribute("data-phase"),
              s = model.segments.find((s) => s.id === id);
            if (s) {
              e.preventDefault();
              onPhase(s);
            }
          }
        }}
      >
        <div role="row" className="timeline-axis">
          <span role="columnheader">Task / agent</span>
          <div role="columnheader" className="lane">
            {ticks(from, to).map((t) => (
              <time
                style={{ left: `${(100 * (t - from)) / (to - from)}%` }}
                key={t}
              >
                {timeLabel(t)}
              </time>
            ))}
            {hasLive && (
              <span
                className="now-tag"
                style={{ left: `${(100 * (now - from)) / (to - from)}%` }}
              >
                now
              </span>
            )}
          </div>
        </div>
        {!mini && (
          <div
            role="row"
            data-timeline-row="run"
            className="timeline-row run-band"
          >
            <div role="gridcell">
              <button
                data-row-label
                tabIndex={focus === "run" ? 0 : -1}
                onFocus={() => setFocus("run")}
                onClick={onGraph}
              >
                Run lifecycle
              </button>
            </div>
            <div role="gridcell" className="lane">
              {model.segments
                .filter((s) => !s.task && !s.session)
                .map((s) => (
                  <PhaseBar
                    key={s.id}
                    segment={s}
                    from={from}
                    to={to}
                    now={now}
                    selected={phaseId === s.id}
                    onSelect={onPhase}
                  />
                ))}
            </div>
          </div>
        )}
        {groups.map((group) => (
          <div role="rowgroup" key={group.id}>
            {!mini && (
              <div className="timeline-group">
                <button
                  aria-expanded={!groupCollapsed(group.id)}
                  onClick={() =>
                    setGroupOverrides((previous) =>
                      new Map(previous).set(
                        group.id,
                        !groupCollapsed(group.id),
                      ),
                    )
                  }
                >
                  {groupCollapsed(group.id) ? "▸" : "▾"} {group.goal}
                </button>
                <span>
                  {group.tasks.filter((t) => t.status === "COMPLETED").length}/
                  {group.tasks.length} verified
                </span>
              </div>
            )}
            {groupCollapsed(group.id) && !mini ? (
              <div className="timeline-summary">
                {group.tasks.length} tasks · expand for recorded phases
              </div>
            ) : (
              group.tasks.map((task) => {
                const taskSegments = model.segments.filter(
                    (s) => s.task === task.plan.id,
                  ),
                  track = model.states.filter(
                    (s) => s.task === task.plan.id && s.phase === "running",
                  ),
                  relatedRow =
                    !selected ||
                    task.plan.id === selected ||
                    related.upstream.has(task.plan.id) ||
                    related.downstream.has(task.plan.id),
                  latest = taskSegments.at(-1);
                return (
                  <div
                    role="row"
                    key={task.plan.id}
                    data-timeline-row={task.plan.id}
                    className={`timeline-row ${selected === task.plan.id ? "row-selected" : ""} ${relatedRow ? "" : "row-unrelated"}`}
                  >
                    <div role="gridcell">
                      <button
                        data-row-label
                        tabIndex={focus === task.plan.id ? 0 : -1}
                        onFocus={() => setFocus(task.plan.id)}
                        onClick={() => onTask(task.plan.id)}
                        title={`${task.plan.goal} · ${task.status.replaceAll("_", " ").toLowerCase()}`}
                      >
                        <StatusGlyph status={task.status} />
                        <code>{task.plan.id}</code>
                        <span>{task.plan.goal}</span>
                      </button>
                    </div>
                    <div role="gridcell" className="lane">
                      {hasLive && (
                        <span
                          className="timeline-now"
                          style={{
                            left: `${(100 * (now - from)) / (to - from)}%`,
                          }}
                        />
                      )}
                      {task.plan.dependencies
                        .filter(
                          (dep) =>
                            group.tasks.indexOf(task) > 0 &&
                            group.tasks[group.tasks.indexOf(task) - 1].plan
                              .id === dep,
                        )
                        .flatMap((dep) =>
                          model.markers
                            .filter(
                              (m) =>
                                m.task === dep &&
                                m.kind === "completed" &&
                                m.time >= from &&
                                m.time <= to,
                            )
                            .map((m) => (
                              <span
                                aria-hidden="true"
                                key={m.id}
                                className="dependency-connector"
                                style={{
                                  left: `${(100 * (m.time - from)) / (to - from)}%`,
                                }}
                              />
                            )),
                        )}
                      {track.map((s) => (
                        <span
                          key={s.id}
                          className="running-track"
                          style={{
                            left: `${Math.max(0, (100 * (s.start - from)) / (to - from))}%`,
                            width: `${Math.max(0, (100 * (Math.min(to, intervalEnd(s, now)) - Math.max(from, s.start))) / (to - from))}%`,
                          }}
                        />
                      ))}
                      {taskSegments.map((s) => (
                        <PhaseBar
                          key={s.id}
                          segment={s}
                          from={from}
                          to={to}
                          now={now}
                          selected={phaseId === s.id}
                          onSelect={onPhase}
                        />
                      ))}
                      {model.markers
                        .filter(
                          (m) =>
                            m.task === task.plan.id &&
                            m.time >= from &&
                            m.time <= to,
                        )
                        .map((m) => (
                          <button
                            className={`timeline-marker marker-${m.kind}`}
                            key={m.id}
                            style={{
                              left: `${(100 * (m.time - from)) / (to - from)}%`,
                            }}
                            title={`${m.label} · ${timeLabel(m.time)} · ${sourceLabel(m.source)}`}
                            aria-label={`${m.task}, ${m.label}, ${timeLabel(m.time)}`}
                            tabIndex={-1}
                            onClick={() => onTask(task.plan.id)}
                          >
                            {m.kind === "completed"
                              ? "✓"
                              : m.kind === "retry"
                                ? "↻"
                                : m.kind === "finding"
                                  ? "•"
                                  : m.kind === "failure"
                                    ? "×"
                                    : "◇"}
                          </button>
                        ))}
                      {!taskSegments.length && (
                        <span className="lane-empty">No observed phases</span>
                      )}
                    </div>
                    {mini && (
                      <span className="mini-outcome">
                        {latest?.live
                          ? phaseLabel(latest)
                          : task.status === "COMPLETED"
                            ? "Verified"
                            : task.status === "WAITING"
                              ? `Waits on ${task.plan.dependencies.join(", ") || "readiness"}`
                              : task.status.replaceAll("_", " ").toLowerCase()}
                      </span>
                    )}
                  </div>
                );
              })
            )}
          </div>
        ))}
      </div>
      {!mini && (
        <>
          <div className="timeline-legend">
            {["coding", "checks", "review", "paused", "waiting"].map((p) => (
              <span key={p}>
                <i className={`legend-swatch phase-${p}`} />
                {labels[p as Segment["phase"]]}
              </span>
            ))}
            <span>
              <i className="legend-swatch phase-live" />
              Live only
            </span>
            <span>↻ Retry · ◇ Decision · ✓ Verified</span>
          </div>
          <footer className="timeline-footnote">
            Times from the daemon journal · session admission may precede
            process start · F follow · arrows navigate · Enter inspect · L logs
            · G graph
          </footer>
        </>
      )}
    </section>
  );
}
export function ExecutionView({
  run,
  model,
  history,
  selected,
  phaseId,
  onSelect,
  onPhase,
  onProcess,
  onGraph,
  onHuman,
  connected,
  logDockOpen = false,
}: {
  run: Run;
  model: ExecutionModel | undefined;
  history: History | undefined;
  selected: string | null;
  phaseId: string | null;
  onSelect: (id: string | null) => void;
  onPhase: (id: string | null) => void;
  onProcess: (id: string) => void;
  onGraph: () => void;
  onHuman: (id: string) => void;
  connected: boolean;
  logDockOpen?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const now = useNow(),
    task = run.tasks.find((t) => t.plan.id === selected),
    phase = model?.segments.find((s) => s.id === phaseId),
    close = useCallback(() => onSelect(null), [onSelect]);
  const inspect = (s: Segment) => {
    onSelect(s.task ?? null);
    onPhase(s.id);
    if (s.process) onProcess(s.process);
  };
  if (!model || !history)
    return <Empty title="Replaying recorded execution…" />;
  const agents = model.segments.filter((s) => s.live && s.session),
    checks = [
      ...run.tasks.flatMap((t) => t.attempts.flatMap((a) => a.verification)),
      ...run.integration_verification,
    ],
    reviews = run.tasks.flatMap((t) => t.attempts.flatMap((a) => a.reviews)),
    checkCounts = verificationCounts(checks);
  return (
    <div
      className={`execution-layout ${task ? "has-inspector" : ""} ${expanded ? "execution-expanded" : ""}`}
    >
      <div className="execution-main">
        <div className="execution-stats">
          <div>
            <span>Agent activity</span>
            <strong>
              {agents.length}{" "}
              <small>
                {connected ? "live sessions" : "last observed sessions"}
              </small>
            </strong>
          </div>
          <div>
            <span>Verified work</span>
            <strong>
              {run.tasks.filter((t) => t.status === "COMPLETED").length}
              <small> / {run.tasks.length} tasks</small>
            </strong>
          </div>
          <div>
            <span>Check results</span>
            <strong>
              {checkCounts.passed}
              <small className="check-outcomes">
                {" "}
                passed · {checkCounts.test_failure} test failed
                {checkCounts.timeout > 0 &&
                  ` · ${checkCounts.timeout} timed out`}
                {checkCounts.cancelled > 0 &&
                  ` · ${checkCounts.cancelled} cancelled`}
                {checkCounts.infrastructure_failure > 0 &&
                  ` · ${checkCounts.infrastructure_failure} infrastructure`}
                {checkCounts.unknown > 0 &&
                  ` · ${checkCounts.unknown} unavailable`}
              </small>
            </strong>
          </div>
          <div>
            <span>Independent reviews</span>
            <strong>
              {reviews.length}
              <small> · {model.reviewRejections} rejected</small>
            </strong>
          </div>
          <div>
            <span>Retries / human pause</span>
            <strong>
              {model.retries}
              <small>
                {" "}
                /{" "}
                {elapsed(
                  unionDuration(
                    model.states.filter((s) => s.phase === "paused"),
                    connected ? now : (model.lastEvent ?? now),
                  ),
                )}
              </small>
            </strong>
          </div>
          <div>
            <span>Last recorded progress</span>
            <strong className="stat-time">
              {model.lastProgress
                ? timeLabel(model.lastProgress)
                : "None recorded"}
            </strong>
          </div>
        </div>
        {!model.caughtUp && (
          <div className="notice" role="status">
            Replaying journal from cursor {model.cursor}… open intervals appear
            after catch-up.
          </div>
        )}
        {model.limited && (
          <div className="notice notice-warning">
            Timeline limited to the first 5,000 events. The journal continues
            following the latest 500 records.
          </div>
        )}
        {(model.unparsed > 0 || model.incomplete > 0) && (
          <div className="history-notice">
            {model.unparsed} events not shown on the timeline ·{" "}
            {model.incomplete} incomplete intervals. Inspect original evidence
            in the journal; missing history is not estimated.
          </div>
        )}
        <Timeline
          run={run}
          model={model}
          selected={selected}
          phaseId={phaseId}
          onTask={onSelect}
          onPhase={inspect}
          onGraph={onGraph}
          connected={connected}
          onExpanded={setExpanded}
        />
        {phase && !phase.task && (
          <div className="run-phase-evidence">
            <strong>
              {phaseLabel(phase)} ·{" "}
              {elapsed(
                intervalEnd(phase, connected ? now : (model.lastEvent ?? now)) -
                  phase.start,
              )}
            </strong>
            <p className="meta">{phase.sources.map(sourceLabel).join(" · ")}</p>
            {phase.process && (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => onProcess(phase.process!)}
              >
                Open check output
              </Button>
            )}
          </div>
        )}
        <div className="execution-journal-split">
          <Panel title="Engineering journal">
            <Journal
              run={run}
              events={history.recent}
              cursor={history.cursor}
              onTask={onSelect}
            />
          </Panel>
          <Findings run={run} onSelect={onSelect} onProcess={onProcess} />
        </div>
        <details className="audit session-archive">
          <summary>All agent sessions and process references</summary>
          <SessionRow session={run.planner} onProcess={onProcess} />
          {run.tasks.flatMap((t) =>
            t.attempts.map((a) => (
              <div key={`${t.plan.id}:${a.number}`}>
                <h3>
                  {t.plan.id} · attempt {a.number}
                </h3>
                <SessionRow session={a.coder} onProcess={onProcess} />
                {(a.reviews.length
                  ? a.reviews.map((r) => r.session)
                  : a.reviewer
                    ? [a.reviewer]
                    : []
                ).map((s) => (
                  <SessionRow key={s.id} session={s} onProcess={onProcess} />
                ))}
              </div>
            )),
          )}
        </details>
      </div>
      {task && (
        <TaskInspector
          key={task.plan.id}
          task={task}
          run={run}
          logDockOpen={logDockOpen}
          onClose={close}
          onProcess={onProcess}
          onHuman={onHuman}
          phaseContext={
            <section className="phase-context">
              <h3>Recorded phases</h3>
              <p className="meta">
                {task.attempts.length}/{run.settings.execution.max_attempts}{" "}
                attempts · maximum {run.settings.execution.max_review_cycles}{" "}
                reviews per attempt
              </p>
              <p>
                Unblocks{" "}
                {run.tasks
                  .filter((t) => t.plan.dependencies.includes(task.plan.id))
                  .map((t) => t.plan.id)
                  .join(", ") || "no direct descendants"}{" "}
                when completed.
              </p>
              {model.segments
                .filter((s) => s.task === task.plan.id)
                .map((s) => (
                  <button
                    className={`phase-evidence-row ${s.id === phaseId ? "selected" : ""}`}
                    key={s.id}
                    aria-pressed={s.id === phaseId}
                    onClick={() => inspect(s)}
                  >
                    <time>{timeLabel(s.start)}</time>
                    <span>
                      {phaseLabel(s)}
                      {s.attempt ? ` · attempt ${s.attempt}` : ""}
                    </span>
                    <code>
                      {elapsed(
                        intervalEnd(
                          s,
                          connected ? now : (model.lastEvent ?? now),
                        ) - s.start,
                      )}
                    </code>
                  </button>
                ))}
              {phase?.task === task.plan.id && (
                <div className="phase-sources">
                  <h4>
                    Selected phase · attempt {phase.attempt ?? "not applicable"}
                  </h4>
                  {phase.sources.map((s) => (
                    <PhaseSource
                      key={`${s.kind}:${s.sequence}`}
                      runId={run.id}
                      source={s}
                    />
                  ))}
                  {phase.process && (
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => onProcess(phase.process!)}
                    >
                      <TerminalSquare size={13} />
                      Open associated output
                    </Button>
                  )}
                  <p className="meta">
                    Journal record times. Task-state changes share the preceding
                    transaction; agent admission can precede process start.
                  </p>
                </div>
              )}
              <Button variant="ghost" size="sm" onClick={onGraph}>
                Show in graph <ArrowRight size={12} />
              </Button>
            </section>
          }
        />
      )}
    </div>
  );
}
export function Findings({
  run,
  onSelect,
  onProcess,
}: {
  run: Run;
  onSelect: (id: string) => void;
  onProcess: (id: string) => void;
}) {
  const reviews = run.tasks.flatMap((t) =>
    t.attempts.flatMap((a) => a.reviews.map((r, i) => ({ t, a, r, i }))),
  );
  const findings = reviews.filter(
    (x) =>
      x.r.diagnostic ||
      x.r.review?.findings.length ||
      x.r.review?.blocker ||
      x.r.review?.criteria.some((c) => !c.satisfied),
  );
  return (
    <Panel
      title="Reviewer findings"
      meta={<span className="meta">{reviews.length} reviews recorded</span>}
    >
      <div className="findings-rail">
        {findings.map(({ t, a, r, i }) => (
          <article key={r.session.id}>
            <div className="row-between">
              <button className="task-chip" onClick={() => onSelect(t.plan.id)}>
                {t.plan.id} · attempt {a.number}
              </button>
              <span className="meta">Review {i + 1}</span>
            </div>
            {r.diagnostic && <p className="danger-text">{r.diagnostic}</p>}
            {r.review?.blocker && <p>{r.review.blocker}</p>}
            {r.review?.criteria
              .filter((c) => !c.satisfied)
              .map((c) => (
                <p key={c.criterion_id}>
                  <strong>Unmet: {c.criterion}</strong>
                  <br />
                  {c.evidence}
                </p>
              ))}
            {r.review?.findings.map((f, j) => (
              <div className="review-finding" key={j}>
                <Badge status={f.severity} />
                <code>
                  {f.file}:{f.line}
                </code>
                <p>{f.explanation}</p>
                <details>
                  <summary>
                    Recommended fix · confidence{" "}
                    {Math.round(f.confidence * 100)}%
                  </summary>
                  <p>{f.recommended_fix}</p>
                </details>
              </div>
            ))}
            {r.session.process_run_id && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => onProcess(r.session.process_run_id!)}
              >
                Reviewer output <ArrowRight size={12} />
              </Button>
            )}
          </article>
        ))}
        {!findings.length && (
          <Empty
            title={
              reviews.length
                ? "No findings recorded"
                : "No reviews recorded yet"
            }
          >
            Independent findings appear here when the daemon retains them.
          </Empty>
        )}
      </div>
    </Panel>
  );
}
export function VerificationLedger({
  run,
  onTask,
  onProcess,
}: {
  run: Run;
  onTask: (id: string) => void;
  onProcess: (id: string) => void;
}) {
  return (
    <Panel title="Verification ledger">
      <div className="verification-ledger">
        {run.tasks.flatMap((t) =>
          t.attempts.map((a) => (
            <div key={`${t.plan.id}:${a.number}`}>
              <button className="task-chip" onClick={() => onTask(t.plan.id)}>
                {t.plan.id} · {a.number}
              </button>
              <CheckSummary run={run} task={t} attempt={a} />
              <span>{a.reviews.length} independent reviews</span>
              <span>
                {a.verified_tree && a.verified_tree === a.preserved_tree
                  ? "✓ Trees match"
                  : "No matching trees"}
              </span>
              <Button
                variant="ghost"
                size="sm"
                disabled={!a.verification.at(-1)?.process_run_id}
                onClick={() => onProcess(a.verification.at(-1)!.process_run_id)}
              >
                Output
              </Button>
            </div>
          )),
        )}
        {!run.tasks.some((t) => t.attempts.length) && (
          <Empty title="No verification attempts yet" />
        )}
      </div>
    </Panel>
  );
}
