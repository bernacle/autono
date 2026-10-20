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
