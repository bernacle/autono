import { AlertTriangle, Circle } from "lucide-react";
import type { ReactNode } from "react";
import { errorMessage } from "../lib/utils";
import { Button } from "./ui/button";
export function StatusGlyph({ status }: { status: string }) {
  const kind = [
    "COMPLETED",
    "SUCCEEDED",
    "VERIFIED",
    "RESOLVED",
    "passed",
    "review accepted",
  ].includes(status)
    ? "success"
    : ["FAILED", "REJECTED", "test_failure", "infrastructure_failure"].includes(
          status,
        )
      ? "danger"
      : ["BLOCKED_HUMAN", "AWAITING_APPROVAL", "PENDING"].includes(status)
        ? "warning"
        : ["RUNNING", "PLANNING", "INTEGRATING"].includes(status)
          ? "active"
          : "neutral";
  return (
    <svg
      className={`status-glyph glyph-${kind}`}
      width="14"
      height="14"
      viewBox="0 0 14 14"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      aria-hidden="true"
    >
      <circle
        cx="7"
        cy="7"
        r="5"
        strokeDasharray={status === "WAITING" ? "2 2" : undefined}
        opacity={kind === "active" ? ".3" : 1}
      />
      {kind === "active" && <path className="spin" d="M7 2a5 5 0 0 1 5 5" />}
      {kind === "success" && <path d="m4.3 7 1.8 1.8L9.8 5" />}
      {kind === "danger" && <path d="m5 5 4 4m0-4L5 9" />}
      {(status === "BLOCKED_HUMAN" || status === "PENDING") && (
        <path d="M5.5 5v4m3-4v4" />
      )}
      {status === "AWAITING_APPROVAL" && (
        <circle cx="7" cy="7" r="1" fill="currentColor" stroke="none" />
      )}
      {status === "BLOCKED" && <path d="M7 4.5v3M7 9v.2" />}
      {status === "INTERRUPTED" && <path d="M4.5 7h5" />}
      {status === "CANCELLED" && <path d="m4.5 9.5 5-5" />}
    </svg>
  );
}
export function Badge({ status, label }: { status: string; label?: string }) {
  const live = ["RUNNING", "PLANNING", "INTEGRATING"].includes(status);
  const attention = ["BLOCKED_HUMAN", "AWAITING_APPROVAL", "PENDING"].includes(
    status,
  );
  const failed = [
    "FAILED",
    "REJECTED",
    "test_failure",
    "infrastructure_failure",
  ].includes(status);
  const text =
    label ??
    (status === "BLOCKED_HUMAN"
      ? "Needs you"
      : status
          .replaceAll("_", " ")
          .toLowerCase()
          .replace(/^./, (c) => c.toUpperCase()));
  return (
    <span
      className={`badge ${live ? "badge-active" : attention ? "badge-warning" : failed ? "badge-danger" : "badge-neutral"}`}
    >
      <StatusGlyph status={status} />
      {text}
    </span>
  );
}
export function ErrorNotice({
  error,
  retry,
}: {
  error: unknown;
  retry?: () => void;
}) {
  return (
    <div role="alert" className="notice notice-error">
      <AlertTriangle size={17} />
      <div>
        <strong>Request could not be confirmed</strong>
        <p>{errorMessage(error)}</p>
      </div>
      {retry && (
        <Button variant="secondary" size="sm" onClick={retry}>
          Retry
        </Button>
      )}
    </div>
  );
}
export function Empty({
  title,
  children,
}: {
  title: string;
  children?: ReactNode;
}) {
  return (
    <div className="empty">
      <Circle size={22} />
      <strong>{title}</strong>
      <p>{children}</p>
    </div>
  );
}
export function Panel({
  title,
  meta,
  children,
  className = "",
}: {
  title: string;
  meta?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={`panel ${className}`}>
      <div className="panel-heading">
        <h2>{title}</h2>
        {meta}
      </div>
      {children}
    </section>
  );
}
export function Field({
  label,
  children,
  hint,
}: {
  label: string;
  children: ReactNode;
  hint?: string;
}) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
      {hint && <small>{hint}</small>}
    </label>
  );
}
