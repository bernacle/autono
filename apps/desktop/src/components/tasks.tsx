import type { ReactNode } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowRight, Check, X } from "lucide-react";
import type { Run, Task } from "../lib/types";
import { blockedAncestors, pending } from "../lib/utils";
import { latestReview, orderedTasks } from "../lib/presentation";
import { Badge, Empty, StatusGlyph } from "./common";
import { DependencyGraph } from "./dependency-graph";
import { TaskEvidence, SessionRow, WorkspaceRow } from "./evidence";
import { Segmented } from "./ui/segmented";
import { Button } from "./ui/button";
export function TasksView({
  run,
  onProcess,
  onHuman,
  onExecution,
  selected: controlled,
  onSelect,
  initialView,
  logDockOpen = false,
}: {
  run: Run;
  onProcess: (id: string) => void;
  onHuman: (id: string) => void;
  onExecution: () => void;
  selected?: string | null;
  onSelect?: (id: string | null) => void;
  initialView?: "list" | "graph";
  logDockOpen?: boolean;
}) {
  const [view, setView] = useState<"list" | "graph">(initialView ?? "list"),
    [local, setLocal] = useState<string | null>(null);
  const selected = controlled === undefined ? local : controlled;
  const setSelected = useCallback(
    (id: string | null) => {
      setLocal(id);
      onSelect?.(id);
    },
    [onSelect],
  );
  const list = useRef<HTMLDivElement>(null),
    task = run.tasks.find((t) => t.plan.id === selected);
  const ordered = orderedTasks(run);
  const closeInspector = useCallback(() => setSelected(null), [setSelected]);
  useEffect(() => {
    let prefix = false;
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (el.closest("input,textarea,select,[contenteditable=true]")) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "Escape") {
        setSelected(null);
        return;
      }
      if (prefix && ["l", "g"].includes(e.key)) {
        e.preventDefault();
        setView(e.key === "l" ? "list" : "graph");
        prefix = false;
        return;
      }
      prefix = e.key === "g";
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [setSelected]);
  const groups = (run.plan?.milestones ?? []).map((m) => ({
    ...m,
    tasks: ordered.filter((t) => m.task_ids.includes(t.plan.id)),
  }));
  const other = ordered.filter(
    (t) => !groups.some((g) => g.task_ids.includes(t.plan.id)),
  );
  if (other.length)
    groups.push({
      id: "other",
      goal: groups.length ? "Other tasks" : "Tasks",
      task_ids: other.map((t) => t.plan.id),
      tasks: other,
    });
  return (
    <div className={`tasks-layout ${task ? "has-inspector" : ""}`}>
      <div className="tasks-main">
        <div className="tasks-view-bar">
          <Segmented
            label="Task view"
            value={view}
            onChange={setView}
            options={[
              { value: "list", label: "List" },
              { value: "graph", label: "Graph" },
            ]}
          />
          <span className="meta">
            {run.tasks.filter((t) => t.status === "COMPLETED").length}/
            {run.tasks.length} completed
          </span>
        </div>
        {!run.tasks.length && (
          <Empty title="Waiting for the planner">
            The daemon will publish a validated plan here.
          </Empty>
        )}
        {view === "graph" ? (
          <>
            <DependencyGraph
              tasks={run.tasks}
              milestones={run.plan?.milestones}
              selected={selected}
              onSelect={setSelected}
            />
            <div className="graph-legend">
              <span>— Satisfied</span>
              <span>┄ Waiting</span>
              <span className="warning-text">┄ Held by a human action</span>
            </div>
            {task && (
              <div className="graph-impact">
                <span>
                  {
                    run.tasks.filter((t) =>
                      blockedAncestors(t, run.tasks).includes(task.plan.id),
                    ).length
                  }{" "}
                  tasks currently held by {task.plan.id}
                </span>
                <Button variant="ghost" size="sm" onClick={onExecution}>
                  Show in timeline
                  <ArrowRight size={13} />
                </Button>
              </div>
            )}
          </>
        ) : (
          <div
            ref={list}
            className="task-list"
            aria-label="Tasks"
            onKeyDown={(e) => {
              if (!["j", "k", "ArrowDown", "ArrowUp"].includes(e.key)) return;
              const rows = [
                ...(list.current?.querySelectorAll<HTMLButtonElement>(
                  "button[data-task]",
                ) ?? []),
              ];
              const index = rows.indexOf(
                document.activeElement as HTMLButtonElement,
              );
              if (index < 0) return;
              e.preventDefault();
              rows[
                Math.max(
                  0,
                  Math.min(
                    rows.length - 1,
                    index + (["j", "ArrowDown"].includes(e.key) ? 1 : -1),
                  ),
                )
              ]?.focus();
            }}
          >
            {groups.map((group) => (
              <section className="task-group" key={group.id}>
                <div className="task-group-heading">
                  <strong>{group.goal}</strong>
                  <span>
                    {group.tasks.filter((t) => t.status === "COMPLETED").length}
                    /{group.tasks.length}
                  </span>
                </div>
                {group.tasks.map((t) => {
                  const blocked =
                      t.status === "WAITING"
                        ? blockedAncestors(t, run.tasks)
                        : [],
                    latest = t.attempts.at(-1),
                    review = latestReview(t);
                  const humanHeld = blocked.some(
                    (id) =>
                      run.tasks.find((t) => t.plan.id === id)?.status ===
                      "BLOCKED_HUMAN",
                  );
                  return (
                    <button
                      data-task={t.plan.id}
                      key={t.plan.id}
                      className="task-row"
                      aria-pressed={selected === t.plan.id}
                      onClick={() => setSelected(t.plan.id)}
                    >
                      <StatusGlyph status={t.status} />
                      <code>{t.plan.id}</code>
                      <span className="task-goal">
                        <strong>{t.plan.goal}</strong>
                        {t.status === "BLOCKED_HUMAN" && (
                          <small className="warning-text">Needs you</small>
                        )}
                      </span>
                      <span
                        className={
                          humanHeld
                            ? "warning-text dependency-hint"
                            : "muted dependency-hint"
                        }
                      >
                        {blocked.length
                          ? `Waits on ${blocked.join(", ")}`
                          : t.plan.dependencies.length
                            ? `After ${t.plan.dependencies.join(", ")}`
                            : "Independent"}
                      </span>
                      <span
                        className={`task-attempts ${t.attempts.length > 1 ? "danger-text" : "muted"}`}
                      >
                        {t.attempts.length > 1 && "↻ "}
                        {t.attempts.length}/
                        {run.settings.execution.max_attempts}
                      </span>
                      <span className="task-check" title="Checks">
                        {latest?.verification.length ? (
                          latest.verification.every((v) => v.passed) ? (
                            <Check className="success-text" size={13} />
                          ) : (
                            <X className="danger-text" size={13} />
                          )
                        ) : (
                          "—"
                        )}
                      </span>
                      <span className="task-check" title="Review">
                        {review ? (
                          review.accepted ? (
                            <Check className="success-text" size={13} />
                          ) : (
                            <X className="danger-text" size={13} />
                          )
                        ) : (
                          "—"
                        )}
                      </span>
                      <span className="sr-only">
                        {t.status.replaceAll("_", " ").toLowerCase()}
                      </span>
                    </button>
                  );
                })}
              </section>
            ))}
          </div>
        )}
      </div>
      {task && (
        <TaskInspector
          key={task.plan.id}
          task={task}
          run={run}
          logDockOpen={logDockOpen}
          onClose={closeInspector}
          onProcess={onProcess}
          onHuman={onHuman}
        />
      )}
    </div>
  );
}
export function TaskInspector({
  task,
  run,
  onClose,
  onProcess,
  onHuman,
  phaseContext,
  logDockOpen = false,
}: {
  task: Task;
  run: Run;
  phaseContext?: ReactNode;
  logDockOpen?: boolean;
  onClose: () => void;
  onProcess: (id: string) => void;
  onHuman: (id: string) => void;
}) {
  const heading = useRef<HTMLHeadingElement>(null),
    pane = useRef<HTMLElement>(null);
  const [overlay, setOverlay] = useState(
    () => matchMedia("(max-width: 1099px)").matches,
  );
  useEffect(() => {
    const media = matchMedia("(max-width: 1099px)");
    const update = () => setOverlay(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    heading.current?.focus({ preventScroll: true });
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
      if (e.key === "Tab" && matchMedia("(max-width: 1099px)").matches) {
        const selector =
          'button:not(:disabled),a[href],input,summary,[tabindex="0"]';
        const nodes = [
          ...(pane.current?.querySelectorAll<HTMLElement>(selector) ?? []),
          ...(document
            .getElementById("process-log-dock")
            ?.querySelectorAll<HTMLElement>(selector) ?? []),
        ].filter((node) => {
          for (
            let parent = node.parentElement;
            parent;
            parent = parent.parentElement
          )
            if (
              parent.tagName === "DETAILS" &&
              !parent.hasAttribute("open") &&
              node !== parent.querySelector(":scope > summary")
            )
              return false;
          return true;
        });
        nodes.sort((a, b) =>
          a === b
            ? 0
            : a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING
              ? -1
              : 1,
        );
        const first = nodes[0],
          last = nodes.at(-1);
        if (
          e.shiftKey &&
          (document.activeElement === first ||
            document.activeElement === heading.current)
        ) {
          e.preventDefault();
          last?.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first?.focus();
        }
      }
    };
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("keydown", key);
      previous?.focus();
    };
  }, [onClose]);
  const actions = run.human_actions.filter(
    (a) => pending(a) && a.requirement.affected_tasks.includes(task.plan.id),
  );
  const latest = task.attempts.at(-1);
  const review = latestReview(task);
  return (
    <>
      <button
        className="inspector-scrim"
        aria-label="Close task inspector"
        onClick={onClose}
        tabIndex={-1}
      />
      <aside
        className="task-inspector"
        ref={pane}
        role={overlay ? "dialog" : undefined}
        aria-modal={overlay ? true : undefined}
        aria-owns={overlay && logDockOpen ? "process-log-dock" : undefined}
        aria-labelledby="task-inspector-title"
      >
        <header>
          <Badge status={task.status} />
          <Button
            variant="ghost"
            size="icon"
            aria-label="Close task inspector"
            onClick={onClose}
          >
            <X size={16} />
          </Button>
        </header>
        <div className="inspector-body">
          <h2 id="task-inspector-title" tabIndex={-1} ref={heading}>
            {task.plan.id} · {task.plan.goal}
          </h2>
          {phaseContext}
          {actions.map((a) => (
            <div className="human-card" key={a.id}>
              <h3>Human inspection needed</h3>
              <p>{a.requirement.requested_action}</p>
              <details>
                <summary>Daemon reason</summary>
                <p>{a.requirement.reason}</p>
              </details>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => onHuman(a.id)}
              >
                Review human action
                <ArrowRight size={12} />
              </Button>
            </div>
          ))}
          <h3>Goal</h3>
          <p>{task.plan.description}</p>
          <h3>Acceptance criteria</h3>
          <ul className="criteria-list">
            {task.plan.acceptance_criteria.map((c, i) => {
              const result = review?.criteria.find((r) => r.criterion === c);
              return (
                <li key={i}>
                  {result?.satisfied && (
                    <Check className="success-text" size={13} />
                  )}
                  <span>
                    {c}
                    {result && <small>{result.evidence}</small>}
                  </span>
                </li>
              );
            })}
          </ul>
          <h3>Verification expectations</h3>
          <ul>
            {task.plan.verification_expectations.map((c, i) => (
              <li key={i}>{c}</li>
            ))}
          </ul>
          <h3>Dependencies</h3>
          <p>After {task.plan.dependencies.join(", ") || "no other tasks"}</p>
          <p>
            Downstream:{" "}
            {run.tasks
              .filter((t) => t.plan.dependencies.includes(task.plan.id))
              .map((t) => t.plan.id)
              .join(", ") || "none"}
          </p>
          <p className="meta">
            Requirements: {task.plan.requirement_ids.join(", ")}
          </p>
          <h3>Sessions and attempts</h3>
          {task.attempts.map((a) => (
            <div key={a.number}>
              <p className="meta">Attempt {a.number}</p>
              <SessionRow session={a.coder} onProcess={onProcess} />
              {a.reviews.length ? (
                a.reviews.map((r) => (
                  <SessionRow
                    key={r.session.id}
                    session={r.session}
                    onProcess={onProcess}
                  />
                ))
              ) : (
                <SessionRow session={a.reviewer} onProcess={onProcess} />
              )}
            </div>
          ))}
          {latest?.verified_tree &&
            latest.verified_tree === latest.preserved_tree && (
              <p>
                <Check className="success-text" size={13} />
                Verified and preserved trees match
              </p>
            )}
          <TaskEvidence
            compact
            run={{ ...run, tasks: [task] }}
            onProcess={onProcess}
          />
          {task.workspace && (
            <>
              <h3>Workspace</h3>
              <WorkspaceRow runId={run.id} workspace={task.workspace} />
            </>
          )}
        </div>
      </aside>
    </>
  );
}
