import { Check, ChevronRight } from "lucide-react";
import type { Run } from "../lib/types";
import { lifecycleIndex } from "../lib/presentation";
import { Badge } from "./common";
export function Lifecycle({ run }: { run: Run }) {
  const active = lifecycleIndex(run);
  const terminal = ["FAILED", "CANCELLED", "INTERRUPTED"].includes(run.status);
  const done = run.tasks.filter((t) => t.status === "COMPLETED").length;
  const labels = [
    "Planned",
    "Approved",
    `Executing ${done}/${run.tasks.length}`,
    "Verified",
    "Integrating",
    "Completed",
  ];
  return (
    <ol className="lifecycle" aria-label="Run lifecycle">
      {labels.map((label, i) => {
        if (terminal && i > active) return null;
        return (
          <li
            key={label}
            aria-current={i === active ? "step" : undefined}
            data-complete={i < active || run.status === "COMPLETED"}
          >
            {i > 0 && <ChevronRight size={12} />}
            <span>
              {(i < active || run.status === "COMPLETED") && (
                <Check size={12} className="success-text" />
              )}
              {label}
            </span>
          </li>
        );
      })}
      {terminal && (
        <li>
          <ChevronRight size={12} />
          <Badge status={run.status} />
        </li>
      )}
    </ol>
  );
}
