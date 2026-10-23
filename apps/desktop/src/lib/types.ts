// Wire projection of autono-protocol v1. Mutations use the same daemon request objects.
export type Status =
  | "PLANNING"
  | "AWAITING_APPROVAL"
  | "RUNNING"
  | "VERIFIED"
  | "INTEGRATING"
  | "COMPLETED"
  | "BLOCKED"
  | "FAILED"
  | "CANCELLED"
  | "INTERRUPTED";
export type TaskStatus =
  | "BLOCKED_HUMAN"
  | "WAITING"
  | "READY"
  | "RUNNING"
  | "BLOCKED"
  | "FAILED"
  | "COMPLETED"
  | "CANCELLED"
  | "INTERRUPTED";
export interface Permission {
  operation: string;
  scope: string;
}
export interface Resolution {
  expected_action_version?: number;
  request_id: string;
  disposition: "resolve" | "approve" | "reject" | "defer";
  note: string;
  effect: "unknown" | "not_performed" | "already_performed";
  permission: Permission | null;
}
export interface HumanAction {
  prerequisite?: {
    category: string;
    basis: string;
    alternative_decision_id: string | null;
    verification_remaining: string[];
  } | null;
  id: string;
  status: "PENDING" | "RESOLVED" | "DEFERRED" | "REJECTED";
  continuable: boolean;
  requirement: {
    kind: "environment" | "product_decision" | "permission" | "runtime";
    reason: string;
    requested_action: string;
    affected_tasks: string[];
    blocking: boolean;
    evidence: string;
    confidence: number;
    permission: Permission | null;
  };
  resolution: Resolution | null;
  created_at: string;
  updated_at: string;
  audit: { at: string; status: string; request: Resolution | null }[];
}
export interface Preflight {
  independent_work_ready?: boolean;
  ready: boolean;
  checked_at: string;
  limitation: string;
  checks: {
    id: string;
    ready: boolean | null;
    blocking: boolean;
    evidence: string;
  }[];
  action_ids: string[];
}
export interface Workspace {
  id: string;
  path: string;
  branch: string;
  base_commit: string;
  head_commit: string | null;
}
export interface Session {
  id: string;
  role: string;
  provider: string;
  provider_session_id: string | null;
  process_run_id: string | null;
  process_key: string;
  outcome: string | null;
}
export interface Verification {
  process_key: string;
  process_run_id: string;
  command: string[];
  exit_code: number | null;
  passed: boolean;
  outcome: string | null;
  output_complete: boolean;
  summary: string;
}
export interface Review {
  decision_reviews?: {
    decision_id: string;
    implemented: boolean;
    evidence: string;
  }[];
  accepted: boolean;
  blocker: string;
  criteria: {
    criterion_id: string;
    criterion: string;
    satisfied: boolean;
    evidence: string;
  }[];
  findings: {
    severity: string;
    file: string;
    line: number;
    explanation: string;
    recommended_fix: string;
    confidence: number;
  }[];
}
export interface TaskPlan {
  id: string;
  goal: string;
  description: string;
  dependencies: string[];
  acceptance_criteria: string[];
  requirement_ids: string[];
  verification_expectations: string[];
  role: string;
}
export interface Task {
  plan: TaskPlan;
  status: TaskStatus;
  workspace: Workspace | null;
  artifact_commit: string | null;
  attempts: {
    number: number;
    decision_context?: string[];
    decision_ids?: string[] | null;
    coder: Session;
    reviewer: Session | null;
    verification: Verification[];
    review: Review | null;
    reviews: {
      session: Session;
      workspace: Workspace | null;
      review: Review | null;
      diagnostic: string | null;
    }[];
    snapshot: string | null;
    verified_tree: string | null;
    preserved_tree: string | null;
    detail: string | null;
  }[];
}
export interface AgentConfig {
  provider: string;
  model: string | null;
  timeout_seconds: number;
}
export interface Settings {
  agents: Record<"planner" | "coder" | "reviewer", AgentConfig>;
  execution: {
    max_parallel: number;
    max_attempts: number;
    max_review_cycles: number;
  };
  verification: { commands: string[][]; timeout_seconds: number };
}
export interface RunHandoff {
  outcome: string;
  summary: string;
  tasks: {
    task_id: string;
    delivery: string;
    implementation_recorded: boolean;
    checks_passed: boolean;
    artifact_commit: string | null;
  }[];
  needs_you: {
    action_id: string;
    action_version: number;
    category: string | null;
    affected_tasks: string[];
    waiting_descendants: string[];
    requested_action: string;
  }[];
  combined_checks_passed: boolean;
  completion_requirements_satisfied: boolean;
  internal_decision_ids: string[];
}
export interface Run {
  delegation?: {
    contract: DelegationContract;
    authorized_at: string;
    expires_at: string;
  } | null;
  handoff?: RunHandoff | null;

