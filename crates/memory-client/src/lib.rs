use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use memory_domain::{CANONICAL_MEMORY_TYPE_NAMES, EventTime, MemoryType, StructuredContent};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::redirect::Policy;
use reqwest::{Response, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use tokio::time::sleep;
use uuid::Uuid;

const MAX_REMOTE_ERROR_BYTES: usize = 4_096;

#[derive(Debug, Error)]
pub enum MemoryClientError {
    #[error("invalid Memory client configuration: {0}")]
    Config(String),
    #[error("Memory Service HTTP client setup failed: {0}")]
    Client(#[source] reqwest::Error),
    #[error("Memory Service transport failed: {0}")]
    Transport(#[source] reqwest::Error),
    #[error(
        "Memory Service rejected the request with HTTP {status}; code={code}; request_id={request_id}; retryable={retryable}"
    )]
    Rejected {
        status: StatusCode,
        code: String,
        request_id: String,
        retryable: bool,
    },
    #[error("Memory Service returned an invalid response: {0}")]
    InvalidResponse(String),
    #[error("Memory Service contract violation: {0}")]
    Contract(String),
}

pub struct MemoryClientConfig {
    pub base_url: String,
    pub timeout: Duration,
    pub bearer_token: Option<String>,
    pub transport_retries: u8,
}

impl MemoryClientConfig {
    pub fn loopback(base_url: impl Into<String>, timeout: Duration) -> Self {
        Self {
            base_url: base_url.into(),
            timeout,
            bearer_token: None,
            transport_retries: 1,
        }
    }

    pub fn authenticated(
        base_url: impl Into<String>,
        timeout: Duration,
        bearer_token: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            timeout,
            bearer_token: Some(bearer_token.into()),
            transport_retries: 1,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ScopeSelector {
    pub kind: String,
    pub id: Option<String>,
}

impl ScopeSelector {
    pub fn tenant() -> Self {
        Self {
            kind: "tenant".into(),
            id: None,
        }
    }

    pub fn user(id: impl Into<String>) -> Self {
        Self {
            kind: "user".into(),
            id: Some(id.into()),
        }
    }

    pub fn project(id: impl Into<String>) -> Self {
        Self {
            kind: "project".into(),
            id: Some(id.into()),
        }
    }

    pub fn agent(id: impl Into<String>) -> Self {
        Self {
            kind: "agent".into(),
            id: Some(id.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskContext {
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
}

#[derive(Debug, Clone)]
pub struct SourceEventCommand {
    pub idempotency_key: String,
    pub event_id: String,
    pub source_type: String,
    pub source_ref: String,
    pub content: String,
    pub occurred_at: DateTime<Utc>,
    pub scope_selector: Option<ScopeSelector>,
    pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone)]
pub struct MemoryExtractionCommand {
    pub idempotency_key: String,
    pub event_id: String,
    pub source_type: String,
    pub source_ref: String,
    pub content: String,
    pub occurred_at: DateTime<Utc>,
    pub timezone: String,
    pub max_candidates: usize,
    pub scope_selector: Option<ScopeSelector>,
    /// Optional project scope used only for project-scoped `semantic_fact` proposals. Personal
    /// memories continue to use `scope_selector`.
    pub project_scope_selector: Option<ScopeSelector>,
}

#[derive(Debug, Clone)]
pub struct ContextPackCommand {
    pub task_context: TaskContext,
    pub query: String,
    pub reference_time: Option<DateTime<Utc>>,
    pub timezone: Option<String>,
    pub scope_selectors: Vec<ScopeSelector>,
    pub memory_types: Vec<String>,
    pub limit: usize,
    pub token_budget: i32,
    pub tokenizer_id: String,
    pub memory_required: bool,
    pub pinned_memory_version_ids: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct ConsumeContextPackCommand {
    pub context_pack_id: Uuid,
    pub task_context: TaskContext,
    pub idempotency_key: String,
}

#[derive(Debug, Clone)]
pub struct ForgetCommand {
    pub memory_record_id: Uuid,
    pub idempotency_key: String,
    pub expected_revision: i64,
    pub reason: String,
    pub request_erasure: bool,
}

#[derive(Debug, Clone)]
pub struct CorrectionCommand {
    pub memory_record_id: Uuid,
    pub idempotency_key: String,
    pub expected_revision: i64,
    pub new_content: String,
    pub reason: String,
    pub source_ref: String,
    pub effective_from: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ReviewDecisionCommand {
    pub candidate_id: Uuid,
    pub idempotency_key: String,
    pub expected_candidate_revision: i64,
    pub outcome: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ErasureCommand {
    pub idempotency_key: String,
    pub target_kind: String,
    pub target_id: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct OutcomeReceiptCommand {
    pub context_pack_id: Uuid,
    pub task_context: TaskContext,
    pub idempotency_key: String,
    pub outcome: String,
    pub model_id: String,
    pub model_request_id: Option<String>,
    pub latency_ms: i64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OperationResponse {
    pub operation_id: Uuid,
    #[serde(default)]
    pub candidate_ids: Vec<Uuid>,
    pub memory_record_id: Uuid,
    pub memory_version_id: Option<Uuid>,
    pub head_revision: i64,
    pub canonical_status: String,
    pub projection_status: String,
    pub reason_code: Option<String>,
    pub provider_object_id: Option<String>,
    pub attempts: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtractedCandidateProposal {
    pub memory_type: String,
    pub semantic_key: String,
    pub content: String,
    pub confidence: f64,
    pub importance: f64,
    pub event_time: Option<EventTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<StructuredContent>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtractedMemoryResult {
    pub proposal: ExtractedCandidateProposal,
    pub operation: OperationResponse,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryExtractionResponse {
    pub extractor_version: String,
    pub candidate_count: usize,
    pub candidates: Vec<ExtractedMemoryResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContextPackItem {
    pub memory_record_id: Uuid,
    pub memory_version_id: Uuid,
    pub text: String,
    pub source_refs: Vec<String>,
    pub effective_from: Option<DateTime<Utc>>,
    pub inclusion_reason: String,
    pub trust: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContextSection {
    pub kind: String,
    pub items: Vec<ContextPackItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContextPackManifestResponse {
    pub context_pack_id: Uuid,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
    pub authority_snapshot_id: String,
    pub policy_version: String,
    pub compiler_version: String,
    pub ranking_version: String,
    pub projection_generation: String,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContextPackResponse {
    pub context_pack_id: Uuid,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub purpose: String,
    pub authority_snapshot_id: String,
    pub policy_version: String,
    pub compiler_version: String,
    pub ranking_version: String,
    pub projection_generation: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: DateTime<Utc>,
    pub tokenizer_id: String,
    pub token_budget: i32,
    pub estimated_tokens: i32,
    pub sections: Vec<ContextSection>,
    pub omission_summary: BTreeMap<String, i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutcomeReceiptResponse {
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryScopeView {
    pub kind: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryEventView {
    pub sequence_no: i64,
    pub event_type: String,
    pub memory_version_id: Option<Uuid>,
    pub actor_type: String,
    pub actor_id: String,
    pub reason_code: String,
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryExportView {
    pub format_version: String,
    pub generated_at: DateTime<Utc>,
    pub record_count: usize,
    pub records: Vec<MemoryRecordView>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErasureRecordView {
    pub memory_record_id: Uuid,
    pub status: String,
    pub projection_operation_id: Option<Uuid>,
    pub last_error_code: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErasureRequestView {
    pub erasure_request_id: Uuid,
    pub target_kind: String,
    pub target_ref: String,
    pub status: String,
    pub record_count: i32,
    pub handler_status: Value,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub records: Vec<ErasureRecordView>,
}

#[async_trait]
pub trait MemoryPort: Send + Sync {
    async fn ready(&self) -> Result<(), MemoryClientError>;
    async fn submit_event(
        &self,
        command: &SourceEventCommand,
    ) -> Result<OperationResponse, MemoryClientError>;
    async fn extract_memories(
        &self,
        command: &MemoryExtractionCommand,
    ) -> Result<MemoryExtractionResponse, MemoryClientError> {
        let _ = command;
        Err(MemoryClientError::Contract(
            "this MemoryPort does not implement automatic extraction".into(),
        ))
    }
    async fn operation(&self, operation_id: Uuid) -> Result<OperationResponse, MemoryClientError>;
    async fn compile_context_pack(
        &self,
        command: &ContextPackCommand,
    ) -> Result<ContextPackManifestResponse, MemoryClientError>;
    async fn consume_context_pack(
        &self,
        command: &ConsumeContextPackCommand,
    ) -> Result<ContextPackResponse, MemoryClientError>;
    async fn forget(&self, command: &ForgetCommand)
    -> Result<OperationResponse, MemoryClientError>;
    async fn export_memories(&self) -> Result<MemoryExportView, MemoryClientError> {
        Err(MemoryClientError::Contract(
            "this MemoryPort does not implement memory export".into(),
        ))
    }
    async fn inspect_memory(
        &self,
        memory_record_id: Uuid,
    ) -> Result<MemoryRecordView, MemoryClientError> {
        let _ = memory_record_id;
        Err(MemoryClientError::Contract(
            "this MemoryPort does not implement memory inspection".into(),
        ))
    }
    async fn correct(
        &self,
        command: &CorrectionCommand,
    ) -> Result<OperationResponse, MemoryClientError> {
        let _ = command;
        Err(MemoryClientError::Contract(
            "this MemoryPort does not implement memory correction".into(),
        ))
    }
    async fn decide_candidate(
        &self,
        command: &ReviewDecisionCommand,
    ) -> Result<OperationResponse, MemoryClientError> {
        let _ = command;
        Err(MemoryClientError::Contract(
            "this MemoryPort does not implement candidate review".into(),
        ))
    }
    async fn create_erasure_request(
        &self,
        command: &ErasureCommand,
    ) -> Result<ErasureRequestView, MemoryClientError> {
        let _ = command;
        Err(MemoryClientError::Contract(
            "this MemoryPort does not implement Erase".into(),
        ))
    }
    async fn erasure_request(
        &self,
        erasure_request_id: Uuid,
    ) -> Result<ErasureRequestView, MemoryClientError> {
        let _ = erasure_request_id;
        Err(MemoryClientError::Contract(
            "this MemoryPort does not implement Erase status".into(),
        ))
    }
    async fn record_outcome(
        &self,
        command: &OutcomeReceiptCommand,
    ) -> Result<OutcomeReceiptResponse, MemoryClientError>;

    async fn acquire_context_pack(
        &self,
        command: &ContextPackCommand,
        consume_idempotency_key: &str,
    ) -> Result<ContextPackResponse, MemoryClientError> {
        let manifest = self.compile_context_pack(command).await?;
        validate_manifest(&manifest, command)?;
        let pack = self
            .consume_context_pack(&ConsumeContextPackCommand {
                context_pack_id: manifest.context_pack_id,
                task_context: command.task_context.clone(),
                idempotency_key: consume_idempotency_key.to_owned(),
            })
            .await?;
        validate_consumed_pack(&manifest, &pack, command.memory_required)?;
        Ok(pack)
    }
}

#[derive(Clone)]
pub struct MemoryHttpClient {
    base_url: String,
    client: reqwest::Client,
    transport_retries: u8,
}

impl MemoryHttpClient {
    pub fn new(config: MemoryClientConfig) -> Result<Self, MemoryClientError> {
        if config.timeout.is_zero() || config.timeout > Duration::from_secs(300) {
            return Err(MemoryClientError::Config(
                "timeout must be greater than zero and no more than 300 seconds".into(),
            ));
        }
        if config.transport_retries > 3 {
            return Err(MemoryClientError::Config(
                "transport_retries must be between 0 and 3".into(),
            ));
        }

        let url = parse_base_url(&config.base_url)?;
        let loopback = is_loopback(&url);
        if !loopback && url.scheme() != "https" {
            return Err(MemoryClientError::Config(
                "a non-loopback Memory Service must use HTTPS".into(),
            ));
        }
        if !loopback && config.bearer_token.is_none() {
            return Err(MemoryClientError::Config(
                "a non-loopback Memory Service requires a bearer token".into(),
            ));
        }

        let mut headers = HeaderMap::new();
        if let Some(token) = config.bearer_token {
            if token.trim().is_empty() {
                return Err(MemoryClientError::Config(
                    "bearer token must not be empty".into(),
                ));
            }
            let mut value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                MemoryClientError::Config("bearer token is not a valid HTTP header value".into())
            })?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(Policy::none())
            .timeout(config.timeout)
            .user_agent(concat!("lazytt-memory-client/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(MemoryClientError::Client)?;
        Ok(Self {
            base_url: url.as_str().trim_end_matches('/').to_owned(),
            client,
            transport_retries: config.transport_retries,
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, MemoryClientError> {
        for attempt in 0..=self.transport_retries {
            let response = self.client.get(self.endpoint(path)).send().await;
            match response {
                Ok(response) => {
                    let response = require_success(response).await?;
                    return response
                        .json()
                        .await
                        .map_err(|error| MemoryClientError::InvalidResponse(error.to_string()));
                }
                Err(_) if attempt < self.transport_retries => retry_delay(attempt).await,
                Err(error) => return Err(MemoryClientError::Transport(error)),
            }
        }
        unreachable!("bounded GET retry loop always returns")
    }

    async fn post_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        idempotency_key: Option<&str>,
        body: &B,
        repeatable: bool,
    ) -> Result<T, MemoryClientError> {
        if let Some(key) = idempotency_key {
            validate_idempotency_key(key)?;
        }
        let retries = if repeatable {
            self.transport_retries
        } else {
            0
        };
        for attempt in 0..=retries {
            let mut request = self.client.post(self.endpoint(path)).json(body);
            if let Some(key) = idempotency_key {
                request = request.header("idempotency-key", key);
            }
            match request.send().await {
                Ok(response) => {
                    let response = require_success(response).await?;
                    return response
                        .json()
                        .await
                        .map_err(|error| MemoryClientError::InvalidResponse(error.to_string()));
                }
                Err(_) if attempt < retries => retry_delay(attempt).await,
                Err(error) => return Err(MemoryClientError::Transport(error)),
            }
        }
        unreachable!("bounded POST retry loop always returns")
    }
}

#[derive(Serialize)]
struct SourceEventRequest<'a> {
    event_id: &'a str,
    source_type: &'a str,
    source_ref: &'a str,
    content: &'a str,
    occurred_at: DateTime<Utc>,
    scope_selector: &'a Option<ScopeSelector>,
    metadata: &'a Option<Map<String, Value>>,
}

#[derive(Serialize)]
struct MemoryExtractionRequest<'a> {
    event_id: &'a str,
    source_type: &'a str,
    source_ref: &'a str,
    content: &'a str,
    occurred_at: DateTime<Utc>,
    timezone: &'a str,
    max_candidates: usize,
    scope_selector: &'a Option<ScopeSelector>,
    project_scope_selector: &'a Option<ScopeSelector>,
}

#[derive(Serialize)]
struct ContextPackRequest<'a> {
    task_context: &'a TaskContext,
    query: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_time: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timezone: Option<&'a str>,
    scope_selectors: &'a [ScopeSelector],
    memory_types: &'a [String],
    limit: usize,
    token_budget: i32,
    tokenizer_id: &'a str,
    memory_required: bool,
    pinned_memory_version_ids: &'a [Uuid],
}

#[derive(Serialize)]
struct ConsumeContextPackRequest<'a> {
    task_context: &'a TaskContext,
}

#[derive(Serialize)]
struct ForgetRequest<'a> {
    expected_revision: i64,
    reason: &'a str,
    request_erasure: bool,
}

#[derive(Serialize)]
struct CorrectionRequest<'a> {
    expected_revision: i64,
    new_content: &'a str,
    reason: &'a str,
    source_ref: &'a str,
    effective_from: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct ReviewDecisionRequest<'a> {
    expected_candidate_revision: i64,
    outcome: &'a str,
    reason: &'a str,
}

#[derive(Serialize)]
struct ErasureRequest<'a> {
    target_kind: &'a str,
    target_id: &'a str,
    reason: &'a str,
}

#[derive(Serialize)]
struct OutcomeReceiptRequest<'a> {
    task_context: &'a TaskContext,
    outcome: &'a str,
    model_id: &'a str,
    model_request_id: &'a Option<String>,
    latency_ms: i64,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    total_tokens: Option<i64>,
    error_code: &'a Option<String>,
}

#[async_trait]
impl MemoryPort for MemoryHttpClient {
    async fn ready(&self) -> Result<(), MemoryClientError> {
        for attempt in 0..=self.transport_retries {
            match self.client.get(self.endpoint("/health/ready")).send().await {
                Ok(response) => {
                    require_success(response).await?;
                    return Ok(());
                }
                Err(_) if attempt < self.transport_retries => retry_delay(attempt).await,
                Err(error) => return Err(MemoryClientError::Transport(error)),
            }
        }
        unreachable!("bounded readiness retry loop always returns")
    }

    async fn submit_event(
        &self,
        command: &SourceEventCommand,
    ) -> Result<OperationResponse, MemoryClientError> {
        self.post_json(
            "/v1/memory/events",
            Some(&command.idempotency_key),
            &SourceEventRequest {
                event_id: &command.event_id,
                source_type: &command.source_type,
                source_ref: &command.source_ref,
                content: &command.content,
                occurred_at: command.occurred_at,
                scope_selector: &command.scope_selector,
                metadata: &command.metadata,
            },
            true,
        )
        .await
    }

    async fn extract_memories(
        &self,
        command: &MemoryExtractionCommand,
    ) -> Result<MemoryExtractionResponse, MemoryClientError> {
        if !(1..=8).contains(&command.max_candidates)
            || command.timezone.is_empty()
            || command.timezone.len() > 64
            || command.timezone.trim() != command.timezone
            || command.timezone.parse::<chrono_tz::Tz>().is_err()
        {
            return Err(MemoryClientError::Contract(
                "extraction requires max_candidates between 1 and 8 and a valid IANA timezone"
                    .into(),
            ));
        }
        let response: MemoryExtractionResponse = self
            .post_json(
                "/v1/memory/extractions",
                Some(&command.idempotency_key),
                &MemoryExtractionRequest {
                    event_id: &command.event_id,
                    source_type: &command.source_type,
                    source_ref: &command.source_ref,
                    content: &command.content,
                    occurred_at: command.occurred_at,
                    timezone: &command.timezone,
                    max_candidates: command.max_candidates,
                    scope_selector: &command.scope_selector,
                    project_scope_selector: &command.project_scope_selector,
                },
                true,
            )
            .await?;
        validate_extraction_response(&response, command.max_candidates, &command.timezone)?;
        Ok(response)
    }

    async fn operation(&self, operation_id: Uuid) -> Result<OperationResponse, MemoryClientError> {
        self.get_json(&format!("/v1/memory/operations/{operation_id}"))
            .await
    }

    async fn compile_context_pack(
        &self,
        command: &ContextPackCommand,
    ) -> Result<ContextPackManifestResponse, MemoryClientError> {
        self.post_json(
            "/v1/memory/context-packs",
            None,
            &ContextPackRequest {
                task_context: &command.task_context,
                query: &command.query,
                reference_time: command.reference_time,
                timezone: command.timezone.as_deref(),
                scope_selectors: &command.scope_selectors,
                memory_types: &command.memory_types,
                limit: command.limit,
                token_budget: command.token_budget,
                tokenizer_id: &command.tokenizer_id,
                memory_required: command.memory_required,
                pinned_memory_version_ids: &command.pinned_memory_version_ids,
            },
            true,
        )
        .await
    }

    async fn consume_context_pack(
        &self,
        command: &ConsumeContextPackCommand,
    ) -> Result<ContextPackResponse, MemoryClientError> {
        self.post_json(
            &format!(
                "/v1/memory/context-packs/{}/consume",
                command.context_pack_id
            ),
            Some(&command.idempotency_key),
            &ConsumeContextPackRequest {
                task_context: &command.task_context,
            },
            true,
        )
        .await
    }

    async fn forget(
        &self,
        command: &ForgetCommand,
    ) -> Result<OperationResponse, MemoryClientError> {
        self.post_json(
            &format!("/v1/memories/{}/forget", command.memory_record_id),
            Some(&command.idempotency_key),
            &ForgetRequest {
                expected_revision: command.expected_revision,
                reason: &command.reason,
                request_erasure: command.request_erasure,
            },
            true,
        )
        .await
    }

    async fn export_memories(&self) -> Result<MemoryExportView, MemoryClientError> {
        self.get_json("/v1/memory/export").await
    }

    async fn inspect_memory(
        &self,
        memory_record_id: Uuid,
    ) -> Result<MemoryRecordView, MemoryClientError> {
        self.get_json(&format!("/v1/memories/{memory_record_id}"))
            .await
    }

    async fn correct(
        &self,
        command: &CorrectionCommand,
    ) -> Result<OperationResponse, MemoryClientError> {
        self.post_json(
            &format!("/v1/memories/{}/corrections", command.memory_record_id),
            Some(&command.idempotency_key),
            &CorrectionRequest {
                expected_revision: command.expected_revision,
                new_content: &command.new_content,
                reason: &command.reason,
                source_ref: &command.source_ref,
                effective_from: command.effective_from,
            },
            true,
        )
        .await
    }

    async fn decide_candidate(
        &self,
        command: &ReviewDecisionCommand,
    ) -> Result<OperationResponse, MemoryClientError> {
        self.post_json(
            &format!("/v1/memory-candidates/{}/decisions", command.candidate_id),
            Some(&command.idempotency_key),
            &ReviewDecisionRequest {
                expected_candidate_revision: command.expected_candidate_revision,
                outcome: &command.outcome,
                reason: &command.reason,
            },
            true,
        )
        .await
    }

    async fn create_erasure_request(
        &self,
        command: &ErasureCommand,
    ) -> Result<ErasureRequestView, MemoryClientError> {
        self.post_json(
            "/v1/erasure-requests",
            Some(&command.idempotency_key),
            &ErasureRequest {
                target_kind: &command.target_kind,
                target_id: &command.target_id,
                reason: &command.reason,
            },
            true,
        )
        .await
    }

    async fn erasure_request(
        &self,
        erasure_request_id: Uuid,
    ) -> Result<ErasureRequestView, MemoryClientError> {
        self.get_json(&format!("/v1/erasure-requests/{erasure_request_id}"))
            .await
    }

    async fn record_outcome(
        &self,
        command: &OutcomeReceiptCommand,
    ) -> Result<OutcomeReceiptResponse, MemoryClientError> {
        self.post_json(
            &format!(
                "/v1/memory/context-packs/{}/outcomes",
                command.context_pack_id
            ),
            Some(&command.idempotency_key),
            &OutcomeReceiptRequest {
                task_context: &command.task_context,
                outcome: &command.outcome,
                model_id: &command.model_id,
                model_request_id: &command.model_request_id,
                latency_ms: command.latency_ms,
                prompt_tokens: command.prompt_tokens,
                completion_tokens: command.completion_tokens,
                total_tokens: command.total_tokens,
                error_code: &command.error_code,
            },
            true,
        )
        .await
    }
}

fn parse_base_url(raw: &str) -> Result<Url, MemoryClientError> {
    let url = Url::parse(raw)
        .map_err(|error| MemoryClientError::Config(format!("base URL is invalid: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(MemoryClientError::Config(
            "base URL must use HTTP or HTTPS".into(),
        ));
    }
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(MemoryClientError::Config(
            "base URL must have a host and must not contain credentials, query or fragment".into(),
        ));
    }
    Ok(url)
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn validate_idempotency_key(key: &str) -> Result<(), MemoryClientError> {
    if !(16..=128).contains(&key.len())
        || key.trim() != key
        || key.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(MemoryClientError::Contract(
            "Idempotency-Key must contain 16 to 128 visible characters".into(),
        ));
    }
    Ok(())
}

fn validate_extraction_response(
    response: &MemoryExtractionResponse,
    max_candidates: usize,
    timezone: &str,
) -> Result<(), MemoryClientError> {
    if response.extractor_version.trim().is_empty()
        || response.extractor_version.len() > 256
        || response.candidate_count != response.candidates.len()
        || response.candidates.len() > max_candidates
    {
        return Err(MemoryClientError::InvalidResponse(
            "extraction metadata or candidate count was invalid".into(),
        ));
    }
    let mut identities = std::collections::HashSet::new();
    for result in &response.candidates {
        let proposal = &result.proposal;
        if !CANONICAL_MEMORY_TYPE_NAMES.contains(&proposal.memory_type.as_str())
            || !(3..=128).contains(&proposal.semantic_key.len())
            || !proposal.semantic_key.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
            || proposal.content.trim().is_empty()
            || proposal.content.trim() != proposal.content
            || proposal.content.len() > 2_048
            || !proposal.confidence.is_finite()
            || !(if proposal.memory_type == MemoryType::SemanticFact.as_str() {
                0.90
            } else {
                0.80
            }..=1.0)
                .contains(&proposal.confidence)
            || !proposal.importance.is_finite()
            || !(0.0..=1.0).contains(&proposal.importance)
            || proposal.event_time.as_ref().is_some_and(|event_time| {
                proposal.memory_type != MemoryType::EpisodicSummary.as_str()
                    || event_time.timezone != timezone
                    || event_time.validate().is_err()
            })
            || proposal
                .structured_content
                .as_ref()
                .is_some_and(|structured| {
                    canonical_memory_type(&proposal.memory_type)
                        .is_none_or(|memory_type| structured.validate_for(memory_type).is_err())
                })
            || !identities.insert((
                proposal.memory_type.as_str(),
                proposal.semantic_key.as_str(),
            ))
        {
            return Err(MemoryClientError::InvalidResponse(
                "extraction candidate violated the typed proposal contract".into(),
            ));
        }
    }
    Ok(())
}

fn canonical_memory_type(value: &str) -> Option<MemoryType> {
    match value {
        "preference" => Some(MemoryType::Preference),
        "profile_fact" => Some(MemoryType::ProfileFact),
        "semantic_fact" => Some(MemoryType::SemanticFact),
        "episodic_summary" => Some(MemoryType::EpisodicSummary),
        _ => None,
    }
}

fn validate_manifest(
    manifest: &ContextPackManifestResponse,
    command: &ContextPackCommand,
) -> Result<(), MemoryClientError> {
    let valid_consumed_at = manifest.consumed_at.is_none_or(|consumed_at| {
        consumed_at >= manifest.created_at && consumed_at < manifest.expires_at
    });
    if manifest.task_id != command.task_context.task_id
        || manifest.attempt_id != command.task_context.attempt_id
        || manifest.purpose != command.task_context.purpose
        || manifest.tokenizer_id != command.tokenizer_id
        || manifest.token_budget != command.token_budget
        || manifest.ranking_version.trim().is_empty()
        || manifest.projection_generation.trim().is_empty()
        || manifest.created_at >= manifest.expires_at
        || manifest.expires_at <= Utc::now()
        || manifest.estimated_tokens < 0
        || manifest.estimated_tokens > manifest.token_budget
        || manifest.item_count < 0
        || !valid_consumed_at
        || !matches!(manifest.delivery_status.as_str(), "compiled" | "consumed")
    {
        return Err(MemoryClientError::Contract(format!(
            "Context Pack manifest {} does not match the requested Attempt or compiler contract",
            manifest.context_pack_id
        )));
    }
    if command.memory_required && manifest.item_count == 0 {
        return Err(MemoryClientError::Contract(format!(
            "required Context Pack {} compiled no memory",
            manifest.context_pack_id
        )));
    }
    Ok(())
}

fn validate_consumed_pack(
    manifest: &ContextPackManifestResponse,
    pack: &ContextPackResponse,
    memory_required: bool,
) -> Result<(), MemoryClientError> {
    let item_count = pack
        .sections
        .iter()
        .map(|section| section.items.len())
        .sum::<usize>();
    let items_are_safe = pack
        .sections
        .iter()
        .flat_map(|section| &section.items)
        .all(|item| item.trust == "untrusted_memory_data" && !item.source_refs.is_empty());
    if pack.context_pack_id != manifest.context_pack_id
        || pack.task_id != manifest.task_id
        || pack.attempt_id != manifest.attempt_id
        || pack.purpose != manifest.purpose
        || pack.authority_snapshot_id != manifest.authority_snapshot_id
        || pack.policy_version != manifest.policy_version
        || pack.compiler_version != manifest.compiler_version
        || pack.ranking_version != manifest.ranking_version
        || pack.projection_generation != manifest.projection_generation
        || pack.created_at != manifest.created_at
        || pack.expires_at != manifest.expires_at
        || pack.tokenizer_id != manifest.tokenizer_id
        || pack.token_budget != manifest.token_budget
        || pack.estimated_tokens != manifest.estimated_tokens
        || pack.omission_summary != manifest.omission_summary
        || pack.consumed_at < pack.created_at
        || pack.consumed_at >= pack.expires_at
        || i32::try_from(item_count).unwrap_or(i32::MAX) != manifest.item_count
        || !items_are_safe
    {
        return Err(MemoryClientError::Contract(format!(
            "consumed Context Pack {} does not match its manifest",
            pack.context_pack_id
        )));
    }
    if memory_required && item_count == 0 {
        return Err(MemoryClientError::Contract(format!(
            "required Context Pack {} delivered no memory",
            pack.context_pack_id
        )));
    }
    Ok(())
}

async fn retry_delay(attempt: u8) {
    let multiplier = 1_u64 << u32::from(attempt.min(3));
    sleep(Duration::from_millis(150 * multiplier)).await;
}

async fn require_success(mut response: Response) -> Result<Response, MemoryClientError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let header_request_id = ["x-request-id", "request-id"]
        .iter()
        .find_map(|name| response.headers().get(*name))
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let mut body = Vec::new();
    while body.len() < MAX_REMOTE_ERROR_BYTES {
        let chunk = response
            .chunk()
            .await
            .map_err(MemoryClientError::Transport)?;
        let Some(chunk) = chunk else {
            break;
        };
        let remaining = MAX_REMOTE_ERROR_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    let value = serde_json::from_slice::<Value>(&body).ok();
    let code = value
        .as_ref()
        .and_then(|json| json.get("code").or_else(|| json.pointer("/error/code")))
        .map(value_to_short_string)
        .unwrap_or_else(|| "REMOTE_REJECTED".into());
    let request_id = header_request_id
        .or_else(|| {
            value
                .as_ref()
                .and_then(|json| json.get("request_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "unavailable".into());
    let retryable = value
        .as_ref()
        .and_then(|json| json.get("retryable"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Err(MemoryClientError::Rejected {
        status,
        code,
        request_id,
        retryable,
    })
}

fn value_to_short_string(value: &Value) -> String {
    let raw = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => "REMOTE_REJECTED".into(),
    };
    raw.chars().take(128).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_command() -> ContextPackCommand {
        ContextPackCommand {
            task_context: TaskContext {
                task_id: Uuid::new_v4(),
                attempt_id: Uuid::new_v4(),
                purpose: "answer_user".into(),
            },
            query: "解释事务型 Outbox".into(),
            reference_time: None,
            timezone: None,
            scope_selectors: vec![ScopeSelector::user("alice")],
            memory_types: vec!["preference".into()],
            limit: 5,
            token_budget: 512,
            tokenizer_id: "demo-conservative-char-v1".into(),
            memory_required: true,
            pinned_memory_version_ids: Vec::new(),
        }
    }

    fn manifest(command: &ContextPackCommand) -> ContextPackManifestResponse {
        let now = Utc::now();
        ContextPackManifestResponse {
            context_pack_id: Uuid::new_v4(),
            task_id: command.task_context.task_id,
            attempt_id: command.task_context.attempt_id,
            purpose: command.task_context.purpose.clone(),
            authority_snapshot_id: "authority-v1".into(),
            policy_version: "policy-v1".into(),
            compiler_version: "compiler-v1".into(),
            ranking_version: "ranker-v1".into(),
            projection_generation: "generation-1".into(),
            created_at: now,
            expires_at: now + chrono::Duration::minutes(5),
            tokenizer_id: command.tokenizer_id.clone(),
            token_budget: command.token_budget,
            estimated_tokens: 48,
            item_count: 1,
            delivery_status: "compiled".into(),
            consumed_at: None,
            omission_summary: BTreeMap::new(),
        }
    }

    fn consumed_pack(manifest: &ContextPackManifestResponse) -> ContextPackResponse {
        ContextPackResponse {
            context_pack_id: manifest.context_pack_id,
            task_id: manifest.task_id,
            attempt_id: manifest.attempt_id,
            purpose: manifest.purpose.clone(),
            authority_snapshot_id: manifest.authority_snapshot_id.clone(),
            policy_version: manifest.policy_version.clone(),
            compiler_version: manifest.compiler_version.clone(),
            ranking_version: manifest.ranking_version.clone(),
            projection_generation: manifest.projection_generation.clone(),
            created_at: manifest.created_at,
            expires_at: manifest.expires_at,
            consumed_at: manifest.created_at,
            tokenizer_id: manifest.tokenizer_id.clone(),
            token_budget: manifest.token_budget,
            estimated_tokens: manifest.estimated_tokens,
            sections: vec![ContextSection {
                kind: "user_preference".into(),
                items: vec![ContextPackItem {
                    memory_record_id: Uuid::new_v4(),
                    memory_version_id: Uuid::new_v4(),
                    text: "先给生活类比".into(),
                    source_refs: vec!["conversation:test:message-1".into()],
                    effective_from: Some(manifest.created_at),
                    inclusion_reason: "EXPLICIT_PREFERENCE_AND_TASK_RELEVANT".into(),
                    trust: "untrusted_memory_data".into(),
                }],
            }],
            omission_summary: manifest.omission_summary.clone(),
        }
    }

    #[test]
    fn remote_service_requires_https_and_authentication() {
        assert!(
            MemoryHttpClient::new(MemoryClientConfig::loopback(
                "http://127.0.0.1:18080",
                Duration::from_secs(5)
            ))
            .is_ok()
        );
        assert!(
            MemoryHttpClient::new(MemoryClientConfig::loopback(
                "http://memory.example.test",
                Duration::from_secs(5)
            ))
            .is_err()
        );
        assert!(
            MemoryHttpClient::new(MemoryClientConfig::authenticated(
                "https://memory.example.test",
                Duration::from_secs(5),
                "test-token"
            ))
            .is_ok()
        );
    }

    #[test]
    fn idempotency_key_is_checked_before_transport() {
        assert!(validate_idempotency_key("0123456789abcdef").is_ok());
        assert!(validate_idempotency_key("short").is_err());
        assert!(validate_idempotency_key(" 0123456789abcdef").is_err());
        assert!(validate_idempotency_key("0123456789abcde\n").is_err());
    }

    #[test]
    fn context_pack_contract_binds_attempt_generation_and_trust() {
        let command = context_command();
        let manifest = manifest(&command);
        let pack = consumed_pack(&manifest);
        validate_manifest(&manifest, &command).unwrap();
        validate_consumed_pack(&manifest, &pack, true).unwrap();

        let mut wrong_generation = pack.clone();
        wrong_generation.projection_generation = "generation-2".into();
        assert!(validate_consumed_pack(&manifest, &wrong_generation, true).is_err());

        let mut unsafe_item = pack;
        unsafe_item.sections[0].items[0].trust = "system_instruction".into();
        assert!(validate_consumed_pack(&manifest, &unsafe_item, true).is_err());
    }

    #[test]
    fn extraction_response_rejects_count_drift_and_duplicate_identity() {
        let operation = OperationResponse {
            operation_id: Uuid::new_v4(),
            candidate_ids: vec![Uuid::new_v4()],
            memory_record_id: Uuid::new_v4(),
            memory_version_id: Some(Uuid::new_v4()),
            head_revision: 1,
            canonical_status: "committed".into(),
            projection_status: "pending".into(),
            reason_code: None,
            provider_object_id: None,
            attempts: 0,
        };
        let proposal = ExtractedCandidateProposal {
            memory_type: "preference".into(),
            semantic_key: "response.style".into(),
            content: "先给结论，再解释细节".into(),
            confidence: 0.95,
            importance: 0.8,
            event_time: None,
            structured_content: None,
        };
        let response = MemoryExtractionResponse {
            extractor_version: "test-extractor-v1".into(),
            candidate_count: 1,
            candidates: vec![ExtractedMemoryResult {
                proposal: proposal.clone(),
                operation: operation.clone(),
            }],
        };
        validate_extraction_response(&response, 2, "Asia/Shanghai").unwrap();

        let mut wrong_count = response.clone();
        wrong_count.candidate_count = 2;
        assert!(validate_extraction_response(&wrong_count, 2, "Asia/Shanghai").is_err());

        let mut duplicate = response;
        duplicate.candidate_count = 2;
        duplicate.candidates.push(ExtractedMemoryResult {
            proposal,
            operation,
        });
        assert!(validate_extraction_response(&duplicate, 2, "Asia/Shanghai").is_err());

        let mut event_response = wrong_count;
        event_response.candidate_count = 1;
        event_response.candidates[0].proposal.memory_type = "episodic_summary".into();
        event_response.candidates[0].proposal.event_time = Some(
            EventTime::new(
                DateTime::parse_from_rfc3339("2026-07-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                DateTime::parse_from_rfc3339("2026-07-02T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                "America/New_York",
                memory_domain::EventTimePrecision::Day,
            )
            .unwrap(),
        );
        assert!(validate_extraction_response(&event_response, 2, "Asia/Shanghai").is_err());
    }
}
