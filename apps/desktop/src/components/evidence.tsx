import { useMutation } from "@tanstack/react-query";
import { Folder, TerminalSquare } from "lucide-react";
import type { Run, Session, Verification, Workspace } from "../lib/types";
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
              <VerificationList
                results={attempt.verification}
                onProcess={setProcess}
              />
              {(attempt.reviews.length
                ? attempt.reviews.map((r) => r.review)
                : [attempt.review]
              )
                .filter((r) => r !== null)
                .map((review, index) => (
                  <div className="review" key={index}>
                    <div className="row-between">
                      <h4>Independent review {index + 1}</h4>
                      <Badge
                        status={
                          review.accepted ? "review accepted" : "REJECTED"
                        }
                      />
                    </div>
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
                  </div>
                ))}
              {attempt.reviews.map(
                (r, i) =>
                  r.diagnostic && (
                    <p className="warning-text" key={i}>
                      {r.diagnostic}
                    </p>
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
      <Badge status={session.outcome ?? "RUNNING"} />
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
                status={result.outcome ?? (result.passed ? "passed" : "FAILED")}
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
