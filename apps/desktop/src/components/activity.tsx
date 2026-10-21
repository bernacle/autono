import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useEvents, useLogs } from "../lib/queries";
import { date } from "../lib/utils";
import { Empty, ErrorNotice } from "./common";
import { Button } from "./ui/button";
export function Activity({
  id,
  compact = false,
  showErrors = true,
}: {
  id: string;
  compact?: boolean;
  showErrors?: boolean;
}) {
  const events = useEvents(id),
    [filter, setFilter] = useState("all");
  const items = events.data?.items ?? [],
    kinds = [...new Set(items.map((e) => e.kind))].sort();
  const visible = items
    .filter(
      (e) =>
        !compact ||
        ![
          "scheduler",
          "workspace_reserved",
          "workspace_ready",
          "task_state",
        ].includes(e.kind),
    )
    .filter((e) => filter === "all" || e.kind === filter)
    .slice(compact ? -4 : -500);
  return (
    <>
      {!compact && (
        <div className="activity-toolbar">
          <label>
            Event filter{" "}
            <select value={filter} onChange={(e) => setFilter(e.target.value)}>
              <option value="all">All engineering events</option>
              {kinds.map((k) => (
                <option key={k}>{k}</option>
              ))}
            </select>
          </label>
          <span className="muted">
            Cursor {events.data?.cursor ?? 0} · latest 500 retained
          </span>
        </div>
      )}
      {showErrors && events.isError && (
        <ErrorNotice error={events.error} retry={() => void events.refetch()} />
      )}
      <div className={compact ? "event-list event-list-compact" : "event-list"}>
        {visible.map((event) => (
          <div className="event" key={event.sequence}>
            <time>{date(event.timestamp)}</time>
            <span className="event-dot" />
            <div>
              <div className="row">
                <strong>{event.kind.replaceAll("_", " ")}</strong>
                <code>#{event.sequence}</code>
              </div>
              <p>{event.detail}</p>
            </div>
          </div>
        ))}
      </div>
      {!visible.length && !events.isError && (
        <Empty
          title={events.isPending ? "Loading events…" : "No matching events"}
        >
          Events are replayed from the daemon’s durable cursor.
        </Empty>
      )}
    </>
  );
}
export function ProcessLogs({ id }: { id: string }) {
  const logs = useLogs(id),
    client = useQueryClient();
  const [following, setFollowing] = useState(true),
    output = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (following && output.current)
      output.current.scrollTop = output.current.scrollHeight;
  }, [logs.data, following]);
  return (
    <>
      <div className="row-between">
        <code>{id}</code>
        <label className="checkbox-label log-follow">
          <input
            type="checkbox"
            checked={following}
            onChange={(e) => setFollowing(e.target.checked)}
          />
          Follow output
        </label>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => {
            void client.resetQueries({ queryKey: ["logs", id], exact: true });
          }}
        >
          Replay logs
        </Button>
      </div>
      <p className="footnote">
        Sanitized complete lines · latest 500 · cursor {logs.data?.cursor ?? 0}.
        Lines over 16 KiB are withheld. Raw provider result fields and
        credentials are filtered in Rust.
      </p>
      {logs.isError && (
        <ErrorNotice error={logs.error} retry={() => void logs.refetch()} />
      )}
      <div
        ref={output}
        className="terminal"
        role="log"
        aria-live="off"
        aria-label="Process output"
      >
        {logs.data?.items.map((line, index) => (
          <div
            key={`${line.sequence}-${index}`}
            className={
              line.stream === "stderr" || line.stream === "warning"
                ? "log-warning"
                : ""
            }
          >
            <span title={line.timestamp}>
              {line.timestamp
                ? new Date(line.timestamp).toLocaleTimeString([], {
                    hour12: false,
                  })
                : "—"}{" "}
              · {line.stream}
            </span>
            <code>{line.text}</code>
          </div>
        ))}
        {!logs.data?.items.length && (
          <p>No complete output lines received yet.</p>
        )}
      </div>
    </>
  );
}
