/** Public Limina runtime contracts consumed by the web UI (see src/limina_cloud/schemas.py). */

export type Engine = "codex" | "claude-code";
export type Role = "OWNER" | "EDITOR" | "VIEWER";
export type ProjectStatus =
  | "CREATED"
  | "RUNNING"
  | "WAITING"
  | "PAUSED"
  | "STOPPED"
  | "FAILED"
  | "COMPLETE"
  | "ARCHIVED"
  | (string & {});

export type LifecycleAction = "start" | "pause" | "resume" | "stop" | "archive";
export type GuidanceKind = "STEER" | "ANSWER" | "APPROVAL" | "BLOCKER" | "INTERRUPT";

/** How this runtime signs people in (GET /auth/config). */
export type AuthMode = "workos" | "oidc" | "token" | "open";

export interface Me {
  subject: string;
  display_name: string;
  email: string | null;
  instance_admin: boolean;
  auth_mode: "local" | "oidc" | "workos";
}

export interface Project {
  slug: string;
  name: string;
  mission: string;
  success_criteria: string;
  context: string;
  runtime: Engine;
  model?: string | null;
  reasoning_effort?: string | null;
  status: ProjectStatus;
  current_objective: string;
  next_step: string;
  blocker: string;
  created_at: string;
  updated_at: string;
  role?: Role | null;
}

export interface Page<T> {
  items: T[];
  next_cursor: string | null;
  total: number;
}

export interface Observation {
  id: string;
  experiment_id: string;
  body: string;
  evidence_ref: string | null;
  actor: string;
  created_at: string;
}

export interface Artifact {
  id: string;
  kind: string;
  title: string;
  status: string;
  content: Record<string, unknown>;
  hypothesis_id?: string | null;
  experiment_id?: string | null;
  created_at: string;
  updated_at: string;
  observations?: Observation[] | null;
  tags: string[];
}

export interface KnowledgePage extends Page<Artifact> {
  search_backend: string;
}

export interface ProjectStatusResponse {
  project: Project;
  knowledge: Record<string, Record<string, number>>;
  active_work: Artifact[];
  pending_guidance: number;
  event_cursor: number;
}

export interface LiminaEvent {
  sequence: number;
  type: string;
  actor: string;
  artifact_id: string | null;
  detail: EventDetail;
  created_at: string;
}

/** Normalized runtime event detail. Every field is optional because event types differ. */
export interface EventDetail {
  summary?: string;
  run_id?: string;
  kind?: "message" | "tool" | "plan" | "status" | "knowledge";
  text?: string;
  tool?: string;
  input?: string | null;
  item_id?: string | null;
  phase?: "started" | "completed";
  state?: "running" | "ok" | "failed" | "declined" | "stopped" | null;
  exit_code?: number | null;
  duration_ms?: number | null;
  files?: { path: string; kind: string }[] | null;
  guidance_id?: string;
  origin?: string;
  body?: string;
  status?: string;
  current_objective?: string;
  next_step?: string;
  blocker?: string;
  error?: string;
  reason?: string;
  wake_at?: string;
  retry_count?: number;
  [key: string]: unknown;
}

export interface EventPage {
  events: LiminaEvent[];
  cursor: number;
  has_earlier?: boolean;
}

export interface GuidanceReceipt {
  id: string;
  delivery: "LIVE" | "QUEUED";
  kind: GuidanceKind;
  accepted_at: string;
  status: string;
}

export interface Guidance {
  id: string;
  sequence: number;
  kind: GuidanceKind;
  body: string;
  actor: string;
  status: string;
  created_at: string;
  acknowledged_at: string | null;
}

export interface RunUsage {
  input_tokens: number | null;
  output_tokens: number | null;
  cached_input_tokens: number | null;
  reasoning_output_tokens: number | null;
  total_tokens: number | null;
  cost_microusd: number | null;
  usage_source: string | null;
  cost_source: string | null;
}

