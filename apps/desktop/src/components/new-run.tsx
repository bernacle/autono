import { useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { FolderOpen, ArrowRight } from "lucide-react";
import { useTransport } from "../lib/transport";
import type { Run } from "../lib/types";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { ErrorNotice, Field } from "./common";
export function NewRun({
  onClose,
  onCreated,
  connected,
}: {
  onClose: () => void;
  onCreated: (id: string) => void;
  connected: boolean;
}) {
  const [repository, setRepository] = useState(""),
    [base, setBase] = useState("HEAD"),
    [specification, setSpecification] = useState(""),
    [pickerError, setPickerError] = useState<unknown>(null),
    [ack, setAck] = useState(false);
  const transport = useTransport(),
    client = useQueryClient(),
    receipt = useRef<{ signature: string; id: string } | null>(null);
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
      onCreated(run.id);
      onClose();
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
  return (
    <Dialog
      wide
      title="New engineering run"
      description="Choose a local repository, then ask the daemon to prepare a plan."
      onClose={onClose}
      busy={mutation.isPending}
    >
      <div className="steps">
        <span className="current">1 · Specification</span>
        <ArrowRight size={14} />
        <span>2 · Plan & preflight</span>
        <ArrowRight size={14} />
        <span>3 · Approve execution</span>
      </div>
      <Field label="Existing local repository">
        <div className="input-with-button">
          <input
            value={repository}
            placeholder="Select a Git repository…"
            readOnly
          />
          <Button
            variant="secondary"
            onClick={() => void pick()}
            disabled={mutation.isPending}
          >
            <FolderOpen size={15} />
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
        hint={`${new TextEncoder().encode(specification).length.toLocaleString()} / 131,072 bytes · nonsecret requirements only`}
      >
        <textarea
          value={specification}
          onChange={(e) => setSpecification(e.target.value)}
          rows={10}
          placeholder="# Feature name\n\nDescribe behavior, constraints and acceptance criteria."
        />
      </Field>
      <label className="checkbox-label">
        <input
          type="checkbox"
          checked={ack}
          onChange={(e) => setAck(e.target.checked)}
        />
        Start the configured planner. This may use paid inference.
      </label>
      <p className="footnote">
        Coding begins only after you inspect the plan, resolve preflight, and
        explicitly approve execution.
      </p>
      {mutation.isError && <ErrorNotice error={mutation.error} />}
      <div className="dialog-actions">
        <Button
          variant="secondary"
          onClick={onClose}
          disabled={mutation.isPending}
        >
          Back
        </Button>
        <Button
          onClick={() => mutation.mutate()}
          disabled={
            !connected ||
            !repository ||
            !base.trim() ||
            !specification.trim() ||
            new TextEncoder().encode(specification).length > 131072 ||
            !ack ||
            mutation.isPending
          }
        >
          {mutation.isPending ? "Submitting to daemon…" : "Submit planning"}
          <ArrowRight size={15} />
        </Button>
      </div>
    </Dialog>
  );
}
