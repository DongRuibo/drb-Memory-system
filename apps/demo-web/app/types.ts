export type Health = {
  ready: boolean;
  memory_service: string;
  chat_store: string;
  model_configuration: string;
  detail: string | null;
};

export type DemoUserProfile = {
  user_id: string;
  display_name: string;
  role_label: string;
  avatar_initial: string;
  project_ids: string[];
  primary_project_id: string;
  primary_project_label: string;
  can_reset_project: boolean;
};

export type SessionSummary = {
  id: string;
  title: string;
  mode: "guided" | "free";
  guided_step: string;
  created_at: string;
  updated_at: string;
};

export type Message = {
  id: string;
  session_id: string;
  turn_id: string | null;
  role: "user" | "assistant" | "system";
  content: string;
  status: "pending" | "streaming" | "completed" | "failed" | "cancelled";
  created_at: string;
};

export type MemoryType =
  | "preference"
  | "profile_fact"
  | "semantic_fact"
  | "episodic_summary";

export type MemoryScopeKind = "user" | "project" | "tenant" | "agent";

export type EventTime = {
  start: string;
  end: string;
  timezone: string;
  precision:
    | "instant"
    | "minute"
    | "hour"
    | "part_of_day"
    | "day"
    | "week"
    | "month"
    | "quarter"
    | "year";
};

export type StructuredContent = {
  schema_name: string;
  schema_version: number;
  payload: Record<string, unknown>;
};

export type MemoryEffect = {
  kind:
    | "remembered"
    | "source_merged"
    | "corrected"
    | "forgotten"
    | "needs_review"
    | "extraction_summary";
  memory_record_id?: string;
  memory_version_id?: string | null;
  memory_type?: MemoryType;
  content?: string;
  revision?: number;
  resolution?: "automatic_supersede";
  model_candidate_count?: number | null;
  valid_model_candidate_count?: number;
  candidate_count?: number;
  filtered_reason_counts?: Record<string, number>;
  coverage_added_count?: number;
  coverage_enriched_count?: number;
  remembered_count?: number;
  source_merged_count?: number;
  corrected_count?: number;
  ignored_count?: number;
};

export type RuntimeContext = {
  current_time_utc: string;
  timezone: string;
  local_date: string;
};

export type PromptMemoryContext = {
  memory_record_id: string;
  memory_version_id: string;
  section_kind: "user_preference" | "current_fact" | "relevant_experience";
  memory_data: string | null;
  source_refs: string[];
  event_time: EventTime | null;
  inclusion_reason: string;
  trust: "untrusted_memory_data";
  allowed_effect:
    | "response_style_only"
    | "factual_personalization_only"
    | "contextual_example_only";
};

export type PromptContextSnapshot = {
  runtime_context: RuntimeContext;
  memory_context: PromptMemoryContext[];
};

export type Turn = {
  id: string;
  session_id: string;
  assistant_message_id: string;
  status: "accepted" | "processing" | "streaming" | "completed" | "failed" | "cancelled";
  context_pack_id: string | null;
  outcome_receipt_id: string | null;
  used_memory_ids: string[];
  used_memory_version_ids: string[];
  memory_effects: MemoryEffect[];
  prompt_context: PromptContextSnapshot | null;
  model_id: string | null;
  model_request_id_hash: string | null;
  total_tokens: number | null;
  error_detail: string | null;
};

export type SessionDetail = SessionSummary & {
  messages: Message[];
  turns: Turn[];
};

export type MemoryVersion = {
  memory_version_id: string;
  version_no: number;
  content: string | null;
  content_hash: string;
  normalized_key: string | null;
  confidence: number;
  importance: number;
  inference_kind: string;
  source_refs: string[];
  effective_from: string | null;
  effective_to: string | null;
  event_time: EventTime | null;
  structured_content: StructuredContent | null;
  classification: string;
  allowed_purposes: string[];
  consent_ref: string | null;
  retention_policy_id: string;
  retain_until: string | null;
  created_at: string;
  content_erased_at: string | null;
};

export type MemoryRecord = {
  memory_record_id: string;
  memory_type: MemoryType;
  semantic_key: string;
  scope: { kind: MemoryScopeKind; id: string | null };
  head_state: string;
  revision: number;
  current_version_id: string | null;
  identity_retired_at: string | null;
  versions: MemoryVersion[];
  events: Array<{
    sequence_no: number;
    event_type: string;
    memory_version_id: string | null;
    actor_type: string;
    actor_id: string;
    reason_code: string;
    payload: Record<string, unknown>;
    occurred_at: string;
  }>;
};

export type MemoryExport = {
  format_version: string;
  generated_at: string;
  record_count: number;
  records: MemoryRecord[];
};

export type DemoBootstrap = {
  current_user: DemoUserProfile;
  available_users: DemoUserProfile[];
  identity_revision: number;
  health: Health;
  sessions: SessionSummary[];
  memories: MemoryExport;
};

export type ApiError = { code?: string; detail?: string };