export interface Run {
  id: string;
  runtime: Engine;
  model: string | null;
  status: string;
  summary: string;
  error: { code: string | null; message: string | null } | null;
  usage: RunUsage;
  tool_calls: number;
  retry_count: number;
  started_at: string;
  completed_at: string | null;
  duration_ms: number | null;
}

export interface MissionUpdate {
  headline: string;
  established: string;
  ruled_out: string;
  uncertain: string;
  evidence: string[];
  decision: { question: string; recommendation: string; trade_off: string } | null;
}
export interface WorkspaceAssessment {
  sequence: number;
  created_at: string;
  summary: string;
  mission_update: MissionUpdate | null;
}
export interface Revision {
  version: number;
  title: string;
  status: string;
  content: Record<string, unknown>;
  actor: string;
  created_at: string;
}
export interface OutputFile { name: string; bytes: number }
export interface OutputPage { items: OutputFile[]; next_offset: number | null; truncated: boolean }
export interface OutputPreview extends OutputFile { text: string | null; truncated: boolean }

export interface Analytics {
  window: { days: number; since: string };
  runs: {
    total: number;
    by_status: Record<string, number>;
    success_rate: number | null;
    average_duration_ms: number | null;
    p95_duration_ms: number | null;
    total_tokens: number | null;
    cost_microusd: number | null;
    tool_calls: number;
  };
  knowledge: { created: number; by_kind: Record<string, number>; by_status: Record<string, number> };
  guidance: { total: number; pending: number; average_acknowledgement_seconds: number | null };
  timeseries: {
    date: string;
    runs: number;
    completed_runs: number;
    failed_runs: number;
    artifacts: number;
    guidance: number;
  }[];
}

export interface PreflightCheck {
  name: string;
  status: "PASS" | "WARN" | "FAIL" | "INFO";
  detail: string;
}

export interface Preflight {
  ready: boolean;
  checks: PreflightCheck[];
}

export interface EngineInfo {
  engine: Engine;
  label: string;
  installed: boolean;
  credentials: boolean;
  detail: string;
  model: string | null;
}

export interface Resource {
  name: string;
  type: "VARIABLE" | "SECRET";
  status: string;
  value?: string | null;
  configured?: boolean | null;
  created_at: string;
  updated_at: string;
}

export interface Source {
  id: string;
  name: string;
  type: "URL" | "CONNECTOR" | "UPLOAD";
  uri: string;
  media_type: string | null;
  metadata: Record<string, unknown>;
  status: string;
  created_at: string;
  updated_at: string;
}

export interface Member {
  subject: string;
  display_name: string;
  email: string | null;
  role: Role;
  created_at: string;
  updated_at: string;
}

export interface KickoffTemplate {
  id: string;
  name: string;
  description: string;
  defaults: { context: string; success_criteria: string };
}

export interface EngineAccount {
  engine: Engine;
  method: "claude-login" | "subscription" | "api-key" | "chatgpt" | (string & {});
  label: string | null;
  connected_by: string | null;
  connected_at: string | null;
}

/** Which account a project's turns use for one engine. */
export interface ProjectEngineAccount {
  engine: Engine;
  label: string;
  account: EngineAccount | null;
  runtime_available: boolean;
  runtime_detail: string;
}

export interface CodexDeviceLogin {
  login_id: string;
  status: "PENDING" | "SUCCEEDED" | "FAILED" | "CANCELLED";
  verification_url: string;
  user_code: string | null;
  error: string | null;
  created_at: string;
  completed_at: string | null;
}

export interface ModelOption {
  id: string;
  label: string;
  resolved_model?: string | null;
  reasoning_efforts: string[];
  default_reasoning_effort: string | null;
}
export interface ModelCatalog {
  default_model: string | null;
  models: ModelOption[];
}

export interface MetricInput {
  name: string;
  criterion: string;
  unit: string;
  direction: "at_least" | "at_most";
  target: number;
  baseline: number | null;
  current: number | null;
  evidence_id: string | null;
  observed_at: string | null;
  notes: string;
}
export interface Metric extends MetricInput {
  id: string;
  version: number;
  updated_by: string;
  updated_at: string;
}
