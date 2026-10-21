import {
  checkEventOutcome,
  reviewAccepts,
  type CheckOutcome,
} from "./evidence-model";
import type { EngineeringEvent, Run, Session, TaskStatus } from "./types";

/** Journal conventions, pinned to engineering/{store,engine,service}.rs. No mutations. */
export interface Source {
  sequence: number | null;
  timestamp: string;
  kind: string;
}
export interface Fact extends Source {
  sequence: number;
  time: number;
  task?: string;
  attempt?: number;
  state?: TaskStatus;
  from?: TaskStatus;
  session?: string;
  process?: string;
  key?: string;
  role?: string;
  provider?: string;
  outcome?: string;
  accepted?: boolean;
  findings?: number;
  cause?: Source;
}
export interface History {
  cursor: number;
  count: number;
  limited: boolean;
  caughtUp: boolean;
  unparsed: number;
  facts: Fact[];
  recent: EngineeringEvent[];
  cause?: Source;
}
export type Phase =
  | "planning"
  | "awaiting"
  | "executing"
  | "verified"
  | "integrating"
  | "waiting"
  | "ready"
  | "running"
  | "paused"
  | "blocked"
  | "coding"
  | "checks"
  | "review";
export interface Segment {
  id: string;
  task?: string;
  attempt?: number;
  review?: number;
  phase: Phase;
  start: number;
  end: number | null;
  live: boolean;
  outcome: "recorded" | "failed" | "interrupted" | CheckOutcome;
  sources: Source[];
  session?: string;
  process?: string;
  key?: string;
  provider?: string;
  waitingOn?: string[];
  humanHeld?: boolean;
}
export interface Marker {
  id: string;
  task?: string;
  attempt?: number;
  time: number;
  label: string;
  kind:
    | "retry"
    | "completed"
    | "decision"
    | "continued"
    | "failure"
    | "finding"
    | "incomplete";
  source: Source;
}
export interface ExecutionModel {
  runId: string;
  segments: Segment[];
  states: Segment[];
  markers: Marker[];
  unparsed: number;
  incomplete: number;
  lastProgress: number | null;
  lastEvent: number | null;
  cursor: number;
  limited: boolean;
  caughtUp: boolean;
  retries: number;
  reviewRejections: number;
}
const states: Record<string, TaskStatus> = {
  Waiting: "WAITING",
  Ready: "READY",
  Running: "RUNNING",
  BlockedHuman: "BLOCKED_HUMAN",
  Blocked: "BLOCKED",
  Failed: "FAILED",
  Completed: "COMPLETED",
  Cancelled: "CANCELLED",
  Interrupted: "INTERRUPTED",
};
const id = "[A-Za-z0-9_-]{1,64}";
const uuid = "[a-fA-F0-9]{8}(?:-[a-fA-F0-9]{4}){3}-[a-fA-F0-9]{12}";
const patterns = {
  state: new RegExp(`^(${id}): (?:None|Some\\(([A-Za-z]+)\\)) -> ([A-Za-z]+)$`),
  bare: new RegExp(`^(${id})$`),
  attempt: new RegExp(`^(${id}): attempt ([1-9][0-9]*)$`),
  reserved: new RegExp(`^(${id}): review ([1-9][0-9]*)$`),
  review: new RegExp(`^(${id}): accepted=(true|false) findings=([0-9]+)$`),
  commit: new RegExp(`^(${id}): ([a-f0-9]{40}|[a-f0-9]{64})$`),
  prefix: new RegExp(`^(${id}): [\\s\\S]+$`),
  agentStart: new RegExp(
    `^(Planner|Coder|Reviewer) (Codex|Claude); session=(${uuid}) process_key=(${uuid})$`,
  ),
  agentEnd: new RegExp(
    `^(Planner|Coder|Reviewer) session=(${uuid}) outcome=([a-z_]+)$`,
  ),
  agentProcess: new RegExp(`^(Planner|Coder|Reviewer) process=(${uuid})$`),
  check: new RegExp(
    `^target=(None|Some\\(\\("(${id})", ([1-9][0-9]*)\\)\\)) (process_key|process)=(${uuid})(?: command=(\\[[\\s\\S]*\\])| passed=(true|false))?$`,
  ),
};
const controls = new Set([
  "created",
  "planning_workspace_ready",
  "plan_ready",
  "preflight",
  "approved",
  "continued",
  "scheduler",
  "execution_finished",
  "integration_requested",
  "integration_workspace_ready",
  "completed",
  "cancelled",
  "workflow_stopped",
  "interrupted",
  "human_resolution",
]);
const bare = new Set([
  "task_started",
  "workspace_ready",
  "verification_passed",
  "review_workspace_ready",
]);
const prefixed = new Set([
  "workspace_reserved",
  "task_stopped",
  "review_invalid_or_failed",
]);
export const emptyHistory = (): History => ({
  cursor: 0,
  count: 0,
  limited: false,
  caughtUp: false,
  unparsed: 0,
  facts: [],
  recent: [],
});
export function parseFact(
  event: EngineeringEvent,
  taskIds: ReadonlySet<string>,
): Fact | null {
  const time = Date.parse(event.timestamp);
  if (!Number.isFinite(time) || !/^\d{4}-\d\d-\d\dT/.test(event.timestamp))
    return null;
  const f: Fact = {
    sequence: event.sequence,
    timestamp: event.timestamp,
    kind: event.kind,
    time,
  };
  const d = event.detail;
  let m: RegExpMatchArray | null;
  if (controls.has(f.kind)) return f;
  if (f.kind === "task_state") {
    m = d.match(patterns.state);
    if (!m || !states[m[3]] || (m[2] && !states[m[2]])) return null;
    f.task = m[1];
    f.from = states[m[2]];
    f.state = states[m[3]];
  } else if (bare.has(f.kind)) {
    m = d.match(patterns.bare);
    if (!m) return null;
    f.task = m[1];
  } else if (["attempt_started", "attempt_feedback"].includes(f.kind)) {
    m = d.match(patterns.attempt);
    if (!m || !Number.isSafeInteger(+m[2])) return null;
    f.task = m[1];
    f.attempt = +m[2];
  } else if (f.kind === "review_reserved") {
    m = d.match(patterns.reserved);
    if (!m) return null;
    f.task = m[1];
  } else if (f.kind === "review_finished") {
    m = d.match(patterns.review);
    if (!m) return null;
    f.task = m[1];
    f.accepted = m[2] === "true";
    f.findings = +m[3];
  } else if (["task_completed", "integration_merge"].includes(f.kind)) {
    m = d.match(patterns.commit);
    if (!m) return null;
    f.task = m[1];
  } else if (prefixed.has(f.kind)) {
    m = d.match(patterns.prefix);
    if (!m) return null;
    f.task = m[1];
  } else if (f.kind === "agent_started") {
    m = d.match(patterns.agentStart);
    if (!m) return null;
    f.role = m[1].toLowerCase();
    f.provider = m[2].toLowerCase();
    f.session = m[3].toLowerCase();
    f.key = m[4].toLowerCase();
  } else if (f.kind === "agent_finished") {
    m = d.match(patterns.agentEnd);
    if (!m) return null;
    f.role = m[1].toLowerCase();
    f.session = m[2].toLowerCase();
    f.outcome = m[3];
  } else if (f.kind === "agent_process") {
    m = d.match(patterns.agentProcess);
    if (!m) return null;
    f.role = m[1].toLowerCase();
    f.process = m[2].toLowerCase();
  } else if (
    [
      "verification_started",
      "verification_process",
      "verification_finished",
    ].includes(f.kind)
  ) {
    m = d.match(patterns.check);
    if (!m) return null;
    f.task = m[2] || undefined;
    f.attempt = m[3] ? +m[3] : undefined;
    if (f.attempt && !Number.isSafeInteger(f.attempt)) return null;
    if (f.kind === "verification_started") {
      if (m[4] !== "process_key" || !m[6] || m[7]) return null;
      f.key = m[5].toLowerCase();
    } else {
      if (
        m[4] !== "process" ||
        m[6] ||
        (f.kind === "verification_finished") !== !!m[7]
      )
        return null;
      f.process = m[5].toLowerCase();
      if (m[7]) f.accepted = m[7] === "true";
    }
  } else return null;
  return f.task && !taskIds.has(f.task) ? null : f;
}
/** Stores only normalized facts (<=5,000) and the bounded journal tail (<=500). */
export function appendHistory(
  previous: History | undefined,
  runId: string,
  taskIds: ReadonlySet<string>,
  incoming: EngineeringEvent[],
  pageSize = 100,
): History {
  const p = previous ?? emptyHistory();
  const h: History = {
    ...p,
    facts: [...p.facts],
    recent: [...p.recent],
    caughtUp: incoming.length < pageSize,
  };
  for (const e of [...incoming].sort((a, b) => a.sequence - b.sequence)) {
    if (
      e.run_id !== runId ||
      !Number.isSafeInteger(e.sequence) ||
      e.sequence <= h.cursor
    )
      continue;
    h.cursor = e.sequence;
    h.recent.push(e);
    if (h.count >= 5000) {
      h.limited = true;
      continue;
    }
    h.count++;
    const fact = parseFact(e, taskIds);
    if (fact) {
      if (fact.kind === "task_state") fact.cause = h.cause;
      h.facts.push(fact);
    } else h.unparsed++;
    if (e.kind !== "task_state")
      h.cause = { sequence: e.sequence, timestamp: e.timestamp, kind: e.kind };
  }
  h.recent = h.recent.slice(-500);
  return h;
}
interface Binding {
  task?: string;
  attempt?: number;
  review?: number;
  session: Session;
}
export function sessionBindings(run: Run) {
  const map = new Map<string, Binding>();
  if (run.planner) map.set(run.planner.id, { session: run.planner });
  for (const t of run.tasks)
    for (const a of t.attempts) {
      map.set(a.coder.id, {
        task: t.plan.id,
        attempt: a.number,
        session: a.coder,
      });
      const reviews = a.reviews.length
        ? a.reviews.map((r) => r.session)
        : a.reviewer
          ? [a.reviewer]
          : [];
      reviews.forEach((s, i) =>
        map.set(s.id, {
          task: t.plan.id,
          attempt: a.number,
          review: i + 1,
          session: s,
        }),
      );
    }
  return map;
}
const statePhase: Partial<Record<TaskStatus, Phase>> = {
  WAITING: "waiting",
  READY: "ready",
  RUNNING: "running",
  BLOCKED_HUMAN: "paused",
  BLOCKED: "blocked",
};
const terminalRun = (r: Run) =>
  ["COMPLETED", "FAILED", "CANCELLED", "INTERRUPTED"].includes(r.status);
