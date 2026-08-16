use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use memory_application::{ExtractCandidateProposal, ExtractionDiagnostics, TemporalWindow};
use memory_domain::{
    CompletedTurn, ConsolidationStatus, ConsolidationTrigger, Episode, EventTime,
    StructuredContent, TurnOutcome, VerifiedToolResult,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DemoAuthority {
    pub tenant_id: String,
    pub user_id: String,
    pub tenant_scope: bool,
    pub project_ids: BTreeSet<String>,
    pub agent_ids: BTreeSet<String>,
}

impl DemoAuthority {
    pub fn user(tenant_id: impl Into<String>, user_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            user_id: user_id.into(),
            tenant_scope: false,
            project_ids: BTreeSet::new(),
            agent_ids: BTreeSet::new(),
        }
    }

    pub fn worker(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            user_id: "__memory_worker__".into(),
            tenant_scope: true,
            project_ids: BTreeSet::new(),
            agent_ids: BTreeSet::new(),
        }
    }

    pub fn authorizes_scope(&self, kind: &str, id: Option<&str>) -> bool {
        match (kind, id) {
            ("tenant", None) => self.tenant_scope,
            ("user", Some(id)) => id == self.user_id,
            ("project", Some(id)) => self.project_ids.contains(id),
            ("agent", Some(id)) => self.agent_ids.contains(id),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IngestCommand {
    pub idempotency_key: String,
    pub event_id: String,
    pub source_type: String,
    pub source_ref: String,
    pub source_content: String,
    pub memory_content: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct IngestCandidateCommand {
    pub idempotency_key: String,
    pub event_id: String,
    pub source_type: String,
    pub source_ref: String,
    pub source_content: String,
    pub memory_content: String,
    pub occurred_at: DateTime<Utc>,
    pub event_time: Option<EventTime>,
    pub structured_content: Option<StructuredContent>,
    pub memory_type: String,
    pub semantic_key: String,
    pub confidence: f64,
    pub importance: f64,
    pub inference_kind: String,
    pub extractor_name: String,
    pub extractor_version: String,
    pub scope_kind: String,
    pub scope_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExtractionSnapshotCommand {
    pub idempotency_key: String,
    pub request_hash: String,
    pub extractor_version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractionSnapshotView {
    pub extractor_version: String,
    pub proposals: Option<Vec<ExtractCandidateProposal>>,
    pub diagnostics: Option<ExtractionDiagnostics>,
}

#[derive(Debug, Clone)]
pub struct AppendCompletedTurnCommand {
    pub idempotency_key: String,
    pub request_hash: String,
    pub episode_id: Uuid,
    pub turn_id: Uuid,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub user_content: String,
    pub assistant_content: String,
    pub verified_tool_results: Vec<VerifiedToolResult>,
    pub outcome: TurnOutcome,
    pub completed_at: DateTime<Utc>,
    pub timezone: String,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletedTurnReceiptView {
    pub episode_id: Uuid,
    pub turn_id: Uuid,
    pub sequence_no: u64,
    pub consolidation_id: Option<Uuid>,
    pub consolidation_status: Option<ConsolidationStatus>,
}

#[derive(Debug, Clone)]
pub struct RequestConsolidationCommand {
    pub idempotency_key: String,
    pub episode_id: Uuid,
    pub trigger: ConsolidationTrigger,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationView {
    pub consolidation_id: Uuid,
    pub episode_id: Uuid,
    pub from_turn_sequence: u64,
    pub through_turn_sequence: u64,
    pub trigger: ConsolidationTrigger,
    pub status: ConsolidationStatus,
    pub attempts: u32,
    pub proposal_count: Option<usize>,
    pub operation_ids: Vec<Uuid>,
    pub last_error_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpisodeStatusView {
    pub episode: Episode,
    pub user_id: String,
    pub project_id: Option<String>,
    pub unconsolidated_turn_count: u64,
    pub consolidations: Vec<ConsolidationView>,
}

#[derive(Debug, Clone)]
pub struct ConsolidationWork {
    pub consolidation_id: Uuid,
    pub episode_id: Uuid,
    pub user_id: String,
    pub project_id: Option<String>,
    pub from_turn_sequence: u64,
    pub through_turn_sequence: u64,
    pub trigger: ConsolidationTrigger,
    pub attempts: u32,
    pub worker_id: String,
    pub turns: Vec<CompletedTurn>,
    pub extractor_version: Option<String>,
    pub proposals: Option<Vec<ExtractCandidateProposal>>,
    pub diagnostics: Option<ExtractionDiagnostics>,
}

#[derive(Debug, Clone)]
pub struct ForgetCommand {
    pub idempotency_key: String,
    pub memory_record_id: Uuid,
    pub expected_revision: i64,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct CorrectionCommand {
    pub idempotency_key: String,
    pub memory_record_id: Uuid,
    pub expected_revision: i64,
    pub new_content: String,
    pub reason: String,
    pub source_ref: String,
    pub effective_from: Option<DateTime<Utc>>,
    pub new_structured_content: Option<StructuredContent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewOutcome {
    Accept,
    Reject,
}

impl ReviewOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Reject => "reject",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewDecisionCommand {
    pub idempotency_key: String,
    pub candidate_id: Uuid,
    pub expected_candidate_revision: i64,
    pub outcome: ReviewOutcome,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErasureTargetKind {
    MemoryRecord,
    User,
    Project,
}

impl ErasureTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MemoryRecord => "memory_record",
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ErasureCommand {
    pub idempotency_key: String,
    pub target_kind: ErasureTargetKind,
    pub target_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErasureRecordView {
    pub memory_record_id: Uuid,
    pub status: String,
    pub projection_operation_id: Option<Uuid>,
    pub last_error_code: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErasureRequestView {
    pub erasure_request_id: Uuid,
    pub target_kind: String,
    pub target_ref: String,
    pub status: String,
    pub record_count: i32,
    pub handler_status: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub records: Vec<ErasureRecordView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalStatus {
    NeedsReview,
    MemoryAccepted,
    MemoryCorrected,
    Expired,
    Forgotten,
    ErasurePending,
    ErasureComplete,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionOperationStatus {
    Pending,
    Leased,
    Unknown,
    Ready,
    Failed,
    Stale,
    Deleted,
    DeadLetter,
}

impl ProjectionOperationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Unknown => "unknown",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Stale => "stale",
            Self::Deleted => "deleted",
            Self::DeadLetter => "dead_letter",
        }
    }
}

impl TryFrom<&str> for ProjectionOperationStatus {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "leased" => Ok(Self::Leased),
            "unknown" => Ok(Self::Unknown),
            "ready" => Ok(Self::Ready),
            "failed" => Ok(Self::Failed),
            "stale" => Ok(Self::Stale),
            "deleted" => Ok(Self::Deleted),
            "dead_letter" => Ok(Self::DeadLetter),
            other => Err(format!("unknown projection status {other:?}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionOperationType {
    Upsert,
    Delete,
}

impl ProjectionOperationType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Upsert => "upsert",
            Self::Delete => "delete",
        }
    }
}

impl TryFrom<&str> for ProjectionOperationType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "upsert" => Ok(Self::Upsert),
            "delete" => Ok(Self::Delete),
            other => Err(format!("unknown projection operation type {other:?}")),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OperationView {
    pub operation_id: Uuid,
    pub candidate_id: Option<Uuid>,
    pub memory_record_id: Uuid,
    pub memory_version_id: Option<Uuid>,
    pub head_revision: i64,
    pub canonical_status: CanonicalStatus,
    pub projection_status: ProjectionOperationStatus,
    pub reason_code: Option<String>,
    pub provider_object_id: Option<String>,
    pub attempts: i32,
}

#[derive(Debug, Clone)]
pub struct ProjectionWork {
    pub outbox_id: Uuid,
    pub operation_id: Uuid,
    pub operation_type: ProjectionOperationType,
    pub prior_status: ProjectionOperationStatus,
    pub memory_record_id: Uuid,
    pub memory_version_id: Option<Uuid>,
    pub expected_head_revision: i64,
    pub current_head_revision: i64,
    pub current_head_state: String,
    pub current_version_id: Option<Uuid>,
    pub generation: String,
    pub model_manifest_hash: String,
    pub namespace_key: String,
    pub content: Option<String>,
    pub content_hash: Option<String>,
    pub provider_object_id: Option<String>,
    pub attempts: i32,
    pub lease_version: i64,
    pub lease_owner: String,
}

#[derive(Debug, Clone)]
pub struct ProjectionRebuildCommand {
    pub idempotency_key: String,
    pub generation: String,
    pub model_manifest: ProjectionModelManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionModelManifest {
    pub index_provider: String,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_revision: String,
    pub embedding_dimensions: i32,
    pub embedding_normalization: String,
    pub embedding_instruction_version: String,
    pub semantic_embedding: bool,
    pub manifest_hash: String,
}

impl ProjectionModelManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index_provider: String,
        embedding_provider: String,
        embedding_model: String,
        embedding_revision: String,
        embedding_dimensions: i32,
        embedding_normalization: String,
        embedding_instruction_version: String,
        semantic_embedding: bool,
    ) -> Result<Self, String> {
        let fields = [
            ("index_provider", index_provider.as_str(), 128usize),
            ("embedding_provider", embedding_provider.as_str(), 128usize),
            ("embedding_model", embedding_model.as_str(), 256usize),
            ("embedding_revision", embedding_revision.as_str(), 256usize),
            (
                "embedding_instruction_version",
                embedding_instruction_version.as_str(),
                256usize,
            ),
        ];
        for (name, value, max_len) in fields {
            if value.is_empty()
                || value.len() > max_len
                || value.trim() != value
                || value.chars().any(char::is_control)
            {
                return Err(format!(
                    "{name} must be trimmed, non-empty, bounded, and contain no control characters"
                ));
            }
        }
        if !(1..=4096).contains(&embedding_dimensions) {
            return Err("embedding_dimensions must be between 1 and 4096".into());
        }
        if !matches!(
            embedding_normalization.as_str(),
            "l2" | "none" | "provider_default"
        ) {
            return Err("embedding_normalization must be l2, none, or provider_default".into());
        }
        let canonical = [
            index_provider.as_str(),
            embedding_provider.as_str(),
            embedding_model.as_str(),
            embedding_revision.as_str(),
            &embedding_dimensions.to_string(),
            embedding_normalization.as_str(),
            embedding_instruction_version.as_str(),
            if semantic_embedding { "true" } else { "false" },
        ]
        .into_iter()
        .map(|value| format!("{}:{value}", value.len()))
        .collect::<Vec<_>>()
        .join("|");
        let manifest_hash = hex::encode(Sha256::digest(canonical.as_bytes()));
        Ok(Self {
            index_provider,
            embedding_provider,
            embedding_model,
            embedding_revision,
            embedding_dimensions,
            embedding_normalization,
            embedding_instruction_version,
            semantic_embedding,
            manifest_hash,
        })
    }

    pub fn validate_integrity(&self) -> Result<(), String> {
        let rebuilt = Self::new(
            self.index_provider.clone(),
            self.embedding_provider.clone(),
            self.embedding_model.clone(),
            self.embedding_revision.clone(),
            self.embedding_dimensions,
            self.embedding_normalization.clone(),
            self.embedding_instruction_version.clone(),
            self.semantic_embedding,
        )?;
        if rebuilt.manifest_hash != self.manifest_hash {
            return Err("projection model manifest hash does not match its fields".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionGenerationView {
    pub generation: String,
    pub status: String,
    pub source_generation: Option<String>,
    pub model_manifest: ProjectionModelManifest,
    pub expected_records: i32,
    pub ready_records: i32,
    pub failed_records: i32,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub activated_at: Option<DateTime<Utc>>,
    pub retired_at: Option<DateTime<Utc>>,
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorizedMemory {
    pub memory_record_id: Uuid,
    pub memory_version_id: Uuid,
    pub content: String,
    pub memory_type: String,
    pub score: f64,
    pub source_refs: Vec<String>,
    pub effective_from: Option<DateTime<Utc>>,
    pub event_time: Option<EventTime>,
}

#[derive(Debug, Clone)]
pub struct RetrievalCandidateReceiptCommand {
    pub scope_kind: String,
    pub scope_id: Option<String>,
    pub channel: String,
    pub memory_record_id: Uuid,
    pub memory_version_id: Uuid,
    pub channel_rank: i32,
    pub raw_score: f64,
    pub channel_weight: f64,
    pub rrf_contribution: f64,
    pub fused_score: Option<f64>,
    pub preauthorized: bool,
    pub rerank_score: Option<f64>,
    pub postauthorized: bool,
    pub final_rank: Option<i32>,
    pub included: bool,
    pub exclusion_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RetrievalRunCommand {
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
    pub query_hash: String,
    pub requested_limit: i32,
    pub candidate_limit: i32,
    pub reference_time: DateTime<Utc>,
    pub timezone: String,
    pub temporal_window: Option<TemporalWindow>,
    pub projection_generation: String,
    pub ranking_version: String,
    pub degraded: bool,
    pub degradation_reason: Option<String>,
    pub candidates: Vec<RetrievalCandidateReceiptCommand>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrievalCandidateTraceView {
    pub scope_kind: String,
    pub scope_id: Option<String>,
    pub channel: String,
    pub memory_record_id: Uuid,
    pub memory_version_id: Uuid,
    pub channel_rank: i32,
    pub raw_score: f64,
    pub channel_weight: f64,
    pub rrf_contribution: f64,
    pub fused_score: Option<f64>,
    pub preauthorized: bool,
    pub rerank_score: Option<f64>,
    pub postauthorized: bool,
    pub final_rank: Option<i32>,
    pub included: bool,
    pub exclusion_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrievalRunView {
    pub retrieval_run_id: Uuid,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
    pub query_hash: String,
    pub requested_limit: i32,
    pub candidate_limit: i32,
    pub reference_time: DateTime<Utc>,
    pub timezone: String,
    pub temporal_window: Option<TemporalWindow>,
    pub projection_generation: String,
    pub ranking_version: String,
    pub degraded: bool,
    pub degradation_reason: Option<String>,
    pub candidates: Vec<RetrievalCandidateTraceView>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryScopeView {
    pub kind: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryVersionView {
    pub memory_version_id: Uuid,
    pub version_no: i32,
    pub content: Option<String>,
    pub content_hash: String,
    pub normalized_key: Option<String>,
    pub confidence: f64,
    pub importance: f64,
    pub inference_kind: String,
    pub source_refs: Vec<String>,
    pub effective_from: Option<DateTime<Utc>>,
    pub effective_to: Option<DateTime<Utc>>,
    pub event_time: Option<EventTime>,
    pub structured_content: Option<StructuredContent>,
    pub classification: String,
    pub allowed_purposes: Vec<String>,
    pub consent_ref: Option<String>,
    pub retention_policy_id: String,
    pub retain_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub content_erased_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryEventView {
    pub sequence_no: i64,
    pub event_type: String,
    pub memory_version_id: Option<Uuid>,
    pub actor_type: String,
    pub actor_id: String,
    pub reason_code: String,
    pub payload: serde_json::Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryRecordView {
    pub memory_record_id: Uuid,
    pub scope: MemoryScopeView,
    pub memory_type: String,
    pub semantic_key: String,
    pub head_state: String,
    pub revision: i64,
    pub current_version_id: Option<Uuid>,
    pub identity_retired_at: Option<DateTime<Utc>>,
    pub versions: Vec<MemoryVersionView>,
    pub events: Vec<MemoryEventView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryExportView {
    pub format_version: String,
    pub generated_at: DateTime<Utc>,
    pub record_count: usize,
    pub records: Vec<MemoryRecordView>,
}

#[derive(Debug, Clone)]
pub struct ContextPackCommand {
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
    pub query: String,
    pub limit: usize,
    pub memory_types: Vec<String>,
    pub ranking_version: String,
    pub projection_generation: String,
    pub token_budget: i32,
    pub tokenizer_id: String,
    pub memory_required: bool,
    pub pinned_memory_version_ids: Vec<Uuid>,
    pub temporal_window_hash: Option<String>,
    pub reference_time: DateTime<Utc>,
    pub timezone: String,
}

#[derive(Debug, Clone)]
pub struct ConsumeContextPackCommand {
    pub idempotency_key: String,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
}

#[derive(Debug, Clone)]
pub struct OutcomeReceiptCommand {
    pub idempotency_key: String,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
    pub outcome: String,
    pub model_id: String,
    pub model_request_id: Option<String>,
    pub latency_ms: i64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutcomeReceiptView {
    pub outcome_receipt_id: Uuid,
    pub context_pack_id: Uuid,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
    pub outcome: String,
    pub model_id: String,
    pub model_request_id_hash: Option<String>,
    pub latency_ms: i64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub error_code: Option<String>,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextPackManifestView {
    pub context_pack_id: Uuid,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
    pub authority_snapshot_id: String,
    pub policy_version: String,
    pub compiler_version: String,
    pub ranking_version: String,
    pub projection_generation: String,
    pub reference_time: DateTime<Utc>,
    pub timezone: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub tokenizer_id: String,
    pub token_budget: i32,
    pub estimated_tokens: i32,
    pub item_count: i32,
    pub delivery_status: String,
    pub consumed_at: Option<DateTime<Utc>>,
    pub omission_summary: BTreeMap<String, i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextPackItem {
    pub memory_record_id: Uuid,
    pub memory_version_id: Uuid,
    pub text: String,
    pub source_refs: Vec<String>,
    pub effective_from: Option<DateTime<Utc>>,
    pub event_time: Option<EventTime>,
    pub inclusion_reason: String,
    pub trust: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextSection {
    pub kind: String,
    pub items: Vec<ContextPackItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextPackView {
    pub context_pack_id: Uuid,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
    pub authority_snapshot_id: String,
    pub policy_version: String,
    pub compiler_version: String,
    pub ranking_version: String,
    pub projection_generation: String,
    pub reference_time: DateTime<Utc>,
    pub timezone: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: DateTime<Utc>,
    pub tokenizer_id: String,
    pub token_budget: i32,
    pub estimated_tokens: i32,
    pub sections: Vec<ContextSection>,
    pub omission_summary: BTreeMap<String, i32>,
}
