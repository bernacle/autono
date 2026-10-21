import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import captured from "./fixtures/observability-journal.json";
import type { Run } from "../lib/types";
import { TaskEvidence } from "../components/evidence";
it("retains the actual review ordinal after an invalid review", () => {
  const run = structuredClone(captured.run) as unknown as Run;
  run.tasks = run.tasks.filter((t) => t.plan.id === "E");
  expect(run.tasks[0].attempts[0].reviews[0].review).toBeNull();
  expect(run.tasks[0].attempts[0].reviews[1].review).not.toBeNull();
  const output = renderToStaticMarkup(
    <TaskEvidence run={run} onProcess={() => {}} />,
  );
  expect(output.includes("Independent review 2")).toBe(true);
});
