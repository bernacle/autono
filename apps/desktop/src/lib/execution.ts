import { useMemo, useSyncExternalStore } from "react";
import type { Run } from "./types";
import { deriveExecution } from "./execution-model";
import { useExecutionHistory } from "./queries";
let time = Date.now();
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | undefined;
function subscribe(listener: () => void) {
  listeners.add(listener);
  if (!timer)
    timer = setInterval(() => {
      time = Date.now();
      for (const f of listeners) f();
    }, 1000);
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}
export const useNow = () =>
  useSyncExternalStore(
    subscribe,
    () => time,
    () => time,
  );
export function useExecution(run: Run | undefined) {
  const query = useExecutionHistory(run);
  const model = useMemo(
    () => (run && query.data ? deriveExecution(run, query.data) : undefined),
    [run, query.data],
  );
  return { query, model };
}
export function elapsed(ms: number) {
  const s = Math.max(0, Math.floor(ms / 1000));
  return s >= 3600
    ? `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`
    : s >= 60
      ? `${Math.floor(s / 60)}m ${s % 60}s`
      : `${s}s`;
}
export function timeLabel(ms: number) {
  return new Date(ms).toLocaleTimeString([], {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
export function relatedTasks(run: Run, task: string | null) {
  const upstream = new Set<string>(),
    downstream = new Set<string>();
  if (!task) return { upstream, downstream };
  const visit = (id: string, up: boolean, seen: Set<string>) => {
    const next = up
      ? (run.tasks.find((t) => t.plan.id === id)?.plan.dependencies ?? [])
      : run.tasks
          .filter((t) => t.plan.dependencies.includes(id))
          .map((t) => t.plan.id);
    for (const n of next)
      if (n !== task && !seen.has(n)) {
        seen.add(n);
        visit(n, up, seen);
      }
  };
  visit(task, true, upstream);
  visit(task, false, downstream);
  return { upstream, downstream };
}
