import { useRef, useState } from "react";
import type { Run, Task } from "../lib/types";
import { blockedAncestors } from "../lib/utils";
import { useId } from "react";
import { relatedTasks } from "../lib/execution";
import { taskDepths } from "../lib/presentation";
import { Badge } from "./common";
export function DependencyGraph({
  tasks,
  milestones,
  selected,
  onSelect,
}: {
  tasks: Task[];
  milestones?: NonNullable<Run["plan"]>["milestones"];
  selected?: string | null;
  onSelect?: (id: string) => void;
}) {
  const markerId = useId().replaceAll(":", "");
  const related = relatedTasks({ tasks } as Run, selected ?? null);
  const depths = taskDepths(tasks),
    maxDepth = Math.max(0, ...depths.values());
  const [focused, setFocused] = useState(tasks[0]?.plan.id);
  const container = useRef<HTMLDivElement>(null);
  const width = (maxDepth + 1) * 292 + 24;
  const positions = new Map<string, { x: number; y: number }>();
  const assigned = new Set<string>();
  const groups = (milestones ?? []).map((m) => ({
    ...m,
    task_ids: m.task_ids.filter((id) => {
      if (assigned.has(id) || !tasks.some((t) => t.plan.id === id))
        return false;
      assigned.add(id);
      return true;
    }),
  }));
  const remaining = tasks
    .filter((t) => !assigned.has(t.plan.id))
    .map((t) => t.plan.id);
  if (remaining.length)
    groups.push({
      id: "other",
      goal: groups.length ? "Other tasks" : "Tasks",
      task_ids: remaining,
    });
  const { height, lanes } = groups.reduce(
    (layout, group) => {
      const y = layout.height;
      let rows = 0;
      for (let depth = 0; depth <= maxDepth; depth++) {
        const bucket = tasks.filter(
          (t) =>
            group.task_ids.includes(t.plan.id) &&
            depths.get(t.plan.id) === depth,
        );
        rows = Math.max(rows, bucket.length);
        bucket.forEach((task, i) =>
          positions.set(task.plan.id, {
            x: 24 + depth * 292,
            y: y + 42 + i * 88,
          }),
        );
      }
      return {
        height: y + 52 + rows * 88,
        lanes: [...layout.lanes, { ...group, y }],
      };
    },
    { height: 42, lanes: [] as ((typeof groups)[number] & { y: number })[] },
  );
  const focus = (id?: string) => {
    if (!id) return;
    setFocused(id);
    const button = [
      ...(container.current?.querySelectorAll<HTMLButtonElement>(
        "button[data-task]",
      ) ?? []),
    ].find((b) => b.dataset.task === id);
    button?.focus();
    button?.scrollIntoView({ block: "nearest", inline: "nearest" });
  };
  const order = [...tasks].sort(
    (a, b) =>
      (depths.get(a.plan.id) ?? 0) - (depths.get(b.plan.id) ?? 0) ||
      (positions.get(a.plan.id)?.y ?? 0) - (positions.get(b.plan.id)?.y ?? 0),
  );
  return (
    <div
      className="graph-scroll"
      ref={container}
      aria-label="Task dependency graph"
    >
      <div className="dependency-graph" style={{ width, height }}>
        {Array.from({ length: maxDepth + 1 }, (_, i) => (
          <div
            key={i}
            className="graph-column-label"
            style={{ left: 24 + i * 292 }}
          >
            {i === 0
              ? "Can start immediately"
              : `After ${i} ${i === 1 ? "dependency" : "dependencies"}`}
          </div>
        ))}
        {lanes.map((lane) => (
          <div
            className="graph-lane-label"
            key={lane.id}
            style={{ top: lane.y, width }}
          >
            {lane.goal}
          </div>
        ))}
        <svg
          width={width}
          height={height}
          className="graph-edges"
          aria-hidden="true"
        >
          <defs>
            <marker
              id={markerId}
              markerWidth="7"
              markerHeight="7"
              refX="6"
              refY="3.5"
              orient="auto"
            >
              <path d="M0 0 L7 3.5 L0 7" fill="none" stroke="context-stroke" />
            </marker>
          </defs>
          {tasks.flatMap((task) =>
            task.plan.dependencies.map((dep) => {
              const from = positions.get(dep),
                to = positions.get(task.plan.id);
              if (!from || !to) return null;
              const source = tasks.find((t) => t.plan.id === dep);
              const held =
                source?.status === "BLOCKED_HUMAN" ||
                (source &&
                  blockedAncestors(source, tasks).some(
                    (id) =>
                      tasks.find((t) => t.plan.id === id)?.status ===
                      "BLOCKED_HUMAN",
                  ));
              const kind =
                source?.status === "COMPLETED"
                  ? "satisfied"
                  : held
                    ? "held"
                    : "waiting";
              const start = from.x + 240,
                end = to.x,
                mid = (start + end) / 2;
              return (
                <path
                  key={`${dep}-${task.plan.id}`}
                  markerEnd={`url(#${markerId})`}
                  className={`edge-${kind} ${selected && (dep === selected || task.plan.id === selected || related.upstream.has(task.plan.id) || related.downstream.has(dep)) ? "edge-selected" : ""}`}
                  d={`M${start} ${from.y + 32} C${mid} ${from.y + 32},${mid} ${to.y + 32},${end - 5} ${to.y + 32}`}
                />
              );
            }),
          )}
        </svg>
        {order.map((task) => {
          const pos = positions.get(task.plan.id)!;
          const blocked =
            task.status === "WAITING" ? blockedAncestors(task, tasks) : [];
          const humanHeld = blocked.some(
            (id) =>
              tasks.find((t) => t.plan.id === id)?.status === "BLOCKED_HUMAN",
          );
          return (
            <button
              key={task.plan.id}
              data-task={task.plan.id}
              className={`task-node ${selected && selected !== task.plan.id && !related.upstream.has(task.plan.id) && !related.downstream.has(task.plan.id) ? "node-unrelated" : ""}`}
              style={{ left: pos.x, top: pos.y }}
              tabIndex={
                (focused ?? tasks[0]?.plan.id) === task.plan.id ? 0 : -1
              }
              aria-pressed={selected === task.plan.id}
              aria-label={`${task.plan.id}: ${task.plan.goal}; ${task.status.toLowerCase().replaceAll("_", " ")}; ${task.plan.dependencies.length ? `after ${task.plan.dependencies.join(", ")}` : "no dependencies"}`}
              onFocus={() => setFocused(task.plan.id)}
              onClick={() => onSelect?.(task.plan.id)}
              onKeyDown={(e) => {
                const index = order.indexOf(task);
                const next =
                  e.key === "ArrowRight"
                    ? tasks.find((t) =>
                        t.plan.dependencies.includes(task.plan.id),
                      )?.plan.id
                    : e.key === "ArrowLeft"
                      ? task.plan.dependencies[0]
                      : e.key === "ArrowDown" || e.key === "j"
                        ? order[Math.min(order.length - 1, index + 1)]?.plan.id
                        : e.key === "ArrowUp" || e.key === "k"
                          ? order[Math.max(0, index - 1)]?.plan.id
                          : e.key === "Home"
                            ? order[0]?.plan.id
                            : e.key === "End"
                              ? order.at(-1)?.plan.id
                              : undefined;
                if (next) {
                  e.preventDefault();
                  focus(next);
                }
              }}
            >
              <div className="row-between">
                <code>{task.plan.id}</code>
                <Badge status={task.status} />
              </div>
              <strong title={task.plan.goal}>{task.plan.goal}</strong>
              <span className="sr-only">
                {task.plan.dependencies.length
                  ? `← ${task.plan.dependencies.join(" + ")}`
                  : "No dependencies"}
              </span>
              {blocked.length > 0 && (
                <span
                  className={`graph-blocked-label ${humanHeld ? "human-held" : ""}`}
                >
                  Waiting on {blocked.join(", ")}
                </span>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}