  decisions?: DecisionRecord[];
  id: string;
  revision: number;
  status: Status;
  created_at: string;
  updated_at: string;
  repository: string;
  requested_base: string;
  base_commit: string;
  specification: string;
  settings: Settings & { permissions: { rules: Record<string, string> } };
  approved_at: string | null;
  planner: Session | null;
  planning_workspace: Workspace;
  tasks: Task[];
  plan: {
    requirements: { id: string; text: string }[];
    milestones: { id: string; goal: string; task_ids: string[] }[];
    tasks: TaskPlan[];
  } | null;
  preflight: Preflight | null;
  human_actions: HumanAction[];
  blockers: {
    task_id: string | null;
    kind: string;
    detail: string;
    action: string;
    created_at: string;
  }[];
  integration_workspace: Workspace | null;
  integration_verification: Verification[];
  summary: string;
}
export interface Connection {
  address: string;
  home: string;
  engineering: { active_run: string | null; persistence_error: string | null };
  host: {
    api_version: number;
    pid: number;
    platform: string;
    database: string;
    admission: {
      active_runs: number;
      executing_runs: number;
      finalizing_runs: number;
      max_active_runs: number;
    };
    persistence: { healthy: boolean };
    power: {
      enabled: boolean;
      held: boolean;
      eligible_runs: number;
      last_error: string | null;
    };
  };
}
export interface Process {
  id: string;
  status: string;
  pid: number | null;
  program: string | null;
  cwd: string;
  created_at: string;
  detail: string | null;
  output_complete: boolean | null;
  runtime: { phase: string; persistence_error: string | null } | null;
}
export interface EngineeringEvent {
  sequence: number;
  run_id: string;
  timestamp: string;
  kind: string;
  detail: string;
}
export interface LogLine {
  sequence: number;
  timestamp?: string;
  stream: string;
  text: string;
}
export interface LogPage {
  after: number;
  lines: LogLine[];
  has_more: boolean;
}
export interface LocalSettings extends Settings {
  home: string;
  config_path: string;
  source: string;
  runner: {
    keep_awake: boolean;
    max_active_runs: number;
    max_pending_submissions: number;
  };
  cli: { name: string; available: boolean }[];
}
export type RunAction =
  "execute" | "preflight" | "continue" | "cancel" | "integrate";
export type Request =
  | { type: "propose_delegation"; request: DelegationProposal }
  | {
      type: "authorize_delegation";
      id: string;
      displayed_contract: DelegationContract;
    }
  | { type: "status" }
  | { type: "runs"; offset: number }
  | { type: "run"; id: string }
  | { type: "events"; id: string; after: number }
  | { type: "processes"; offset: number }
  | { type: "process_events"; id: string; after: number }
  | { type: "settings" }
  | {
      type: "plan";
      submission: {
        id: string;
        repository: string;
        base: string;
        specification: string;
      };
    }
  | { type: "action"; id: string; revision: number; action: RunAction }
  | {
      type: "resolve";
      id: string;
      revision: number;
      action_id: string;
      resolution: Resolution;
    };

export interface SourceScope {
  version: number;
  read_subtrees: string[];
  write_subtrees: string[];
  allow_create: boolean;
}
export interface DelegationProposal {
  submission: {
    id: string;
    repository: string;
    base: string;
    specification: string;
  };
  source_scope?: SourceScope | null;
  writable_files: string[];
  max_runtime_seconds: number;
  local_integration: boolean;
}
export interface DelegationContract {
  version: number;
  request: DelegationProposal;
  base_commit: string;
  worktree_root: string;
  settings: Run["settings"];
  verification_sandbox: string;
  source_inventory?: string[];
  limitations: string[];
}

export interface DecisionRecord {
  id: string;
  policy_basis: string;
  proposal: {
    question: string;
    chosen: string;
    rationale: string;
    alternatives: string[];
    affected_tasks: string[];
    assumptions: string[];
    uncertainty: string[];
  };
  evidence: {
    file: string | null;
    content_sha256: string;
    excerpt: string;
    source?: "frozen_base" | "specification" | "legacy_unverified";
    base_commit?: string | null;
  }[];
  audit: {
    sequence: number;
    at: string;
    kind: string;
    task_id: string | null;
    attempt: number | null;
    detail: string;
  }[];
}
