import { useState } from "react";
import type { EngineeringEvent, Run } from "../lib/types";
import { describeEvent, type JournalFilter } from "../lib/journal";
import { timeLabel } from "../lib/execution";
import { Button } from "./ui/button";
import { Empty } from "./common";
export function Journal({
  run,
  events,
  cursor,
  compact = false,
  onTask,
}: {
  run: Run;
  events: EngineeringEvent[];
  cursor: number;
  compact?: boolean;
  onTask?: (id: string) => void;
}) {
  const [filter, setFilter] = useState<JournalFilter>("All"),
    [limit, setLimit] = useState(50);
  const matching = events
    .map((e) => ({ e, ...describeEvent(e, run) }))
    .filter((r) =>
      compact
        ? !r.quiet
        : filter === "All" ||
          r.group === filter ||
          (filter === "Human" && r.human) ||
          (filter === "Agents" && r.e.kind.startsWith("agent_")) ||
          (filter === "Reviews" && r.e.kind.startsWith("review_")) ||
          (filter === "Checks" && r.e.kind.startsWith("verification_")),
    )
    .reverse();
  const rows = matching.slice(0, compact ? 5 : limit);
  return (
    <>
      {!compact && (
        <div className="journal-toolbar">
          <label>
            Show{" "}
            <select
              aria-label="Journal filter"
              value={filter}
              onChange={(e) => setFilter(e.target.value as JournalFilter)}
            >
              {["All", "Agents", "Checks", "Reviews", "Human", "Problems"].map(
                (k) => (
                  <option key={k}>{k}</option>
                ),
              )}
            </select>
          </label>
          <span className="meta">Cursor {cursor} · latest 500 records</span>
        </div>
      )}
      <div
        className={`translated-journal ${compact ? "compact" : ""}`}
        aria-label="Engineering journal"
      >
        {rows.map((r) => (
          <article
            key={r.e.sequence}
            className={r.problem ? "journal-problem" : ""}
          >
            <span className="event-dot" />
            <div>
              {r.task && (
                <button className="task-chip" onClick={() => onTask?.(r.task!)}>
                  {r.task}
                </button>
              )}
              <strong>{r.label}</strong>
              {r.attempt && (
                <span className="meta"> · attempt {r.attempt}</span>
              )}
              {!compact && (
                <details>
                  <summary>
                    <code>
                      {r.e.kind} #{r.e.sequence}
                    </code>{" "}
                    · original evidence
                  </summary>
                  <pre>{r.e.detail}</pre>
                </details>
              )}
            </div>
            <time title={r.e.timestamp}>
              {timeLabel(Date.parse(r.e.timestamp))}
            </time>
          </article>
        ))}
        {!rows.length && <Empty title="No matching activity recorded" />}
        {!compact && matching.length > limit && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setLimit((n) => Math.min(500, n + 50))}
          >
            Show older records · {matching.length - limit} retained
          </Button>
        )}
      </div>
    </>
  );
}
