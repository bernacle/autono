import { useMutation } from "@tanstack/react-query";
import { Folder, TerminalSquare } from "lucide-react";
import type { Run, Session, Task, Verification, Workspace } from "../lib/types";
import {
  attemptChecks,
  checkLabels,
  reviewAccepts,
  reviewRecords,
  verificationOutcome,
  type Attempt,
} from "../lib/evidence-model";
import { useTransport } from "../lib/transport";
import { short } from "../lib/utils";
import { Badge, Empty, ErrorNotice, Panel } from "./common";
import { Button } from "./ui/button";
export function TaskEvidence({
  run,
  onProcess,
  compact = false,
}: {
  run: Run;
  onProcess: (id: string) => void;
  compact?: boolean;
}) {
  const setProcess = onProcess;
  return (
    <>
      {run.tasks.map((task) => (
        <Panel
          key={task.plan.id}
          title={
            compact ? "Attempt evidence" : `${task.plan.id} · ${task.plan.goal}`
          }
          meta={<Badge status={task.status} />}
        >
          {!task.attempts.length && <Empty title="No attempt evidence yet" />}
          {task.attempts.map((attempt) => (
            <div className="attempt" key={attempt.number}>
              <h3>Attempt {attempt.number}</h3>
              {attempt.detail && (
                <p className="warning-text">{attempt.detail}</p>
              )}
              <p>
                <CheckSummary run={run} task={task} attempt={attempt} />
              </p>
              <VerificationList
                results={attempt.verification}
                onProcess={setProcess}
              />
              {reviewRecords(attempt).map(
                ({ ordinal, session, review, diagnostic }) => (
                  <div
                    className="review"
                    key={session?.id ?? `legacy-${ordinal}`}
                    data-review-ordinal={ordinal}
                    data-review-session={session?.id}
                  >
                    <div className="row-between">
                      <h4>Independent review {ordinal}</h4>
                      <Badge
                        status={
                          review
                            ? reviewAccepts(review)
                              ? "review accepted"
                              : "REJECTED"
                            : "review unavailable"
                        }
                      />
                    </div>
                    <SessionRow session={session} onProcess={setProcess} />
                    {diagnostic && <p className="warning-text">{diagnostic}</p>}
                    {review ? (
                      <>
                        {review.blocker && (
                          <p className="warning-text">{review.blocker}</p>
                        )}
                        {review.criteria.map((c, i) => (
                          <details key={i}>
                            <summary>
                              {c.satisfied ? "✓" : "✕"} {c.criterion_id}{" "}
                              {c.criterion}
                            </summary>
                            <p>{c.evidence}</p>
                          </details>
                        ))}
                        {(review.decision_reviews ?? []).map((d) => (
                          <details key={d.decision_id}>
                            <summary>
                              {d.implemented ? "✓" : "✕"} Decision{" "}
                              {d.decision_id}
                            </summary>
                            <p>{d.evidence}</p>
                          </details>
                        ))}
                        {review.findings.map((finding, i) => (
                          <div className="finding" key={i}>
                            <strong>
                              {finding.severity} · {finding.file}:{finding.line}
                            </strong>
                            <p>{finding.explanation}</p>
                            <p>Recommended fix: {finding.recommended_fix}</p>
                            <small>
                              Confidence {Math.round(finding.confidence * 100)}%
                            </small>
                          </div>
                        ))}
                        {!review.findings.length && (
                          <p className="muted">No reviewer findings.</p>
                        )}
                      </>
                    ) : (
                      <p className="muted">No validated review result.</p>
                    )}
                  </div>
                ),
              )}
              <dl className="tree-evidence">
                <dt>Verified tree</dt>
                <dd>{attempt.verified_tree ?? "No verified tree"}</dd>
                <dt>Preserved tree</dt>
                <dd>{attempt.preserved_tree ?? "No preserved tree"}</dd>
                <dt>Task final commit</dt>
                <dd>{task.artifact_commit ?? "Not preserved"}</dd>
              </dl>
            </div>
          ))}
        </Panel>
      ))}
      {!run.tasks.length && (
        <Empty title="Verification starts after plan approval" />
      )}
    </>
  );
}
export function IntegrationEvidence({
  run,
  onProcess,
}: {
  run: Run;
  onProcess: (id: string) => void;
}) {
  const setProcess = onProcess;
  const id = run.id;
  return (
    <>
      <Panel title="Local integration">
        <div className="task-content">
          <Badge status={run.status} />
          <details className="audit">
            <summary>Daemon summary</summary>
            <p>{run.summary}</p>
          </details>
          <p className="muted">
            Verified task commits require a separate local integration and
            combined checks.
          </p>
          {run.integration_workspace && (
            <WorkspaceRow runId={id} workspace={run.integration_workspace} />
          )}
          <VerificationList
            results={run.integration_verification}
            onProcess={setProcess}
          />
        </div>
      </Panel>
      <Panel title="Associated workspaces">
        <WorkspaceRow runId={id} workspace={run.planning_workspace} />
        {run.tasks.map(
          (task) =>
            task.workspace && (
              <WorkspaceRow
                key={task.plan.id}
                runId={id}
                workspace={task.workspace}
              />
            ),
        )}
        {run.tasks.flatMap((t) =>
          t.attempts.flatMap((a) =>
            a.reviews.map(
              (r) =>
                r.workspace && (
                  <WorkspaceRow
                    key={r.session.id}
                    runId={id}
                    workspace={r.workspace}
                  />
                ),
            ),
          ),
        )}
      </Panel>
    </>
  );
}
export function SessionRow({
  session,
  onProcess,
}: {
  session: Session | null;
  onProcess: (id: string) => void;
}) {
  if (!session) return null;
  return (
    <div className="session">
      <span className="provider-avatar">
        {session.provider.slice(0, 1).toUpperCase()}
      </span>
      <div>
        <strong>
          {session.provider} <span className="muted">/ {session.role}</span>
        </strong>
        <small>
          {session.provider_session_id ?? `Session ${short(session.id)}`}
        </small>
      </div>
      <Badge status={session.outcome ?? "Outcome pending"} />
      {session.process_run_id ? (
        <Button
          variant="ghost"
          size="sm"
          onClick={() => onProcess(session.process_run_id!)}
        >
          <TerminalSquare size={14} />
          Logs
        </Button>
      ) : (
        <small className="muted">
          Awaiting process · {short(session.process_key)}
        </small>
      )}
    </div>
  );
}
export function CheckSummary({
  run,
  task,
  attempt,
  compact = false,
}: {
  run: Run;
  task: Task;
  attempt: Attempt;
  compact?: boolean;
}) {
  const summary = attemptChecks(run, task, attempt),
    symbol =
      summary.state === "passed"
        ? "✓"
        : summary.state === "test_failure"
          ? "✕"
          : ["partial", "pending"].includes(summary.state)
            ? "…"
            : "—",
    tone =
      summary.state === "passed"
        ? "success-text"
        : summary.state === "test_failure"
          ? "danger-text"
          : ["timeout", "infrastructure_failure", "partial"].includes(
                summary.state,
              )
            ? "warning-text"
            : "muted";
  return (
    <span className={tone} title={summary.label} aria-label={summary.label}>
      {symbol}
      {!compact && ` ${summary.label}`}
    </span>
  );
}
export function VerificationList({
  results,
  onProcess,
}: {
  results: Verification[];
  onProcess: (id: string) => void;
}) {
  return (
    <div className="verification-list">
      {results.map((result, i) => (
        <div key={`${result.process_run_id}-${i}`}>
          <div className="row-between">
            <code>{JSON.stringify(result.command)}</code>
            <div className="row">
              <Badge
                status={verificationOutcome(result)}
                label={checkLabels[verificationOutcome(result)]}
              />
              <Button
                variant="ghost"
                size="sm"
                onClick={() => onProcess(result.process_run_id)}
              >
                <TerminalSquare size={14} />
                Output
              </Button>
            </div>
          </div>
          <details className="audit">
            <summary>Verification details</summary>
            <p>{result.summary}</p>
          </details>
          <small>
            Exit {result.exit_code ?? "unknown"} · output{" "}
            {result.output_complete ? "complete" : "incomplete"} ·{" "}
            {short(result.process_run_id)}
          </small>
        </div>
      ))}
    </div>
  );
}
export function WorkspaceRow({
  runId,
  workspace,
}: {
  runId: string;
  workspace: Workspace;
}) {
  const transport = useTransport();
  const inspect = useMutation({
    mutationFn: () => transport.inspectArtifact(runId, workspace.path),
    retry: false,
  });
  return (
    <div className="workspace">
      <div className="row-between">
        <div>
          <div className="row">
            <Folder size={15} />
            <strong>{workspace.branch}</strong>
          </div>
          <code>{workspace.path}</code>
          <small>
            Base {short(workspace.base_commit)} · head{" "}
            {workspace.head_commit
              ? short(workspace.head_commit)
              : "not published"}
          </small>
        </div>
        <Button
          variant="secondary"
          size="sm"
          disabled={inspect.isPending}
          onClick={() => inspect.mutate()}
        >
          Check access
        </Button>
      </div>
      {inspect.isSuccess && (
        <p className="success-text" role="status">
          Directory is accessible.
        </p>
      )}
      {inspect.isError && <ErrorNotice error={inspect.error} />}
    </div>
  );
}