const ref = (f: Source): Source => ({
  sequence: f.sequence,
  timestamp: f.timestamp,
  kind: f.kind,
});
const sourceId = (s: Source) => `${s.kind}:${s.sequence ?? s.timestamp}`;
/** Pure projection: sequence is causal order; timestamps are never sorted or repaired. */
export function deriveExecution(run: Run, history: History): ExecutionModel {
  const segments: Segment[] = [],
    intervals: Segment[] = [],
    markers: Marker[] = [];
  const bindings = sessionBindings(run),
    byProcess = new Map<string, Binding>();
  for (const b of bindings.values())
    if (b.session.process_run_id) byProcess.set(b.session.process_run_id, b);
  const tasks = new Map(run.tasks.map((t) => [t.plan.id, t]));
  const observed = new Map<string, Fact>(),
    attempts = new Map<string, number>();
  const agents = new Map<string, Segment>(),
    checks = new Map<string, Segment>();
  const checkProcesses = new Map<string, Segment>(),
    reviewByTask = new Map<string, Segment>();
  const checkKeys = new Map<string, string>();
  for (const t of run.tasks)
    for (const a of t.attempts)
      for (const v of a.verification)
        checkKeys.set(v.process_run_id, v.process_key);
  for (const v of run.integration_verification)
    checkKeys.set(v.process_run_id, v.process_key);
  const finishedCheckKeys = new Set(checkKeys.values());
  let unparsed = history.unparsed,
    incomplete = 0,
    lastProgress: number | null = null;
  let band: Segment | undefined;
  const liveAllowed = history.caughtUp && !history.limited;
  const add = (s: Segment, end: Fact, target = segments) => {
    if (end.time < s.start) {
      incomplete++;
      return;
    }
    const closed = {
      ...s,
      end: end.time,
      live: false,
      sources: [...s.sources, ref(end)],
    };
    target.push(closed);
    return closed;
  };
  const start = (
    phase: Phase,
    f: Fact,
    extra: Partial<Segment> = {},
  ): Segment => ({
    id: `${phase}:${f.sequence}`,
    phase,
    start: f.time,
    end: null,
    live: false,
    outcome: "recorded",
    sources: [ref(f)],
    ...extra,
  });
  const changeBand = (phase: Phase | undefined, f: Fact) => {
    if (band) add(band, f);
    band = phase ? start(phase, f) : undefined;
  };
  const mark = (f: Fact, kind: Marker["kind"], label: string) =>
    markers.push({
      id: `${kind}:${f.sequence}`,
      task: f.task,
      attempt: f.attempt,
      time: f.time,
      kind,
      label,
      source: ref(f),
    });
  for (const f of history.facts) {
    if (f.kind === "created") changeBand("planning", f);
    if (f.kind === "plan_ready") {
      changeBand("awaiting", f);
      lastProgress = f.time;
    }
    if (f.kind === "approved") changeBand("executing", f);
    if (f.kind === "continued") {
      if (band?.phase !== "executing") changeBand("executing", f);
      mark(f, "continued", "Developer explicitly continued eligible work");
    }
    if (f.kind === "integration_requested") changeBand("integrating", f);
    if (
      ["completed", "cancelled", "interrupted", "workflow_stopped"].includes(
        f.kind,
      )
    ) {
      changeBand(undefined, f);
      if (f.kind === "completed") lastProgress = f.time;
      for (const [key, s] of agents) {
        add({ ...s, outcome: "interrupted" }, f);
        agents.delete(key);
      }
      for (const [key, s] of checks) {
        add({ ...s, outcome: checkEventOutcome(run, s) }, f);
        checks.delete(key);
      }
      if (f.kind !== "completed")
        mark(
          f,
          "failure",
          f.kind === "workflow_stopped"
            ? "Workflow stopped; inspect retained evidence"
            : `Run ${f.kind}`,
        );
    }
    if (f.kind === "task_state" && f.task && f.state) {
      const previous = observed.get(f.task);
      if (previous?.state) {
        const phase = statePhase[previous.state];
        if (phase) {
          if (f.from && f.from !== previous.state) incomplete++;
          else
            add(
              start(phase, previous, {
                task: f.task,
                sources: [
                  ref(previous),
                  ...(previous.cause ? [ref(previous.cause)] : []),
                ],
              }),
              f,
              intervals,
            );
        }
      }
      observed.set(f.task, f);
      if (["FAILED", "INTERRUPTED", "CANCELLED"].includes(f.state))
        mark(f, "failure", `${f.task}: ${f.state.toLowerCase()}`);
    }
    if (f.kind === "execution_finished" && band?.phase === "executing") {
      // This event can be a no-op. Match the daemon checkpoint only when all task
      // states are known and no Ready/Waiting task has satisfied dependencies.
      const known = run.tasks.every((t) => observed.has(t.plan.id));
      const runnable = run.tasks.some(
        (t) =>
          ["READY", "WAITING"].includes(observed.get(t.plan.id)?.state ?? "") &&
          t.plan.dependencies.every(
            (d) => observed.get(d)?.state === "COMPLETED",
          ),
      );
      if (
        known &&
        !runnable &&
        ![...observed.values()].some((t) => t.state === "RUNNING")
      ) {
        const all = run.tasks.every(
          (t) => observed.get(t.plan.id)?.state === "COMPLETED",
        );
        const blocked = [...observed.values()].some((t) =>
          ["BLOCKED", "BLOCKED_HUMAN"].includes(t.state ?? ""),
        );
        changeBand(all ? "verified" : blocked ? "blocked" : undefined, f);
      }
    }
    if (f.kind === "attempt_started" && f.task && f.attempt) {
      attempts.set(f.task, f.attempt);
      if (f.attempt > 1) mark(f, "retry", `Attempt ${f.attempt} started`);
    }
    if (f.kind === "task_completed") {
      mark(f, "completed", "Verified artifact accepted and preserved");
      lastProgress = f.time;
    }
    if (
      f.kind === "verification_passed" ||
      (f.kind === "review_finished" && f.accepted) ||
      f.kind === "integration_merge"
    )
      lastProgress = f.time;
    if (f.kind === "agent_started" || f.kind === "agent_finished") {
      const b = bindings.get(f.session ?? "");
      if (
        !b ||
        b.session.role.toLowerCase() !== f.role ||
        (f.key && b.session.process_key !== f.key)
      ) {
        unparsed++;
        continue;
      }
      if (f.kind === "agent_started") {
        if (agents.has(f.session!)) {
          unparsed++;
          continue;
        }
        const phase =
          f.role === "planner"
            ? "planning"
            : f.role === "reviewer"
              ? "review"
              : "coding";
        const s = start(phase, f, {
          task: b.task,
          attempt: b.attempt,
          review: b.review,
          session: f.session,
          process: b.session.process_run_id ?? undefined,
          provider: f.provider,
        });
        agents.set(f.session!, s);
        if (b.task && phase === "review") reviewByTask.set(b.task, s);
      } else {
        const s = agents.get(f.session!);
        if (!s) {
          unparsed++;
          continue;
        }
        s.outcome =
          f.outcome === "succeeded"
            ? s.phase === "review"
              ? "recorded"
              : "passed"
            : "failed";
        const closed = add(s, f);
        if (s.task && s.phase === "review" && closed)
          reviewByTask.set(s.task, closed);
        agents.delete(f.session!);
      }
    }
    if (f.kind === "agent_process") {
      const b = byProcess.get(f.process ?? "");
      if (!b || b.session.role.toLowerCase() !== f.role) unparsed++;
      else {
        const s = agents.get(b.session.id);
        if (s) {
          s.process = f.process;
          s.sources.push(ref(f));
        }
      }
    }
    if (f.kind === "verification_started")
      checks.set(
        f.key!,
        start("checks", f, { task: f.task, attempt: f.attempt, key: f.key }),
      );
    if (["verification_process", "verification_finished"].includes(f.kind)) {
      let s = checkProcesses.get(f.process!);
      if (!s) {
        const key = checkKeys.get(f.process!);
        if (key) s = checks.get(key);
      }
      if (!s) {
        const candidates = [...checks.values()].filter(
          (s) => s.task === f.task && s.attempt === f.attempt && !s.process,
        );
        if (candidates.length === 1) s = candidates[0];
      }
      if (!s || s.task !== f.task || s.attempt !== f.attempt) {
        unparsed++;
        continue;
      }
      s.process = f.process;
      checkProcesses.set(f.process!, s);
      if (f.kind === "verification_process") s.sources.push(ref(f));
      else {
        s.outcome = checkEventOutcome(run, { ...f, key: s.key });
        add(s, f);
        checks.delete(s.key!);
        checkProcesses.delete(f.process!);
      }
    }
    if (["review_finished", "review_invalid_or_failed"].includes(f.kind)) {
      const s = reviewByTask.get(f.task!);
      if (!s || s.attempt !== attempts.get(f.task!)) {
        unparsed++;
        continue;
      }
      const recordedReview = tasks
        .get(f.task!)
        ?.attempts.find((a) => a.number === s.attempt)
        ?.reviews.find((r) => r.session.id === s.session)?.review;
      s.outcome =
        f.accepted && (!recordedReview || reviewAccepts(recordedReview))
          ? "passed"
          : "failed";
      s.sources.push(ref(f));
      if (f.findings) {
        const review = tasks
          .get(f.task!)
          ?.attempts.find((a) => a.number === s.attempt)
          ?.reviews.find((r) => r.session.id === s.session)?.review;
        const counts = review?.findings.reduce<Record<string, number>>(
          (all, finding) => {
            all[finding.severity] = (all[finding.severity] ?? 0) + 1;
            return all;
          },
          {},
        );
        mark(
          { ...f, attempt: s.attempt },
          "finding",
          `${f.findings} reviewer ${f.findings === 1 ? "finding" : "findings"}${
            counts
              ? ` · ${Object.entries(counts)
                  .map(([severity, n]) => `${n} ${severity}`)
                  .join(", ")}`
              : ""
          }`,
        );
      }
    }
  }
  for (const [task, f] of observed) {
    const phase = f.state && statePhase[f.state];
    if (!phase) continue;
    const snapshot = tasks.get(task)!;
    if (liveAllowed && !terminalRun(run) && snapshot.status === f.state)
      intervals.push({
        ...start(phase, f, {
          task,
          sources: [ref(f), ...(f.cause ? [ref(f.cause)] : [])],
        }),
        live: true,
      });
    else incomplete++; // A terminal snapshot cannot supply a missing task-state boundary.
  }
  for (const s of agents.values()) {
    const b = bindings.get(s.session!);
    if (
      liveAllowed &&
      !terminalRun(run) &&
      b &&
      !b.session.outcome &&
      (s.phase === "planning"
        ? run.status === "PLANNING"
        : tasks.get(s.task!)?.status === "RUNNING")
    )
      segments.push({ ...s, live: true });
    else {
      incomplete++;
      markers.push({
        id: `incomplete:${s.id}`,
        task: s.task,
        attempt: s.attempt,
        time: s.start,
        kind: "incomplete",
        label: "Session start recorded; end not observed",
        source: s.sources[0],
      });
    }
  }
  for (const s of checks.values()) {
    if (
      liveAllowed &&
      !terminalRun(run) &&
      !finishedCheckKeys.has(s.key!) &&
      (s.task
        ? tasks.get(s.task)?.status === "RUNNING"
        : run.status === "INTEGRATING")
    )
      segments.push({ ...s, live: true });
    else incomplete++;
  }
  if (band) {
    const expected: Partial<Record<Run["status"], Phase>> = {
      PLANNING: "planning",
      AWAITING_APPROVAL: "awaiting",
      RUNNING: "executing",
      BLOCKED: "blocked",
      VERIFIED: "verified",
      INTEGRATING: "integrating",
    };
    if (liveAllowed && expected[run.status] === band.phase && !terminalRun(run))
      segments.push({ ...band, live: true });
    else incomplete++;
  }
  // Dependency names are derived from the recorded status at the wait's start,
  // never from a later snapshot that may already have completed them.
  for (const s of intervals)
    if (s.phase === "waiting") {
      s.waitingOn = tasks
        .get(s.task!)!
        .plan.dependencies.filter(
          (dep) =>
            !history.facts.some(
              (x) =>
                x.task === dep &&
                x.kind === "task_state" &&
                x.state === "COMPLETED" &&
                x.time <= s.start,
            ) &&
            !markers.some(
              (m) =>
                m.task === dep && m.kind === "completed" && m.time <= s.start,
            ),
        );
      s.humanHeld = history.facts.some(
        (f) =>
          f.kind === "task_state" &&
          f.state === "BLOCKED_HUMAN" &&
          s.waitingOn?.includes(f.task!) &&
          f.time < (s.end ?? Infinity),
      );
    }

  // A completed run must not leave stale open bars after a truncated/mismatched stream.
  const stateSegments = intervals.filter((s) => s.phase !== "running");
  for (const a of run.human_actions) {
    const points = [
      { at: a.created_at, status: "requested" },
      ...a.audit
        .filter((x) => x.at !== a.created_at || x.status !== "PENDING")
        .map((x) => ({ at: x.at, status: x.status.toLowerCase() })),
    ];
    for (const p of points)
      if (Number.isFinite(Date.parse(p.at)))
        for (const task of a.requirement.affected_tasks.length
          ? a.requirement.affected_tasks
          : [undefined])
          markers.push({
            id: `decision:${a.id}:${p.at}:${task}`,
            task,
            time: Date.parse(p.at),
            kind: "decision",
            label: `Human decision ${p.status}`,
            source: {
              kind: "human_action.audit",
              sequence: null,
              timestamp: p.at,
            },
          });
  }
  return {
    runId: run.id,
    segments: [...segments, ...stateSegments].sort((a, b) => a.start - b.start),
    states: intervals,
    markers,
    unparsed,
    incomplete,
    lastProgress,
    lastEvent:
      [...history.recent]
        .reverse()
        .map((e) => Date.parse(e.timestamp))
        .find(Number.isFinite) ?? null,
    cursor: history.cursor,
    limited: history.limited,
    caughtUp: history.caughtUp,
    retries: history.facts.filter(
      (f) => f.kind === "attempt_started" && f.attempt! > 1,
    ).length,
    reviewRejections: history.facts.filter(
      (f) => f.kind === "review_finished" && f.accepted === false,
    ).length,
  };
}
export function intervalEnd(s: Segment, now: number) {
  return s.end ?? (s.live ? Math.max(s.start, now) : s.start);
}
export function unionDuration(intervals: Segment[], now: number) {
  const sorted = intervals
    .map((s) => [s.start, intervalEnd(s, now)])
    .sort((a, b) => a[0] - b[0]);
  let total = 0,
    start = 0,
    end = 0;
  for (const [a, b] of sorted) {
    if (a > end) {
      total += end - start;
      start = a;
      end = b;
    } else end = Math.max(end, b);
  }
  return total + end - start;
}
export const sourceLabel = (s: Source) =>
  `${s.kind}${s.sequence === null ? " (snapshot)" : ` #${s.sequence}`}`;
export const sourceKey = sourceId;
