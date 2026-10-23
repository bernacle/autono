import type { RunHandoff } from "../lib/types";
import { Panel } from "./common";
export function PartialHandoff({ report }: { report?: RunHandoff | null }) {
  if (
    !report ||
    ![
      "partially_completed",
      "blocked",
      "failed",
      "interrupted",
      "evidence_incomplete",
    ].includes(report.outcome)
  )
    return null;
  return (
    <Panel
      title={
        report.outcome === "partially_completed"
          ? "Partially completed"
          : "Work remaining"
      }
    >
      <div className="handoff-content">
        <p>{report.summary}</p>
        <ul>
          {report.tasks.map((task) => (
            <li key={task.task_id}>
              {task.task_id}: {task.delivery.replaceAll("_", " ")}
            </li>
          ))}
        </ul>
        {report.needs_you.length > 0 && (
          <>
            <strong>Needs you</strong>
            <ul>
              {report.needs_you.map((need) => (
                <li key={need.action_id}>
                  {need.requested_action}{" "}
                  <span className="muted">
                    Tasks: {need.affected_tasks.join(", ") || "before planning"}
                    {need.waiting_descendants.length > 0 &&
                      `; waiting descendants: ${need.waiting_descendants.join(", ")}`}
                  </span>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>
    </Panel>
  );
}
