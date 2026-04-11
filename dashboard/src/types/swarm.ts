export type RunStatus =
  | "pending"
  | "planning"
  | "planned"
  | "approved"
  | "running"
  | "passed"
  | "failed"
  | "skipped";

export interface PhaseTimingRecord {
  embedding_ms: number;
  rag_ms: number;
  planning_ms: number;
  agent_execution_ms: number;
  quality_gate_ms: number;
}

export interface ToolCallRecord {
  tool: string;
  params_preview: string;
  result_preview: string;
  is_error: boolean;
  duration_ms: number;
}

export interface AttemptRecord {
  attempt: number;
  model: string;
  prompt_preview: string;
  response_preview: string;
  tokens_input: number;
  tokens_output: number;
  duration_ms: number;
  quality_passed: boolean | null;
  error: string | null;
  timestamp: string;
}

export interface AgentRun {
  id: string | null;
  project: string;
  task_description: string;
  agent_id: string;
  model_used: string;
  status: RunStatus;
  files_modified: string[];
  diff: string | null;
  error_message: string | null;
  quality_gate_passed: boolean | null;
  tokens_input: number;
  tokens_output: number;
  duration_ms: number;
  created_at: string;
  started_at: string | null;
  last_activity_at: string | null;
  parent_run_id: string | null;
  progress_message: string | null;
  attempts?: AttemptRecord[];
  tool_calls?: ToolCallRecord[];
  phase_timings?: PhaseTimingRecord | null;
}

export interface PlannedTask {
  id: string;
  description: string;
  files: string[];
  complexity: string;
  rationale: string;
}

export interface GoalPlan {
  id: string | null;
  run_id: string;
  project: string;
  goal: string;
  version: number;
  sub_tasks: PlannedTask[];
  model_used: string;
  tokens_input: number;
  tokens_output: number;
  duration_ms: number;
  user_feedback: string | null;
  status: string;
  context_files: string[];
  reasoning: string;
  created_at: string;
}

export interface Project {
  id: string | null;
  name: string;
  repo_url: string;
  branch: string;
  description: string;
  status: string;
  created_at: string;
}

export interface ResourceSnapshot {
  host: string;
  host_type: string;
  cpu_percent: number;
  ram_total_mb: number;
  ram_used_mb: number;
  ram_percent: number;
  disk_total_gb: number;
  disk_free_gb: number;
  disk_percent: number;
  ollama_models: Array<{ name: string; size_mb: number }>;
  timestamp: string;
}

export interface DashboardStats {
  total_runs: number;
  active: number;
  passed: number;
  failed: number;
  pending: number;
  total_tokens_in: number;
  total_tokens_out: number;
  total_duration_ms: number;
}
