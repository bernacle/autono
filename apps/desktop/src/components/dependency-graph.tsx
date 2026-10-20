import type { Task } from "../lib/types";
import { blockedAncestors } from "../lib/utils";
import { Badge } from "./common";
// Layered, keyboard-accessible graph. Every edge is labelled; status remains daemon-owned.
export function DependencyGraph({ tasks }: { tasks: Task[] }) {
  const depths = new Map<string, number>();
  function depth(id: string, path = new Set<string>()): number {
    if (depths.has(id)) return depths.get(id)!;
    if (path.has(id)) return 0;
    path.add(id);
    const deps = tasks.find((t) => t.plan.id === id)?.plan.dependencies ?? [];
    const value = deps.length
      ? 1 + Math.max(...deps.map((d) => depth(d, new Set(path))))
      : 0;
    depths.set(id, value);
    return value;
  }
  tasks.forEach((t) => depth(t.plan.id));
  const levels = [...new Set(depths.values())].sort((a, b) => a - b);
  return (
    <div className="dependency-graph" aria-label="Task dependency graph">
      {levels.map((level) => (
        <div className="graph-level" key={level}>
          <div className="eyebrow">
            {level === 0 ? "Independent tasks" : `Dependency level ${level}`}
          </div>
          {tasks
            .filter((t) => depths.get(t.plan.id) === level)
            .map((task) => {
              const blocked = blockedAncestors(task, tasks);
              return (
                <a
                  className={`task-node ${blocked.length ? "task-node-waiting" : ""}`}
                  key={task.plan.id}
                  href={`#task-${task.plan.id}`}
                >
                  <div className="row-between">
                    <code>{task.plan.id}</code>
                    <Badge status={task.status} />
                  </div>
                  <strong>{task.plan.goal}</strong>
                  <small>
                    {task.plan.dependencies.length
                      ? `← ${task.plan.dependencies.join(" + ")}`
                      : "No dependencies"}
                  </small>
                  {blocked.length > 0 && (
                    <span className="warning-text">
                      Waiting on {blocked.join(", ")}
                    </span>
                  )}
                </a>
              );
            })}
        </div>
      ))}
    </div>
  );
}
