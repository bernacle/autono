import { appendHistory } from "./execution-model";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTransport } from "./transport";
import { appendEvents, type CursorPage } from "./utils";
import type { EngineeringEvent, LogLine, LogPage } from "./types";
export function useEvents(id: string) {
  const transport = useTransport(),
    client = useQueryClient(),
    key = ["events", id];
  return useQuery({
    queryKey: key,
    queryFn: async () => {
      const previous = client.getQueryData<CursorPage<EngineeringEvent>>(key);
      return appendEvents(
        previous,
        await transport.request<EngineeringEvent[]>({
          type: "events",
          id,
          after: previous?.cursor ?? 0,
        }),
      );
    },
    refetchInterval: (q) => (q.state.data?.hasMore ? 250 : 2000),
    retry: false,
  });
}
export function useLogs(id: string) {
  const transport = useTransport(),
    client = useQueryClient(),
    key = ["logs", id];
  return useQuery({
    queryKey: key,
    queryFn: async () => {
      const previous = client.getQueryData<{
        items: LogLine[];
        cursor: number;
        hasMore: boolean;
      }>(key);
      const page = await transport.request<LogPage>({
        type: "process_events",
        id,
        after: previous?.cursor ?? 0,
      });
      return {
        items: [...(previous?.items ?? []), ...page.lines].slice(-500),
        cursor: page.after,
        hasMore: page.has_more,
      };
    },
    refetchInterval: (q) => (q.state.data?.hasMore ? 250 : 2000),
    retry: false,
  });
}

/** Replays from zero once per immutable plan. Global sequence gaps are expected. */
export function useExecutionHistory(run: import("./types").Run | undefined) {
  const transport = useTransport(),
    client = useQueryClient();
  const ids = run?.tasks.map((t) => t.plan.id).sort() ?? [];
  const key = ["execution", run?.id, ids.join(",")];
  return useQuery({
    queryKey: key,
    enabled: !!run,
    queryFn: async () => {
      const previous =
        client.getQueryData<import("./execution-model").History>(key);
      const page = await transport.request<EngineeringEvent[]>({
        type: "events",
        id: run!.id,
        after: previous?.cursor ?? 0,
      });
      return appendHistory(previous, run!.id, new Set(ids), page);
    },
    refetchInterval: (q) =>
      q.state.data && !q.state.data.caughtUp ? 250 : 2000,
    retry: false,
    gcTime: 60_000,
  });
}
