import type { DecisionRecord } from "./types";

// Same conservative predicate as DecisionRecord::verified in the protocol.
export function decisionVerified(d: DecisionRecord): boolean {
  return (
    d.audit[0]?.kind === "accepted" &&
    !d.audit.some((e) => e.kind === "superseded") &&
    d.proposal.affected_tasks.every((task) => {
      const last = d.audit.filter((e) => e.task_id === task).at(-1);
      if (last?.kind !== "task_verified" || last.attempt == null) return false;
      let sequence = 0;
      return [
        "consumed",
        "implemented",
        "checks_passed",
        "review_accepted",
        "task_verified",
      ].every((kind) => {
        const event = d.audit.find(
          (e) =>
            e.kind === kind &&
            e.task_id === task &&
            e.attempt === last.attempt &&
            e.sequence > sequence &&
            e.sequence <= last.sequence &&
            (kind === "consumed" || e.detail === last.detail),
        );
        if (!event) return false;
        sequence = event.sequence;
        return true;
      });
    })
  );
}
