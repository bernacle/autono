import { useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowRight, FolderOpen, ShieldCheck } from "lucide-react";
import { useTransport } from "../lib/transport";
import type { DelegationContract, DelegationProposal, Run } from "../lib/types";
import { title } from "../lib/utils";
import { Button } from "./ui/button";
import { Dialog } from "./ui/dialog";
import { ErrorNotice, Field } from "./common";

export function BuildFeature({
  connected,
  active,
  recent,
  onCreated,
  onSupervised,
}: {
  connected: boolean;
  active?: Run;
  recent?: Run;
  onCreated: (id: string) => void;
  onSupervised: () => void;
}) {
  const transport = useTransport(),
    client = useQueryClient();
  const [repository, setRepository] = useState(
    () => localStorage.getItem("autono.build.repository") ?? "",
  );
  const [specification, setSpecification] = useState("");
  const [base, setBase] = useState("HEAD"),
    [trees, setTrees] = useState("src, tests");
  const [seconds, setSeconds] = useState(1800),
    [integration, setIntegration] = useState(true);
  const [contract, setContract] = useState<DelegationContract | null>(null),
    [pickerError, setPickerError] = useState<unknown>(null);
  const receipt = useRef<{ signature: string; id: string } | null>(null);
  const proposal = useMutation({
    mutationFn: () => {
      const roots = trees
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      const payload = {
          repository,
          base,
          specification,
          roots,
          seconds,
          integration,
        },
        signature = JSON.stringify(payload);
      if (!receipt.current) {
        try {
          const pending = JSON.parse(
            localStorage.getItem("autono.build.pending") ?? "null",
          ) as { signature?: unknown; id?: unknown } | null;
          if (
            pending?.signature === signature &&
            typeof pending.id === "string"
          )
            receipt.current = { signature, id: pending.id };
        } catch {
          /* Invalid local cache never becomes authority. */
        }
      }
      if (receipt.current?.signature !== signature)
        receipt.current = { signature, id: crypto.randomUUID() };
      localStorage.setItem(
        "autono.build.pending",
        JSON.stringify(receipt.current),
      );
      const request: DelegationProposal = {
        submission: { id: receipt.current.id, repository, base, specification },
        source_scope: {
          version: 1,
          read_subtrees: roots,
          write_subtrees: roots,
          allow_create: true,
        },
        writable_files: [],
        max_runtime_seconds: seconds,
        local_integration: integration,
      };
      return transport.request<DelegationContract>({
        type: "propose_delegation",
        request,
      });
    },
    retry: false,
    onSuccess: setContract,
  });
  const authorize = useMutation({
    mutationFn: () =>
      transport.request<Run>({
        type: "authorize_delegation",
        id: contract!.request.submission.id,
        displayed_contract: contract!,
      }),
    retry: false,
    onSuccess: async (run) => {
      localStorage.setItem("autono.build.repository", run.repository);
      localStorage.removeItem("autono.build.pending");
      await client.invalidateQueries({ queryKey: ["runs"] });
      setContract(null);
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
  const missing = !connected
    ? "Reconnect to the daemon to build"
    : active
      ? "Another feature is running"
      : !repository
        ? "Select a repository"
        : !specification.trim()
          ? "Describe your feature"
          : new TextEncoder().encode(specification).length > 131072
            ? "Feature request exceeds 128 KiB"
            : !base.trim() || !trees.trim() || seconds < 1 || seconds > 86400
              ? "Check advanced settings"
              : "";
  return (
    <div className="build-page">
      <div className="build-intro">
        <span className="eyebrow">
          <ShieldCheck size={14} /> Bounded autonomy
        </span>
        <h1>What do you want to build?</h1>
        <p className="muted">
          Describe a feature. Approve its scope once. Return and review.
        </p>
      </div>
      {active && (
        <div className="notice" role="status">
          <strong>{title(active)}</strong>
          <p>{active.summary}</p>
          <Button variant="secondary" onClick={() => onCreated(active.id)}>
            View current work <ArrowRight size={14} />
          </Button>
        </div>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!missing && !proposal.isPending) proposal.mutate();
        }}
      >
        <Field label="Repository">
          <div className="repository-picker">
            <FolderOpen size={20} />
            <input
              aria-label="Selected repository"
              value={repository}
              placeholder="Select a local Git repository…"
              readOnly
            />
            <Button
              type="button"
              variant="secondary"
              onClick={() => void pick()}
              disabled={proposal.isPending}
            >
              Change repository
            </Button>
          </div>
        </Field>
        {!!pickerError && <ErrorNotice error={pickerError} />}
        <Field label="Feature request">
          <textarea
            className="spec-editor build-prompt"
            value={specification}
            onChange={(e) => setSpecification(e.target.value)}
            rows={8}
            placeholder="Add CSV export to the reporting module, including validation and tests."
          />
        </Field>
        <details className="build-advanced">
          <summary>Advanced settings</summary>
          <div className="build-settings">
            <Field label="Base branch or commit">
              <input value={base} onChange={(e) => setBase(e.target.value)} />
            </Field>
            <Field
              label="Source subtrees"
              hint="Comma-separated read/write subtrees. Protected files remain excluded."
            >
              <input value={trees} onChange={(e) => setTrees(e.target.value)} />
            </Field>
            <Field label="Maximum runtime in seconds">
              <input
                type="number"
                min={1}
                max={86400}
                value={seconds}
                onChange={(e) => setSeconds(Number(e.target.value))}
              />
            </Field>
            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={integration}
                onChange={(e) => setIntegration(e.target.checked)}
              />{" "}
              Integrate verified work into an isolated local branch
            </label>
          </div>
        </details>
        <div className="build-safety">
          <ShieldCheck size={18} />
          <p>
            Source changes in <strong>{trees || "selected subtrees"}</strong>,
            checks and independent review. Protected files excluded. No push,
            deployment or merge to main. Claude API only; small source scopes.
            Inference may be billed; spending caps unavailable.
          </p>
        </div>
        <p className="meta">
          Build feature first performs trusted, read-only local inspection. No
          inference or repository scripts run before consent.
        </p>
        {proposal.isError && <ErrorNotice error={proposal.error} />}
        <div className="form-submit">
          <span className="meta" id="build-reason">
            {missing || "One confirmation before work starts"}
          </span>
          <Button
            type="submit"
            disabled={!!missing || proposal.isPending}
            aria-describedby="build-reason"
          >
            {proposal.isPending ? "Inspecting repository…" : "Build feature"}
            <ArrowRight size={14} />
          </Button>
        </div>
      </form>
      {recent && !active && (
        <button className="build-recent" onClick={() => onCreated(recent.id)}>
          <span>Latest delegated feature</span>
          <strong>{title(recent)}</strong>
          <span>{recent.handoff?.summary ?? recent.summary}</span>
          <ArrowRight size={16} />
        </button>
      )}
      <p className="meta">
        Prefer explicit planning and execution controls?{" "}
        <Button variant="ghost" onClick={onSupervised}>
          Supervised mode
        </Button>
      </p>
      {contract && (
        <Dialog
          title="Authorize this feature"
          description="One bounded grant covers planning, implementation, checks, independent review and the local integration shown below."
          wide
          busy={authorize.isPending}
          onClose={() => {
            if (!authorize.isPending) {
              setContract(null);
              authorize.reset();
            }
          }}
        >
          <div className="build-consent">
            <p>{contract.request.submission.specification}</p>
            <dl className="build-contract">
              <dt>Repository</dt>
              <dd>{contract.request.submission.repository}</dd>
              <dt>Pinned base</dt>
              <dd>
                <code>{contract.base_commit}</code>
              </dd>
              <dt>Read source</dt>
              <dd>
                {contract.request.source_scope?.read_subtrees.join(", ")}; fixed
                pinned project context
              </dd>
              <dt>Write source</dt>
              <dd>
                {contract.request.source_scope?.write_subtrees.join(", ")};
                create/replace ordinary source only
              </dd>
              <dt>Owned resources</dt>
              <dd>{contract.worktree_root}</dd>
              <dt>Agents</dt>
              <dd>
                {Object.entries(contract.settings.agents).map(
                  ([role, agent]) => (
                    <div key={role}>
                      {role}: {agent.provider} ·{" "}
                      {agent.model ?? "provider default"} ·{" "}
                      {agent.timeout_seconds}s
                    </div>
                  ),
                )}
              </dd>
              <dt>Checks</dt>
              <dd>
                {contract.settings.verification.commands.map((argv, i) => (
                  <code key={i}>
                    {JSON.stringify(argv)}
                    <br />
                  </code>
                ))}
                {contract.verification_sandbox};{" "}
                {contract.settings.verification.timeout_seconds}s each
              </dd>
              <dt>Limits</dt>
              <dd>
                {contract.request.max_runtime_seconds}s total;{" "}
                {contract.settings.execution.max_parallel} concurrent;{" "}
                {contract.settings.execution.max_attempts} attempts/task;{" "}
                {contract.settings.execution.max_review_cycles} reviews/task. No
                spending cap.
              </dd>
              <dt>Local integration</dt>
              <dd>
                {contract.request.local_integration
                  ? "Authorized into an isolated Autono branch"
                  : "Not authorized; stop at verified task branches"}
              </dd>
            </dl>
            <details>
              <summary>
                Source discovery · {contract.source_inventory?.length ?? 0}{" "}
                files at pinned base
              </summary>
              <ul>
                {contract.source_inventory?.map((path) => (
                  <li key={path}>
                    <code>{path}</code>
                  </li>
                ))}
              </ul>
            </details>
            <details>
              <summary>
                Exact capabilities, exclusions and full contract
              </summary>
              {contract.limitations.map((s, i) => (
                <p key={i}>{s}</p>
              ))}
              <pre>{JSON.stringify(contract, null, 2)}</pre>
            </details>
            <p className="notice">
              Excluded: credentials, protected configuration, arbitrary tools or
              commands, dependency installation, remote mutation, publishing,
              deployment and merge to main. No automatic crash replay.
            </p>
            {authorize.isError && <ErrorNotice error={authorize.error} />}
            <div className="form-submit">
              <Button
                variant="secondary"
                disabled={authorize.isPending}
                onClick={() => {
                  setContract(null);
                  authorize.reset();
                }}
              >
                Back
              </Button>
              <Button
                disabled={!connected || authorize.isPending}
                onClick={() => authorize.mutate()}
              >
                {authorize.isPending
                  ? "Recording consent…"
                  : "Authorize and build"}
              </Button>
            </div>
          </div>
        </Dialog>
      )}
    </div>
  );
}
