import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useEvents, useLogs } from "../lib/queries";
import { date } from "../lib/utils";
import { Empty, ErrorNotice } from "./common";
import { Button } from "./ui/button";
export function Activity({
  id,
  compact = false,
}: {
  id: string;
  compact?: boolean;
}) {
  const events = useEvents(id),
    [filter, setFilter] = useState("all");
  const items = events.data?.items ?? [],
    kinds = [...new Set(items.map((e) => e.kind))].sort();
  const visible = items
    .filter((e) => filter === "all" || e.kind === filter)
    .slice(compact ? -8 : -500);
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
      {events.isError && (
        <ErrorNotice error={events.error} retry={() => void events.refetch()} />
      )}
      <div className="event-list">
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
  return (
    <>
      <div className="row-between">
        <code>{id}</code>
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
      <div className="terminal" role="log" aria-label="Process output">
        {logs.data?.items.map((line, index) => (
          <div
            key={`${line.sequence}-${index}`}
            className={
              line.stream === "stderr" || line.stream === "warning"
                ? "log-warning"
                : ""
            }
          >
            <span>{line.stream}</span>
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
