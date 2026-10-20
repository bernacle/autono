import { useState } from "react";
import { useQuery, useMutation } from "@tanstack/react-query";
import * as Tabs from "@radix-ui/react-tabs";
import {
  ArrowLeft,
  GitBranch,
  Play,
  RefreshCw,
  Square,
  GitMerge,
  Folder,
  TerminalSquare,
} from "lucide-react";
import type {
  Run,
  RunAction,
  Session,
  Verification,
  Workspace,
} from "../lib/types";
import { useTransport } from "../lib/transport";
import { can, date, pending, short, title } from "../lib/utils";
import { Badge, Empty, ErrorNotice, Panel } from "./common";
import { Button } from "./ui/button";
import { Dialog } from "./ui/dialog";
import { DependencyGraph } from "./dependency-graph";
import { PreflightChecklist } from "./preflight";
import { HumanActionCard } from "./human-action";
import { ConfirmAction } from "./confirm-action";
import { Activity, ProcessLogs } from "./activity";
export function RunDetail({
  id,
  connected,
  onBack,
}: {
  id: string;
  connected: boolean;
  onBack: () => void;
}) {
  const transport = useTransport();
  const query = useQuery({
    queryKey: ["run", id],
    queryFn: () => transport.request<Run>({ type: "run", id }),
    refetchInterval: 2500,
    retry: false,
  });
  const [confirmation, setConfirmation] = useState<{
      run: Run;
      action: RunAction;
    } | null>(null),
    [notice, setNotice] = useState(""),
    [process, setProcess] = useState<string | null>(null);
  if (!query.data)
    return (
      <>
        {query.isError ? (
          <ErrorNotice error={query.error} retry={() => void query.refetch()} />
        ) : (
          <Empty title="Loading engineering run…" />
        )}
        <Button variant="ghost" onClick={onBack}>
          <ArrowLeft size={15} />
          All runs
        </Button>
      </>
    );
  const run = query.data,
    enabled = connected && !query.isError;
  const ask = (action: RunAction) => {
    setNotice("");
    setConfirmation({ run, action });
  };
  const done = run.tasks.filter((t) => t.status === "COMPLETED").length;
  return (
    <>
      <div className="breadcrumb">
        <button onClick={onBack}>
          <ArrowLeft size={14} />
          Runs
        </button>
        <span>/</span>
        <code>{short(id)}</code>
        <span>revision {run.revision}</span>
      </div>
      <div className="page-heading">
        <div>
          <div className="eyebrow">Engineering run</div>
          <h1>{title(run)}</h1>
          <p>
            <Folder size={14} />
            {run.repository}
            <span>·</span>
            <GitBranch size={14} />
            {run.requested_base} @ {short(run.base_commit)}
          </p>
        </div>
        <Badge status={run.status} />
      </div>
      {query.isError && (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      )}
      <div className="run-toolbar">
        <div className="row">
          <Button
            disabled={!enabled || !can(run, "execute")}
            onClick={() => ask("execute")}
          >
            <Play size={14} />
            Approve execution
          </Button>
          <Button
            variant="secondary"
            disabled={!enabled || !can(run, "continue")}
            onClick={() => ask("continue")}
          >
            <Play size={14} />
            Continue
          </Button>
          <Button
            variant="secondary"
            disabled={!enabled || !can(run, "integrate")}
            onClick={() => ask("integrate")}
          >
            <GitMerge size={15} />
            Integrate
          </Button>
        </div>
        <Button
          variant="ghost"
          disabled={!enabled || !can(run, "cancel")}
          onClick={() => ask("cancel")}
        >
          <Square size={13} />
          Cancel
        </Button>
      </div>
      {notice && (
        <div role="status" className="notice">
          {notice}
        </div>
      )}
      {run.status === "INTERRUPTED" && (
        <div className="notice">
          This run was interrupted. The daemon does not support resuming
          interrupted workflows; inspect preserved evidence and create a new
          plan.
        </div>
      )}
      {run.blockers.length > 0 &&
        ["BLOCKED", "FAILED", "INTERRUPTED"].includes(run.status) && (
          <div className="notice notice-warning">
            <div>
              <strong>{run.summary || "Run needs inspection"}</strong>
              {run.blockers.map((b, i) => (
                <p key={i}>
                  {b.task_id && `${b.task_id}: `}
                  {b.kind} — {b.detail}
                  <br />
                  {b.action}
                </p>
              ))}
            </div>
          </div>
        )}
      <div className="run-facts">
        <div>
          <span>Tasks completed</span>
          <strong>
            {done} <small>/ {run.tasks.length}</small>
          </strong>
        </div>
        <div>
          <span>Human actions</span>
          <strong>
            {run.human_actions.filter(pending).length} <small>unresolved</small>
          </strong>
        </div>
        <div>
          <span>Approval</span>
          <strong className="text-value">
            {run.approved_at
              ? date(run.approved_at)
              : "Awaiting explicit approval"}
          </strong>
        </div>
        <div>
          <span>Last update</span>
          <strong className="text-value">{date(run.updated_at)}</strong>
        </div>
      </div>
      <Tabs.Root defaultValue="plan">
        <Tabs.List className="tabs" aria-label="Run details">
          {[
            ["plan", "Plan & tasks"],
            ["preflight", "Preflight & human actions"],
            ["activity", "Live activity"],
            ["evidence", "Verification & reviews"],
            ["artifacts", "Workspaces & integration"],
          ].map(([value, label]) => (
            <Tabs.Trigger key={value} value={value}>
              {label}
            </Tabs.Trigger>
          ))}
        </Tabs.List>
        <Tabs.Content value="plan">
          <Panel
            title="Task dependencies"
            meta={
              <span className="muted">
                {done}/{run.tasks.length} completed
              </span>
            }
          >
            {run.tasks.length ? (
              <DependencyGraph tasks={run.tasks} />
            ) : (
              <Empty title="Waiting for the planner">
                The daemon will publish a validated plan here.
              </Empty>
            )}
          </Panel>
          {run.plan && (
            <Panel title="Milestones">
              <div className="milestones">
                {run.plan.milestones.map((m) => {
                  const completed = m.task_ids.filter((id) =>
                    run.tasks.some(
                      (t) => t.plan.id === id && t.status === "COMPLETED",
                    ),
                  ).length;
                  return (
                    <div key={m.id}>
                      <span className="micro-tag">{m.id}</span>
                      <strong>{m.goal}</strong>
                      <span>
                        {completed}/{m.task_ids.length}
                      </span>
                      <progress value={completed} max={m.task_ids.length} />
                      <small>{m.task_ids.join(" · ")}</small>
                    </div>
                  );
                })}
              </div>
            </Panel>
          )}
          <Panel title="Original specification">
            <pre className="specification">{run.specification}</pre>
          </Panel>
          {run.plan && (
            <Panel title="Requirements">
              <div className="requirements">
                {run.plan.requirements.map((r) => (
                  <p key={r.id}>
                    <code>{r.id}</code>
                    {r.text}
                  </p>
                ))}
              </div>
            </Panel>
          )}
          {run.tasks.map((task) => (
            <Panel
              key={task.plan.id}
              title={`${task.plan.id} · ${task.plan.goal}`}
              meta={<Badge status={task.status} />}
            >
              <div className="task-content" id={`task-${task.plan.id}`}>
                <p>{task.plan.description}</p>
                <div className="form-grid">
                  <div>
                    <h4>Acceptance criteria</h4>
                    <ul>
                      {task.plan.acceptance_criteria.map((c, i) => (
                        <li key={i}>{c}</li>
                      ))}
                    </ul>
                  </div>
                  <div>
                    <h4>Verification expectations</h4>
                    <ul>
                      {task.plan.verification_expectations.map((c, i) => (
                        <li key={i}>{c}</li>
                      ))}
                    </ul>
                  </div>
                </div>
                <small>
                  Dependencies: {task.plan.dependencies.join(", ") || "none"} ·
                  Requirements: {task.plan.requirement_ids.join(", ")} ·
                  Attempts: {task.attempts.length}/
                  {run.settings.execution.max_attempts}
                </small>
              </div>
            </Panel>
          ))}
        </Tabs.Content>
        <Tabs.Content value="preflight">
          <Panel
            title="Preflight readiness"
            meta={
              <Button
                variant="secondary"
                size="sm"
                disabled={!enabled || !can(run, "preflight")}
                onClick={() => ask("preflight")}
              >
                <RefreshCw size={13} />
                Refresh preflight
              </Button>
            }
          >
            <PreflightChecklist report={run.preflight} />
          </Panel>
          <Panel
            title="Human actions"
            meta={<span className="muted">Resolutions never start agents</span>}
          >
            <div className="human-actions">
              {run.human_actions.map((a) => (
                <HumanActionCard
                  key={a.id}
                  run={run}
                  action={a}
                  connected={enabled}
                />
              ))}
              {!run.human_actions.length && <Empty title="No human actions" />}
            </div>
          </Panel>
        </Tabs.Content>
        <Tabs.Content value="activity">
          <Panel title="Agent sessions">
            <SessionRow session={run.planner} onProcess={setProcess} />
            {run.tasks.flatMap((task) =>
              task.attempts.map((attempt) => (
                <div key={`${task.plan.id}-${attempt.number}`}>
                  <div className="session-label">
                    {task.plan.id} · attempt {attempt.number}
                  </div>
                  <SessionRow session={attempt.coder} onProcess={setProcess} />
                  {attempt.reviews.length ? (
                    attempt.reviews.map((r) => (
                      <SessionRow
                        key={r.session.id}
                        session={r.session}
                        onProcess={setProcess}
                      />
                    ))
                  ) : (
                    <SessionRow
                      session={attempt.reviewer}
                      onProcess={setProcess}
                    />
                  )}
                </div>
              )),
            )}
          </Panel>
          <Panel title="Engineering event journal">
            <Activity id={id} />
          </Panel>
        </Tabs.Content>
        <Tabs.Content value="evidence">
          {run.tasks.map((task) => (
            <Panel
              key={task.plan.id}
              title={`${task.plan.id} · ${task.plan.goal}`}
              meta={<Badge status={task.status} />}
            >
              {!task.attempts.length && (
                <Empty title="No attempt evidence yet" />
              )}
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
        </Tabs.Content>
        <Tabs.Content value="artifacts">
          <Panel title="Local integration">
            <div className="task-content">
              <Badge status={run.status} />
              <p>{run.summary}</p>
              <p className="muted">
                Verified task commits require a separate local integration and
                combined checks.
              </p>
              {run.integration_workspace && (
                <WorkspaceRow
                  runId={id}
                  workspace={run.integration_workspace}
                />
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
        </Tabs.Content>
      </Tabs.Root>
      {confirmation && (
        <ConfirmAction
          {...confirmation}
          connected={enabled}
          onClose={() => setConfirmation(null)}
          onAccepted={setNotice}
        />
      )}{" "}
      {process && (
        <Dialog
          wide
          title="Process output"
          description="Read from durable daemon process events."
          onClose={() => setProcess(null)}
        >
          <ProcessLogs id={process} />
        </Dialog>
      )}
    </>
  );
}
function SessionRow({
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
function VerificationList({
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
          <p>{result.summary}</p>
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
function WorkspaceRow({
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
