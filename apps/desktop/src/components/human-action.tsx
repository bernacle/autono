import { useEffect, useId, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, ArrowRight, Check, ShieldCheck } from "lucide-react";
import type { Run, HumanAction, Resolution } from "../lib/types";
import {
  canApprove,
  canResolve,
  date,
  pending,
  short,
  title,
} from "../lib/utils";
import { humanize } from "../lib/presentation";
import { useTransport } from "../lib/transport";
import { Badge, Empty, ErrorNotice, Field } from "./common";
import { Button } from "./ui/button";
import { CopyButton } from "./ui/copy-button";
import { Segmented } from "./ui/segmented";

// Session-only receipts survive remounts. Exact payload identity preserves an
// identical retry after an ambiguous response, without a second resolution.
const receipts = new Map<string, string>();
function receiptId(runId: string, actionId: string, payload: unknown) {
  const key = JSON.stringify([runId, actionId, payload]);
  let id = receipts.get(key);
  if (!id) {
    id = crypto.randomUUID();
    receipts.set(key, id);
  }
  return id;
}
function fingerprint(action: HumanAction) {
  return JSON.stringify([
    action.status,
    action.requirement,
    action.audit.length,
  ]);
}
export type HumanItem = { run: Run; action: HumanAction };
export function HumanActionsView({
  runs,
  connected,
  initial,
  onRun,
  onOlder,
  loadingOlder = false,
}: {
  runs: Run[];
  connected: boolean;
  initial?: string | null;
  onRun: (id: string) => void;
  onOlder?: () => void;
  loadingOlder?: boolean;
}) {
  const all = runs.flatMap((run) =>
    run.human_actions.map((action) => ({ run, action })),
  );
  const open = all.filter(
    ({ run, action }) => canResolve(run) && pending(action),
  );
  const [chosen, setChosen] = useState(
    () => initial ?? (open[0] ? `${open[0].run.id}/${open[0].action.id}` : ""),
  );
  const selected =
    all.find((item) => `${item.run.id}/${item.action.id}` === chosen) ??
    open[0];
  const shown =
    selected && !open.includes(selected) ? [selected, ...open] : open;
  const selectedKey = selected
    ? `${selected.run.id}/${selected.action.id}`
    : "";
  return (
    <div className="human-workspace">
      <aside
        className="human-queue"
        aria-label="Open human actions"
        onKeyDown={(e) => {
          if (!["j", "k", "ArrowDown", "ArrowUp"].includes(e.key)) return;
          const rows = [
            ...e.currentTarget.querySelectorAll<HTMLButtonElement>(
              "button[data-action]",
            ),
          ];
          const index = rows.indexOf(
            document.activeElement as HTMLButtonElement,
          );
          if (index < 0) return;
          e.preventDefault();
          rows[
            Math.max(
              0,
              Math.min(
                rows.length - 1,
                index + (["j", "ArrowDown"].includes(e.key) ? 1 : -1),
              ),
            )
          ]?.focus();
        }}
      >
        <div className="queue-heading">
          Needs you <span className="count">{open.length}</span>
        </div>
        {shown.map(({ run, action }) => (
          <button
            data-action
            key={`${run.id}/${action.id}`}
            aria-current={
              selectedKey === `${run.id}/${action.id}` ? "true" : undefined
            }
            onClick={() => setChosen(`${run.id}/${action.id}`)}
          >
            <div className="row-between">
              <span className="meta">{humanize(action.requirement.kind)}</span>
              <Badge status={action.status} />
            </div>
            <strong>{action.requirement.requested_action}</strong>
            <small>{title(run)}</small>
            <small>
              {action.requirement.affected_tasks.join(", ") || "Whole run"} ·{" "}
              {action.requirement.blocking ? "Blocks execution" : "Advisory"}
            </small>
          </button>
        ))}
        {!open.length && (
          <p className="footnote">
            No actions requiring resolution in these runs.
          </p>
        )}
        <p className="footnote">Showing {runs.length} recent runs.</p>
        {onOlder && (
          <Button
            variant="ghost"
            size="sm"
            onClick={onOlder}
            disabled={loadingOlder}
          >
            Load older runs
          </Button>
        )}
      </aside>
      <div className="human-detail">
        {selected ? (
          <>
            <div className="human-switcher">
              <label>
                Open human action
                <select
                  aria-label="Open human action"
                  value={selectedKey}
                  onChange={(e) => setChosen(e.target.value)}
                >
                  {shown.map(({ run, action }) => (
                    <option
                      key={`${run.id}/${action.id}`}
                      value={`${run.id}/${action.id}`}
                    >
                      {action.requirement.requested_action} · {title(run)}
                    </option>
                  ))}
                </select>
              </label>
              <span className="meta">{open.length} open</span>
              {onOlder && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={onOlder}
                  disabled={loadingOlder}
                >
                  Load older runs
                </Button>
              )}
            </div>
            <ResolutionPanel
              key={selectedKey}
              run={selected.run}
              action={selected.action}
              editable={
                connected &&
                pending(selected.action) &&
                canResolve(selected.run)
              }
              onRun={() => onRun(selected.run.id)}
            />
          </>
        ) : (
          <Empty title="No unresolved human actions">
            Resolved decisions and audit history remain on each run.
            {onOlder && (
              <Button
                variant="secondary"
                size="sm"
                onClick={onOlder}
                disabled={loadingOlder}
              >
                Load older runs
              </Button>
            )}
          </Empty>
        )}
      </div>
    </div>
  );
}
export function HumanActionCard({
  run,
  action,
  connected,
  onInspect,
}: {
  run: Run;
  action: HumanAction;
  connected: boolean;
  onInspect?: () => void;
}) {
  const [inspect, setInspect] = useState(false);
  return (
    <article className="human-card">
      <div className="row-between">
        <span className="meta">
          {humanize(action.requirement.kind)} ·{" "}
          {action.requirement.blocking ? "Blocks execution" : "Advisory"}
        </span>
        <Badge status={action.status} />
      </div>
      <h3>{action.requirement.requested_action}</h3>
      <p>{action.requirement.reason}</p>
      <div className="row-between">
        <span className="meta">
          {action.requirement.affected_tasks.join(", ") || "Whole run"}
        </span>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => (onInspect ? onInspect() : setInspect(true))}
        >
          Inspect {pending(action) && canResolve(run) ? "& resolve" : "record"}
          <ArrowRight size={13} />
        </Button>
      </div>
      {inspect && (
        <ResolutionDialog
          run={run}
          action={action}
          editable={connected && pending(action) && canResolve(run)}
          onClose={() => setInspect(false)}
        />
      )}
    </article>
  );
}
// Standalone callers use the same full-height panel. App navigation puts it in
// the Human actions sheet, never inside a scrolling approval modal.
export function ResolutionDialog({
  onClose,
  ...props
}: {
  run: Run;
  action: HumanAction;
  editable: boolean;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="resolution-standalone">
      <Button variant="ghost" onClick={onClose}>
        <ArrowLeft size={14} />
        Back
      </Button>
      <ResolutionPanel {...props} onResolved={onClose} />
    </div>
  );
}
export function ResolutionPanel({
  run,
  action,
  editable,
  onRun,
  onResolved,
}: {
  run: Run;
  action: HumanAction;
  editable: boolean;
  onRun?: () => void;
  onResolved?: () => void;
}) {
  const [snapshot, setSnapshot] = useState(() => ({
    revision: run.revision,
    action: structuredClone(action),
    fingerprint: fingerprint(action),
  }));
  const permission = snapshot.action.requirement.permission;
  const [disposition, setDisposition] = useState<
    Resolution["disposition"] | ""
  >("");
  const [note, setNote] = useState("");
  const [effect, setEffect] = useState<Resolution["effect"]>("unknown");
  const [confirmed, setConfirmed] = useState(false);
  const [accepted, setAccepted] = useState(false);
  const transport = useTransport(),
    client = useQueryClient();
  const changed = snapshot.fingerprint !== fingerprint(action);
  const allowed = disposition !== "approve" || canApprove(run, snapshot.action);
  const effectRequired =
    snapshot.action.continuable && ["approve", "resolve"].includes(disposition);
  const missing = [
    !disposition && "choose a decision",
    !allowed && "this permission cannot be approved",
    !note.trim() && "add a note",
    effectRequired && effect === "unknown" && "inspect the effect state",
    !confirmed && "confirm inspection",
  ].filter(Boolean);
  const valid = editable && !changed && !accepted && missing.length === 0;
  const finalLabel =
    disposition === "approve"
      ? `Approve ${permission?.operation}`
      : disposition === "resolve"
        ? "Resolve"
        : disposition === "reject"
          ? "Reject"
          : "Defer";
  const label = !disposition
    ? "Choose a decision"
    : !note.trim()
      ? "Add a note"
      : effectRequired && effect === "unknown"
        ? "Inspect effect state"
        : !confirmed
          ? "Confirm inspection"
          : finalLabel;
  const mutation = useMutation({
    mutationFn: () => {
      if (!valid || !disposition)
        throw new Error(
          "Review this action and complete the required inspection first.",
        );
      const payload = {
        expected_action_version: snapshot.action.audit.length,
        disposition,
        note,
        effect,
        permission: disposition === "approve" ? permission : null,
      };
      return transport.request({
        type: "resolve",
        id: run.id,
        revision: snapshot.revision,
        action_id: snapshot.action.id,
        resolution: {
          ...payload,
          request_id: receiptId(run.id, snapshot.action.id, payload),
        },
      });
    },
    retry: false,
    onSuccess: async () => {
      setAccepted(true);
      await client.invalidateQueries({
        predicate: (q) => !["events", "logs"].includes(String(q.queryKey[0])),
      });
      onResolved?.();
    },
    onError: () => {
      void client.invalidateQueries({ queryKey: ["run", run.id] });
      void client.invalidateQueries({ queryKey: ["runs"] });
    },
  });
  const reviewCurrent = () => {
    setSnapshot({
      revision: run.revision,
      action: structuredClone(action),
      fingerprint: fingerprint(action),
    });
    setDisposition("");
    setEffect("unknown");
    setConfirmed(false);
    setNote("");
    setAccepted(false);
    mutation.reset();
  };
  const req = snapshot.action.requirement;
  const needsId = useId();
  return (
    <section
      className="resolution-panel"
      aria-label="Human action decision"
      onKeyDown={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
          e.preventDefault();
          if (valid && !mutation.isPending) mutation.mutate();
        }
      }}
    >
      <div className="resolution-reading">
        <header className="resolution-pinned">
          <div className="row-between">
            <div className="row">
              <span className="micro-tag">{humanize(req.kind)}</span>
              {req.blocking && (
                <span className="attention-chip">Blocks execution</span>
              )}
              <Badge status={snapshot.action.status} />
            </div>
            {onRun && (
              <Button variant="ghost" size="sm" onClick={onRun}>
                Open run
                <ArrowRight size={13} />
              </Button>
            )}
          </div>
          <h1>{req.requested_action}</h1>
          <div className="exact-scope">
            <div className="row-between">
              <strong>
                <ShieldCheck size={14} />
                {permission ? "Exact permission" : "Exact requested action"}
              </strong>
              {permission && (
                <CopyButton
                  label="Copy JSON"
                  text={JSON.stringify(permission, null, 2)}
                />
              )}
            </div>
            {permission ? (
              <dl>
                <dt>operation</dt>
                <dd data-testid="exact-operation">{permission.operation}</dd>
                <dt>scope</dt>
                <dd data-testid="exact-scope">{permission.scope}</dd>
              </dl>
            ) : (
              <>
                <p className="mono">{req.requested_action}</p>
                <p className="mono">{req.reason}</p>
                <p>
                  Affected tasks: {req.affected_tasks.join(", ") || "Whole run"}
                </p>
              </>
            )}
            {permission &&
              (canApprove(run, snapshot.action) ? (
                <p className="scope-policy">
                  <Check size={13} />
                  Scope matches this run · saved policy requires approval
                </p>
              ) : (
                <p className="warning-text">
                  The saved policy or adapter does not permit approval of this
                  scope. Resolve prerequisites externally or submit a new
                  trusted plan.
                </p>
              ))}
          </div>
        </header>
        <div className="resolution-context">
          {changed && !accepted && (
            <div className="notice notice-warning" role="alert">
              <div>
                <strong>This action changed since you opened it</strong>
                <p>Review the new version before deciding.</p>
              </div>
              <Button
                variant="secondary"
                size="sm"
                disabled={mutation.isPending}
                onClick={reviewCurrent}
              >
                Review the new version
              </Button>
            </div>
          )}
          {accepted && (
            <div className="notice" role="status">
              Daemon recorded the resolution. Resolving never starts agents.
            </div>
          )}
          <p className="decision-assurance">
            Resolving never starts agents. Execution or continuation requires a
            separate approval.
          </p>
          {permission && (
            <>
              <h3>Why this is needed</h3>
              <p>{req.reason}</p>
            </>
          )}
          {permission?.operation === "run_checks" && (
            <>
              <h3>Commands allowed by this permission</h3>
              <pre className="well">
                {JSON.stringify(run.settings.verification.commands, null, 2)}
              </pre>
            </>
          )}
          <div className="context-columns">
            <div>
              <h3>Recorded evidence</h3>
              <p>{req.evidence}</p>
              <span className="meta">Confidence {req.confidence}%</span>
            </div>
            <div>
              <h3>Affected tasks</h3>
              <p>{req.affected_tasks.join(", ") || "Whole run"}</p>
            </div>
          </div>
          <div className="context-columns">
            <div>
              <h3>If you approve or resolve</h3>
              <p>
                The daemon records this decision.{" "}
                {snapshot.action.continuable
                  ? "Eligible paused tasks still need explicit continuation."
                  : "Execution still requires explicit approval."}
              </p>
            </div>
            <div>
              <h3>If you reject or defer</h3>
              <p>
                This action remains open. A blocking action continues to block
                execution.
              </p>
            </div>
          </div>
          {editable && !accepted && (
            <>
              <div className="field">
                <span>
                  Has it already happened?{" "}
                  {effectRequired && (
                    <span className="warning-text">Required to continue</span>
                  )}
                </span>
                <Segmented
                  label="Inspected effect state"
                  value={effect}
                  disabled={changed || mutation.isPending}
                  onChange={(v) => {
                    setEffect(v);
                    setConfirmed(false);
                  }}
                  options={[
                    { value: "unknown", label: "Not inspected" },
                    { value: "not_performed", label: "Not yet" },
                    { value: "already_performed", label: "Already happened" },
                  ]}
                />
              </div>
              <Field
                label="Decision or nonsecret evidence"
                hint="Required. Never enter credentials."
              >
                <textarea
                  value={note}
                  onChange={(e) => setNote(e.target.value)}
                  rows={3}
                  maxLength={16384}
                  disabled={changed || mutation.isPending}
                />
              </Field>
            </>
          )}
          {mutation.isError && (
            <>
              <ErrorNotice error={mutation.error} />
              {editable && !changed && run.revision !== snapshot.revision && (
                <Button variant="secondary" size="sm" onClick={reviewCurrent}>
                  Review current revision
                </Button>
              )}
            </>
          )}
          <details className="audit">
            <summary>
              Technical details · audit history ({action.audit.length})
            </summary>
            <p>
              Run {run.id} · action {action.id} · inspected revision{" "}
              {snapshot.revision} · updated {date(action.updated_at)}
            </p>
            <pre>{JSON.stringify(snapshot.action.requirement, null, 2)}</pre>
            {action.audit.map((entry, i) => (
              <div key={i}>
                <Badge status={entry.status} />
                <time>{date(entry.at)}</time>
                <pre>
                  {entry.request
                    ? JSON.stringify(entry.request, null, 2)
                    : "Deterministic daemon check"}
                </pre>
              </div>
            ))}
          </details>
        </div>
      </div>
      <footer className="decision-bar">
        {editable && !accepted ? (
          <>
            <div className="decision-main">
              <Segmented
                label="Decision"
                value={disposition}
                disabled={changed || mutation.isPending}
                onChange={(v) => {
                  setDisposition(v);
                  setConfirmed(false);
                }}
                options={[
                  permission
                    ? {
                        value: "approve" as const,
                        label: "Approve exact permission",
                        disabled: !canApprove(run, snapshot.action),
                      }
                    : {
                        value: "resolve" as const,
                        label: "Resolve with evidence",
                      },
                  { value: "reject", label: "Reject" },
                  { value: "defer", label: "Defer" },
                ]}
              />
              <Button
                aria-describedby={needsId}
                disabled={!valid || mutation.isPending}
                onClick={() => mutation.mutate()}
              >
                {mutation.isPending ? "Waiting for daemon…" : label}
              </Button>
            </div>
            <div className="decision-bottom">
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={confirmed}
                  disabled={changed || mutation.isPending}
                  onChange={(e) => setConfirmed(e.target.checked)}
                />
                I inspected the operation, scope and effect state.
              </label>
              <span id={needsId} className="meta">
                {changed
                  ? "Review the changed action"
                  : missing.length
                    ? `Still needed: ${missing.join(", ")}`
                    : "Ready to send · ⌘↵"}
              </span>
            </div>
          </>
        ) : (
          <span className="meta">
            {accepted || !pending(action)
              ? "Decision recorded. Inspect the run before continuing."
              : !canResolve(run)
                ? `${humanize(run.status)} run · the daemon does not accept new resolutions.`
                : "Read-only. Reconnect to a healthy daemon or inspect the current run state."}{" "}
            · {short(run.id)}
          </span>
        )}
      </footer>
    </section>
  );
}
