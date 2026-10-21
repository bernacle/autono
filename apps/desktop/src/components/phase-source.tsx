import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Source } from "../lib/execution-model";
import { sourceLabel } from "../lib/execution-model";
import type { EngineeringEvent } from "../lib/types";
import { useTransport } from "../lib/transport";
import { timeLabel } from "../lib/execution";
import { ErrorNotice } from "./common";
export function PhaseSource({
  runId,
  source,
}: {
  runId: string;
  source: Source;
}) {
  const [open, setOpen] = useState(false),
    transport = useTransport();
  const q = useQuery({
    queryKey: ["source", runId, source.sequence],
    enabled: open && source.sequence !== null,
    queryFn: async () => {
      const events = await transport.request<EngineeringEvent[]>({
        type: "events",
        id: runId,
        after: source.sequence! - 1,
      });
      return (
        events.find(
          (e) => e.sequence === source.sequence && e.run_id === runId,
        ) ?? null
      );
    },
    retry: false,
    staleTime: Infinity,
    gcTime: 60000,
  });
  return (
    <details onToggle={(e) => setOpen(e.currentTarget.open)}>
      <summary>
        <code>{sourceLabel(source)}</code> ·{" "}
        {timeLabel(Date.parse(source.timestamp))}
      </summary>
      {source.sequence === null ? (
        <p>Authoritative human-action audit in this run snapshot.</p>
      ) : q.isError ? (
        <ErrorNotice error={q.error} retry={() => void q.refetch()} />
      ) : q.data ? (
        <pre>{q.data.detail}</pre>
      ) : (
        <p>
          {q.isPending
            ? "Loading original sanitized evidence…"
            : "Source record unavailable; no replacement inferred."}
        </p>
      )}
    </details>
  );
}
