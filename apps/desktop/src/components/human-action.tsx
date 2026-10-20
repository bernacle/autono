import { useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ShieldCheck, ChevronDown } from "lucide-react";
import type { Run, HumanAction, Resolution } from "../lib/types";
import { canApprove, date, pending } from "../lib/utils";
import { useTransport } from "../lib/transport";
import { Badge, ErrorNotice, Field } from "./common";
import { Button } from "./ui/button";
import { Dialog } from "./ui/dialog";
export function HumanActionCard({
  run,
  action,
  connected,
}: {
  run: Run;
  action: HumanAction;
  connected: boolean;
}) {
  const [snapshot, setSnapshot] = useState<{
    run: Run;
    action: HumanAction;
  } | null>(null);
  const terminal = !["AWAITING_APPROVAL", "RUNNING", "BLOCKED"].includes(
    run.status,
  );
  return (
    <article className="human-card">
      <div className="row-between">
        <div className="row">
          <span className="micro-tag">
            {action.requirement.kind.replace("_", " ")}
          </span>
          <code>{action.id}</code>
        </div>
        <Badge status={action.status} />
      </div>
      <h3>{action.requirement.requested_action}</h3>
      <p>{action.requirement.reason}</p>
      <div className="row-between">
        <span className="muted">
          {action.requirement.blocking ? "Blocking" : "Advisory"} ·{" "}
          {action.requirement.affected_tasks.join(", ") || "Whole run"}
          {action.continuable ? " · live pause" : ""}
        </span>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => setSnapshot({ run, action })}
        >
          Inspect {pending(action) ? "& resolve" : "resolution"}
        </Button>
      </div>
      {snapshot && (
        <ResolutionDialog
          run={snapshot.run}
          action={snapshot.action}
          editable={connected && !terminal && pending(action)}
          onClose={() => setSnapshot(null)}
        />
      )}
    </article>
  );
}
export function ResolutionDialog({
  run,
  action,
  editable,
  onClose,
}: {
  run: Run;
  action: HumanAction;
  editable: boolean;
  onClose: () => void;
}) {
  const permission = action.requirement.permission;
  const [disposition, setDisposition] = useState<Resolution["disposition"]>(
    permission ? "approve" : "resolve",
  );
  const [note, setNote] = useState(""),
    [effect, setEffect] = useState<Resolution["effect"]>("unknown"),
    [confirmed, setConfirmed] = useState(false);
  const transport = useTransport(),
    client = useQueryClient(),
    receipt = useRef<{ signature: string; id: string } | null>(null);
  const mutation = useMutation({
    mutationFn: () => {
      const payload = {
        disposition,
        note,
        effect,
        permission: disposition === "approve" ? permission : null,
      };
      const signature = JSON.stringify(payload);
      if (receipt.current?.signature !== signature)
        receipt.current = { signature, id: crypto.randomUUID() };
      return transport.request({
        type: "resolve",
        id: run.id,
        revision: run.revision,
        action_id: action.id,
        resolution: { ...payload, request_id: receipt.current.id },
      });
    },
    retry: false,
    onSuccess: async () => {
      await client.invalidateQueries({
        predicate: (q) => !["events", "logs"].includes(String(q.queryKey[0])),
      });
      onClose();
    },
    onError: () => {
      void client.invalidateQueries({ queryKey: ["run", run.id] });
    },
  });
  const allowed = disposition !== "approve" || canApprove(run, action);
  const valid =
    editable &&
    allowed &&
    confirmed &&
    note.trim().length > 0 &&
    (!(action.continuable && ["approve", "resolve"].includes(disposition)) ||
      effect !== "unknown");
  return (
    <Dialog
      wide
      title="Human action"
      description={`${action.id} · ${action.requirement.kind} · revision ${run.revision}`}
      onClose={onClose}
      busy={mutation.isPending}
    >
      <h3>{action.requirement.requested_action}</h3>
      <p>{action.requirement.reason}</p>
      <div className="confirmation-scope">
        <strong>
          Recorded evidence · confidence {action.requirement.confidence}%
        </strong>
        <p>{action.requirement.evidence}</p>
        <p>
          Affected tasks:{" "}
          {action.requirement.affected_tasks.join(", ") || "whole run"}
        </p>
      </div>
      {permission && (
        <div className="permission-object">
          <div className="row">
            <ShieldCheck size={17} />
            <strong>Authoritative permission object</strong>
          </div>
          <pre>{JSON.stringify(permission, null, 2)}</pre>
          <p>Approval submits this exact operation and scope to the daemon.</p>
          {!canApprove(run, action) && (
            <p className="warning-text">
              The saved policy or adapter does not permit approval of this
              scope. Resolve prerequisites externally or submit a new trusted
              plan.
            </p>
          )}
        </div>
      )}
      {editable && (
        <>
          <div className="form-grid">
            <Field label="Disposition">
              <select
                value={disposition}
                onChange={(e) => {
                  setDisposition(e.target.value as Resolution["disposition"]);
                  setConfirmed(false);
                }}
              >
                {permission ? (
                  <option value="approve" disabled={!canApprove(run, action)}>
                    Approve exact permission
                  </option>
                ) : (
                  <option value="resolve">Resolve with evidence</option>
                )}
                <option value="reject">Reject</option>
                <option value="defer">Defer</option>
              </select>
            </Field>
            <Field label="Inspected effect state">
              <select
                value={effect}
                onChange={(e) =>
                  setEffect(e.target.value as Resolution["effect"])
                }
              >
                <option value="unknown">Unknown / not yet inspected</option>
                <option value="not_performed">
                  Operation has not happened
                </option>
                <option value="already_performed">
                  Operation already happened
                </option>
              </select>
            </Field>
          </div>
          <Field
            label="Decision or nonsecret evidence"
            hint="Never enter credentials. A resolution alone does not start agents."
          >
            <textarea
              value={note}
              onChange={(e) => setNote(e.target.value)}
              rows={3}
              maxLength={16384}
            />
          </Field>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={confirmed}
              onChange={(e) => setConfirmed(e.target.checked)}
            />
            I inspected the operation, scope and effect state above.
          </label>
        </>
      )}
      {mutation.isError && <ErrorNotice error={mutation.error} />}
      <details className="audit">
        <summary>
          <ChevronDown size={14} />
          Audit history · {action.audit.length} entries
        </summary>
        {action.audit.map((entry, index) => (
          <div key={index}>
            <Badge status={entry.status} />
            <small>{date(entry.at)}</small>
            <pre>
              {entry.request
                ? JSON.stringify(entry.request, null, 2)
                : "Deterministic daemon check"}
            </pre>
          </div>
        ))}
      </details>
      <div className="dialog-actions">
        <Button
          variant="secondary"
          disabled={mutation.isPending}
          onClick={onClose}
        >
          Close
        </Button>
        {editable && (
          <Button
            disabled={!valid || mutation.isPending}
            onClick={() => mutation.mutate()}
          >
            {mutation.isPending ? "Waiting for daemon…" : "Confirm resolution"}
          </Button>
        )}
      </div>
    </Dialog>
  );
}
