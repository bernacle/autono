import type { DecisionRecord } from "../lib/types";

import { decisionVerified } from "../lib/decision-state";

export function Decisions({ ledger = [] }: { ledger?: DecisionRecord[] }) {
  if (!ledger.length) return null;
  return (
    <section aria-label="Decisions Autono made while you were away">
      <h2>Decisions Autono made while you were away</h2>
      {ledger.map((d) => {
        const superseded = d.audit.some((e) => e.kind === "superseded");
        const accepted = d.audit[0]?.kind === "accepted";
        const verified = decisionVerified(d);
        return (
          <details key={d.id} className="audit">
            <summary>
              {d.proposal.question} ·{" "}
              {superseded
                ? "Superseded"
                : verified
                  ? "Independently verified"
                  : accepted
                    ? "Verification incomplete"
                    : "Human action required"}
            </summary>
            <p>
              <strong>{d.proposal.chosen}</strong> — {d.proposal.rationale}
            </p>
            <p>Alternatives: {d.proposal.alternatives.join("; ")}</p>
            <p>Authority: {d.policy_basis}</p>
            <p>Affected tasks: {d.proposal.affected_tasks.join(", ")}</p>
            {d.evidence.map((e, i) => (
              <p key={i}>
                Evidence: {e.file ?? "Original specification"} ·{" "}
                {e.source === "frozen_base"
                  ? `Frozen base ${e.base_commit}`
                  : e.source === "specification"
                    ? "Developer specification"
                    : "Historical origin unverified"}{" "}
                · <code>{e.content_sha256}</code>
                <br />
                {e.excerpt}
              </p>
            ))}
            {!!d.proposal.assumptions.length && (
              <p>Assumptions: {d.proposal.assumptions.join("; ")}</p>
            )}
            {!!d.proposal.uncertainty.length && (
              <p>Uncertainty: {d.proposal.uncertainty.join("; ")}</p>
            )}
            <ol>
              {d.audit.map((e) => (
                <li key={e.sequence}>
                  {e.at} · {e.kind} · {e.task_id}{" "}
                  {e.attempt && `attempt ${e.attempt}`} · {e.detail}
                </li>
              ))}
            </ol>
            <code>{d.id}</code>
          </details>
        );
      })}
    </section>
  );
}
