import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Run, RunAction } from "../lib/types";
import { useTransport } from "../lib/transport";
import { title } from "../lib/utils";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { ErrorNotice } from "./common";
const labels: Record<RunAction, string> = {
  execute: "Approve execution",
  continue: "Continue resolved tasks",
  cancel: "Cancel run",
  integrate: "Integrate verified run",
  preflight: "Refresh preflight",
};
export function ConfirmAction({
  run,
  action,
  onClose,
  onAccepted,
  connected = true,
}: {
  run: Run;
  connected?: boolean;
  action: RunAction;
  onClose: () => void;
  onAccepted: (message: string) => void;
}) {
  const transport = useTransport(),
    client = useQueryClient();
  const mutation = useMutation({
    mutationFn: () =>
      transport.request({
        type: "action",
        id: run.id,
        revision: run.revision,
        action,
      }),
    retry: false,
    onSuccess: async () => {
      await client.invalidateQueries({
        predicate: (q) => !["events", "logs"].includes(String(q.queryKey[0])),
      });
      onAccepted(
        "Daemon accepted the request. Follow the live state for its outcome.",
      );
      onClose();
    },
    onError: () => {
      void client.invalidateQueries({ queryKey: ["run", run.id] });
    },
  });
  return (
    <Dialog
      title={labels[action]}
      description={`Review ${title(run)} · revision ${run.revision}`}
      onClose={onClose}
      busy={mutation.isPending}
    >
      <div className="confirmation-scope">
        <code>{run.id}</code>
        <dl>
          <dt>Repository</dt>
          <dd>{run.repository}</dd>
          <dt>Pinned base</dt>
          <dd>{run.base_commit}</dd>
          <dt>Current state</dt>
          <dd>{run.status}</dd>
        </dl>
      </div>
      {action === "execute" && (
        <>
          <p>
            This authorizes daemon-owned coding agents, configured checks, and
            independent reviews in assigned local worktrees.
          </p>
          <div className="confirmation-scope">
            <strong>Configured roles</strong>
            {Object.entries(run.settings.agents).map(([role, config]) => (
              <p key={role}>
                {role}: {config.provider} · {config.model ?? "provider default"}
              </p>
            ))}
            <strong>Exact verification argument arrays</strong>
            <pre>
              {JSON.stringify(run.settings.verification.commands, null, 2)}
            </pre>
            <p>
              {run.settings.execution.max_parallel} parallel tasks ·{" "}
              {run.settings.execution.max_attempts} coding attempts ·{" "}
              {run.settings.execution.max_review_cycles} review cycles
            </p>
          </div>
        </>
      )}
      {action === "integrate" && (
        <p>
          The daemon will combine verified task commits in its local integration
          worktree and run the configured checks. It does not merge into main or
          push.
        </p>
      )}
      {action === "cancel" && (
        <p>
          The daemon will signal active work to stop. Completed evidence remains
          available. This cannot be undone by continuing the run.
        </p>
      )}
      {action === "continue" && (
        <p>
          Start fresh provider invocations for eligible resolved live pauses.
          Saved workspaces, completed evidence, and remaining attempt limits
          stay authoritative.
        </p>
      )}
      {mutation.isError && <ErrorNotice error={mutation.error} />}
      <div className="dialog-actions">
        <Button
          variant="secondary"
          disabled={mutation.isPending}
          onClick={onClose}
        >
          Back
        </Button>
        <Button
          variant={action === "cancel" ? "destructive" : "default"}
          disabled={!connected || mutation.isPending}
          onClick={() => mutation.mutate()}
        >
          {mutation.isPending ? "Waiting for daemon…" : labels[action]}
        </Button>
      </div>
    </Dialog>
  );
}
