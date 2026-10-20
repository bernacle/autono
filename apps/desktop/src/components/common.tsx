import {
  AlertTriangle,
  Check,
  Circle,
  CirclePause,
  LoaderCircle,
  XCircle,
} from "lucide-react";
import type { ReactNode } from "react";
import { errorMessage } from "../lib/utils";
import { Button } from "./ui/button";
export function Badge({ status }: { status: string }) {
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
      : [
            "BLOCKED",
            "BLOCKED_HUMAN",
            "AWAITING_APPROVAL",
            "PENDING",
            "DEFERRED",
          ].includes(status)
        ? "warning"
        : ["RUNNING", "PLANNING", "INTEGRATING"].includes(status)
          ? "active"
          : "neutral";
  const Icon =
    kind === "success"
      ? Check
      : kind === "danger"
        ? XCircle
        : status === "INTERRUPTED"
          ? CirclePause
          : kind === "active"
            ? LoaderCircle
            : Circle;
  return (
    <span className={`badge badge-${kind}`}>
      <Icon size={12} className={kind === "active" ? "spin" : ""} />
      {status.replaceAll("_", " ").toLowerCase()}
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
