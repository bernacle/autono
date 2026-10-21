import { useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderOpen, ArrowRight, FileText, LockKeyhole } from "lucide-react";
import { useTransport } from "../lib/transport";
import type { LocalSettings, Run } from "../lib/types";
import { Button } from "./ui/button";
import { ErrorNotice, Field } from "./common";
export function RunGuide({ readiness = false }: { readiness?: boolean }) {
  return (
    <aside className="run-guide" aria-label="New run steps">
      <div className="guide-group">
        <span>Planning · may use paid inference</span>
        <ol start={1}>
          <li aria-current={!readiness ? "step" : undefined}>Specification</li>
          <li>Planning</li>
        </ol>
      </div>
      <div className="guide-group">
        <span>Readiness · no code runs</span>
        <ol start={3}>
          <li aria-current={readiness ? "step" : undefined}>Review plan</li>
          <li>Preflight</li>
          <li>Resolve</li>
        </ol>
      </div>
      <div className="guide-group">
        <span>Execution · explicit approval</span>
        <ol start={6}>
          <li>Approve</li>
        </ol>
      </div>
    </aside>
  );
}
export function NewRun({
  onClose,
  onCreated,
  connected,
  activeRun,
}: {
  onClose: () => void;
  onCreated: (id: string) => void;
  connected: boolean;
  activeRun?: boolean;
}) {
  const [repository, setRepository] = useState(""),
    [base, setBase] = useState("HEAD"),
    [specification, setSpecification] = useState(""),
    [pickerError, setPickerError] = useState<unknown>(null),
    [ack, setAck] = useState(false);
  const transport = useTransport(),
    client = useQueryClient(),
    receipt = useRef<{ signature: string; id: string } | null>(null);
  const settings = useQuery({
    queryKey: ["settings"],
    queryFn: () => transport.request<LocalSettings>({ type: "settings" }),
    retry: false,
  });
  const planner = settings.data?.agents?.planner;
  const mutation = useMutation({
    mutationFn: () => {
      const payload = { repository, base, specification },
        signature = JSON.stringify(payload);
      if (receipt.current?.signature !== signature)
        receipt.current = { signature, id: crypto.randomUUID() };
      return transport.request<Run>({
        type: "plan",
        submission: { ...payload, id: receipt.current.id },
      });
    },
    retry: false,
    onSuccess: async (run) => {
      await client.invalidateQueries({ queryKey: ["runs"] });
      onClose();
      onCreated(run.id);
    },
  });
  const pick = async () => {
    try {
      const path = await transport.pickRepository();
      if (path) setRepository(path);
      setPickerError(null);
    } catch (error) {
      setPickerError(error);
    }
  };
  const bytes = new TextEncoder().encode(specification).length;
  const missing = !connected
    ? "Reconnect to a healthy daemon"
    : !repository
      ? "Select a repository"
      : !base.trim()
        ? "Enter a base revision"
        : !specification.trim()
          ? "Add a specification"
          : bytes > 131072
            ? "Specification exceeds 131,072 bytes"
            : !ack
              ? "Acknowledge planner inference"
              : "";
  return (
    <div
      className="new-run-page"
      onKeyDown={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
          e.preventDefault();
          if (!missing && !mutation.isPending) mutation.mutate();
        }
      }}
    >
      <RunGuide />
      <div className="new-run-form">
        <h1>Describe the feature</h1>
        <p className="muted">
          The daemon prepares a plan. You review it before any coding starts.
        </p>
        <Field label="Existing local repository">
          <div className="repository-picker">
            <FolderOpen size={20} />
            <div>
              <input
                value={repository}
                placeholder="Select a Git repository…"
                readOnly
              />
              <small className="meta">
                An existing local checkout. Autono creates its own worktrees.
              </small>
            </div>
            <Button
              variant="secondary"
              aria-label="Browse…"
              onClick={() => void pick()}
              disabled={mutation.isPending}
            >
              Browse…
            </Button>
          </div>
        </Field>
        {!!pickerError && <ErrorNotice error={pickerError} />}
        <Field
          label="Base branch or commit"
          hint="The daemon resolves and pins this revision before planning."
        >
          <input
            value={base}
            onChange={(e) => setBase(e.target.value)}
            maxLength={512}
          />
        </Field>
        <Field
          label="Feature specification"
          hint={`${bytes.toLocaleString()} / 131,072 bytes · nonsecret requirements only`}
        >
          <textarea
            className="spec-editor"
            value={specification}
            onChange={(e) => setSpecification(e.target.value)}
            rows={12}
            placeholder={
              "# Feature name\n\nDescribe behavior, constraints and acceptance criteria."
            }
          />
        </Field>
        {activeRun && (
          <p className="notice notice-warning">
            The daemon accepts one active engineering run. Its admission
            response remains authoritative.
          </p>
        )}
        {mutation.isError && <ErrorNotice error={mutation.error} />}
        <div className="planning-footer">
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={ack}
              onChange={(e) => setAck(e.target.checked)}
            />
            Start the configured planner. This may use paid inference.
          </label>
          <div className="form-submit">
            <Button
              variant="secondary"
              onClick={onClose}
              disabled={mutation.isPending}
            >
              Back
            </Button>
            <span id="planning-reason" className="meta">
              {missing || "Ready to submit · ⌘↵"}
            </span>
            <Button
              aria-describedby="planning-reason"
              disabled={!!missing || mutation.isPending}
              onClick={() => mutation.mutate()}
            >
              {mutation.isPending ? "Submitting to daemon…" : "Start planning"}
              <ArrowRight size={14} />
            </Button>
          </div>
        </div>
      </div>
      <aside className="planner-rail">
        <div className="well">
          <h3>
            <FileText size={15} />
            Planner
          </h3>
          <strong>{planner?.provider ?? "Configured by daemon"}</strong>
          <p>{planner?.model ?? "Provider default model"}</p>
          {planner && (
            <span className="meta">{planner.timeout_seconds}s timeout</span>
          )}
          {settings.isError && (
            <ErrorNotice
              error={settings.error}
              retry={() => void settings.refetch()}
            />
          )}
          <p className="meta">
            User defaults shown. The daemon pins effective project settings on
            the run.
          </p>
        </div>
        <p className="meta">
          <LockKeyhole size={14} />
          Coding begins only after you inspect the plan, resolve preflight and
          explicitly approve execution.
        </p>
      </aside>
    </div>
  );
}
