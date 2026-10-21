import type { Segment } from "../lib/execution-model";
import { useLogs } from "../lib/queries";
import { elapsed, timeLabel, useNow } from "../lib/execution";
import { StatusGlyph } from "./common";
import { Button } from "./ui/button";
export function AgentProgress({
  segment: s,
  onProcess,
  connected,
}: {
  segment: Segment;
  onProcess: (id: string) => void;
  connected: boolean;
}) {
  return (
    <div>
      <StatusGlyph status="RUNNING" />
      <strong>
        {!connected && "Last observed · "}
        {s.task ?? "Plan"} ·{" "}
        {s.phase === "review"
          ? "Independent reviewer"
          : s.phase === "coding"
            ? "Coding agent"
            : "Planner"}
      </strong>
      <span className="meta">{s.provider}</span>
      {s.process && (
        <>
          <OutputProgress id={s.process} connected={connected} />
          <Button
            size="sm"
            variant="ghost"
            onClick={() => onProcess(s.process!)}
          >
            Output
          </Button>
        </>
      )}
    </div>
  );
}
function OutputProgress({ id, connected }: { id: string; connected: boolean }) {
  const logs = useLogs(id),
    now = useNow(),
    last = logs.data?.items
      .filter((l) => l.stream === "stdout" || l.stream === "stderr")
      .at(-1);
  return (
    <span className="meta agent-last-output">
      {logs.isError
        ? "Output unavailable"
        : last?.timestamp
          ? `Last output ${timeLabel(Date.parse(last.timestamp))}${connected ? ` · ${elapsed(now - Date.parse(last.timestamp))} ago` : ""}`
          : "No complete output recorded"}
    </span>
  );
}
