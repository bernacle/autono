import { CheckCircle2, CircleHelp, XCircle } from "lucide-react";
import type { Preflight } from "../lib/types";
import { date } from "../lib/utils";
import { Empty } from "./common";
export function PreflightChecklist({ report }: { report: Preflight | null }) {
  if (!report)
    return (
      <Empty title="Preflight has not run">
        The daemon generates readiness checks after planning. Refresh preflight
        when the plan is ready.
      </Empty>
    );
  return (
    <>
      <div className={`preflight-summary ${report.ready ? "ready" : ""}`}>
        <strong>
          {report.ready
            ? "Ready for explicit approval"
            : "Prerequisites need attention"}
        </strong>
        <span>Checked {date(report.checked_at)}</span>
      </div>
      <div className="checklist">
        {report.checks.map((check) => {
          const Icon =
            check.ready === true
              ? CheckCircle2
              : check.ready === false
                ? XCircle
                : CircleHelp;
          return (
            <div className="check" key={check.id}>
              <Icon
                size={17}
                className={
                  check.ready === true
                    ? "success-text"
                    : check.ready === false
                      ? "danger-text"
                      : "muted"
                }
              />
              <div>
                <div className="row">
                  <strong>{check.id}</strong>
                  <span className="micro-tag">
                    {check.blocking ? "required" : "informational"}
                  </span>
                </div>
                <p>{check.evidence}</p>
              </div>
              <span className="muted">
                {check.ready === null
                  ? "Unknown"
                  : check.ready
                    ? "Ready"
                    : check.id.startsWith("permission-")
                      ? "Policy gate"
                      : "Missing"}
              </span>
            </div>
          );
        })}
      </div>
      <p className="footnote">{report.limitation}</p>
    </>
  );
}
