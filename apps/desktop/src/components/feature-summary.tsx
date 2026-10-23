import type { Run } from "../lib/types";
import { useEvents } from "../lib/queries";
import { can, date, title } from "../lib/utils";
import { Button } from "./ui/button";
import { Decisions } from "./decisions";
import { ErrorNotice, Panel } from "./common";

export function FeatureSummary({
  run,
  connected,
  onDetails,
  onHuman,
  onCancel,
  onRefresh,
}: {
  run: Run;
  connected: boolean;
  onDetails: () => void;
  onHuman: (id: string) => void;
  onCancel: () => void;
  onRefresh: () => void;
}) {
  const events = useEvents(run.id),
    report = run.handoff;
  const ready =
    (report?.outcome === "completed" &&
      report.completion_requirements_satisfied) ||
    report?.outcome === "verified_locally";
  const stopped = ["BLOCKED", "FAILED", "CANCELLED", "INTERRUPTED"].includes(
    run.status,
  );
  const headline = ready
    ? "Ready for review"
    : report?.outcome === "evidence_incomplete"
      ? "Needs inspection — evidence incomplete"
      : report?.outcome === "partially_completed"
        ? "Partially completed — needs your input"
        : stopped
          ? `${run.status[0]}${run.status.slice(1).toLowerCase()} — inspect retained work`
          : run.status === "PLANNING"
            ? "Planning your feature"
            : run.status === "INTEGRATING"
              ? "Checking the combined feature"
              : "Building your feature";
  const meaningful = events.data?.items
    .filter(
      (e) =>
        ![
          "delegation_allowed",
          "agent_process",
          "verification_process",
        ].includes(e.kind),
    )
    .at(-1);
  const files = [
    ...new Set(
      events.data?.items
        .filter((e) => e.kind === "source_edits_applied")
        .flatMap((e) => {
          try {
            const paths: unknown = JSON.parse(e.detail);
            return Array.isArray(paths)
              ? paths.filter((p): p is string => typeof p === "string")
              : [];
          } catch {
            return [];
          }
        }) ?? [],
    ),
  ];
  return (
    <div className="build-page feature-summary">
      <span className="eyebrow">Autonomous feature · {run.id.slice(0, 8)}</span>
      <h1>{headline}</h1>
      <h2>{title(run)}</h2>
      <p className="muted">
        {run.repository} · base {run.base_commit.slice(0, 12)}
      </p>
      <p>{report?.summary ?? run.summary}</p>
      {!connected && (
        <p className="notice notice-warning">
          Connection unavailable. This view may be stale; the daemon owns
          execution independently of this window.
        </p>
      )}
      <Panel title="Verified work">
        <div className="handoff-content">
          <p>
            {report?.tasks.filter(
              (t) => t.delivery === "independently_verified_locally",
            ).length ?? 0}{" "}
            of {run.tasks.length} tasks independently verified locally.
          </p>
          <ul>
            {report?.tasks.map((t) => (
              <li key={t.task_id}>
                {run.tasks.find((task) => task.plan.id === t.task_id)?.plan
                  .goal ?? t.task_id}
                : {t.delivery.replaceAll("_", " ")}
              </li>
            ))}
          </ul>
          {run.tasks
            .filter((t) => t.status === "RUNNING")
            .map((t) => (
              <p key={t.plan.id}>Working on: {t.plan.goal}</p>
            ))}
          <p>
            Combined checks:{" "}
            {report?.combined_checks_passed
              ? "passed"
              : "no complete passing receipt"}
            .
          </p>
        </div>
      </Panel>
      {!!report?.needs_you.length && (
        <Panel title="Needs your input">
          <div className="handoff-content">
            {report.needs_you.map((need) => (
              <div key={need.action_id}>
                <p>{need.requested_action}</p>
                <Button
                  variant="secondary"
                  onClick={() => onHuman(need.action_id)}
                >
                  Inspect exact request
                </Button>
              </div>
            ))}
            {can(run, "preflight") && (
              <Button
                variant="secondary"
                onClick={onRefresh}
                disabled={!connected}
              >
                Check repaired prerequisites
              </Button>
            )}
          </div>
        </Panel>
      )}
      {ready && (
        <Panel title="Review locally">
          <div className="handoff-content">
            <p>
              {run.integration_workspace?.head_commit
                ? `Integrated at ${run.integration_workspace.path} on ${run.integration_workspace.branch}.`
                : "Verified task branches are available in detailed views. Local integration was not completed."}
            </p>
            <p>
              Inspect the diff and configured verification evidence in detailed
              views. Your original branch remains separate.
            </p>
            {files.length > 0 && (
              <>
                <strong>Recorded source edits</strong>
                <ul>
                  {files.map((path) => (
                    <li key={path}>
                      <code>{path}</code>
                    </li>
                  ))}
                </ul>
              </>
            )}
          </div>
        </Panel>
      )}
      {(ready || stopped) && <Decisions ledger={run.decisions} />}
      <div className="build-last-progress">
        <strong>Last recorded progress</strong>
        <p>
          {meaningful
            ? `${date(meaningful.timestamp)} · ${meaningful.detail}`
            : "Waiting for durable activity evidence."}
        </p>
        {events.isError && <ErrorNotice error={events.error} />}
      </div>
      <div className="form-submit">
        <Button variant="secondary" onClick={onDetails}>
          Detailed views · tasks, timeline and evidence
        </Button>
        {can(run, "cancel") && (
          <Button variant="ghost" disabled={!connected} onClick={onCancel}>
            Cancel feature…
          </Button>
        )}
      </div>
      {run.status === "INTERRUPTED" && (
        <p className="notice">
          Inspect retained artifacts, then create a fresh delegation.
          Interrupted execution is never automatically replayed.
        </p>
      )}
    </div>
  );
}
