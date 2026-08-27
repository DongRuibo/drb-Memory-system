use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use memory_application::{
    ExtractMemoryRequest, ExtractorError, IndexError, MemoryExtractor, ProjectionDelete,
    ProjectionLookup, ProjectionStatus, ProjectionUpsert, RetrievalIndex, SecurityAction,
    SecurityPipeline,
};
use memory_domain::{MemoryType, MemoryTypeMapping};
use memory_postgres::{
    ConsolidationWork, DemoAuthority, IngestCandidateCommand, MemoryRepository,
    ProjectionModelManifest, ProjectionOperationStatus, ProjectionOperationType,
    ProjectionRebuildCommand, ProjectionWork, RepositoryError,
};
use serde_json::json;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{error, info, warn};
use uuid::Uuid;

const EXPIRY_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const EXPIRY_SWEEP_BATCH_SIZE: usize = 100;
const CONSOLIDATION_SOURCE_MAX_BYTES: usize = 32_000;
const MAX_DYNAMIC_TENANT_WORKERS: usize = 100;

#[derive(Clone)]
pub struct DynamicTenantWorkers {
    repository: MemoryRepository,
    index: Arc<dyn RetrievalIndex>,
    extractor: Option<Arc<dyn MemoryExtractor>>,
    model_manifest: ProjectionModelManifest,
    started_tenants: Arc<Mutex<BTreeSet<String>>>,
}

impl DynamicTenantWorkers {
    pub fn new(
        repository: MemoryRepository,
        index: Arc<dyn RetrievalIndex>,
        extractor: Option<Arc<dyn MemoryExtractor>>,
        model_manifest: ProjectionModelManifest,
    ) -> Self {
        Self {
            repository,
            index,
            extractor,
            model_manifest,
            started_tenants: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    pub async fn ensure_started(&self, tenant_id: &str) -> Result<(), RepositoryError> {
        let mut started_tenants = self.started_tenants.lock().await;
        if started_tenants.contains(tenant_id) {
            return Ok(());
        }
        if started_tenants.len() >= MAX_DYNAMIC_TENANT_WORKERS {
            return Err(RepositoryError::Conflict(format!(
                "dynamic tenant worker limit of {MAX_DYNAMIC_TENANT_WORKERS} has been reached"
            )));
        }

        let authority = DemoAuthority::worker(tenant_id.to_owned());
        bootstrap_empty_projection_generation(
            &self.repository,
            &authority,
            &self.model_manifest,
        )
        .await?;

        let projection_worker = ProjectionWorker::new(
            self.repository.clone(),
            Arc::clone(&self.index),
            authority.clone(),
            self.model_manifest.index_provider.clone(),
            self.model_manifest.manifest_hash.clone(),
        );
        tokio::spawn(projection_worker.run());
        if let Some(extractor) = self.extractor.as_ref() {
            let consolidation_worker = ConsolidationWorker::new(
                self.repository.clone(),
                Arc::clone(extractor),
                authority,
            );
            tokio::spawn(consolidation_worker.run());
        }
        started_tenants.insert(tenant_id.to_owned());
        info!(tenant_id = tenant_id, "started dynamic tenant workers");
        Ok(())
    }
}

pub async fn bootstrap_empty_projection_generation(
    repository: &MemoryRepository,
    authority: &DemoAuthority,
    manifest: &ProjectionModelManifest,
) -> Result<(), RepositoryError> {
    let current = repository
        .current_projection_generation_view(authority)
        .await?;
    if current.model_manifest.manifest_hash == manifest.manifest_hash {
        return Ok(());
    }
    if current.expected_records != 0 || current.ready_records != 0 || current.failed_records != 0 {
        return Err(RepositoryError::Conflict(format!(
            "MEMORY_BOOTSTRAP_EMPTY_PROJECTION cannot replace non-empty generation {}; run the controlled projection rebuild instead",
            current.generation
        )));
    }

    let hash_prefix = manifest.manifest_hash.get(..16).ok_or_else(|| {
        RepositoryError::Invariant("projection model manifest hash is unexpectedly short".into())
    })?;
    let generation = format!("bootstrap-{hash_prefix}");
    let rebuild_key = format!("bootstrap-rebuild-{hash_prefix}");
    let activate_key = format!("bootstrap-activate-{hash_prefix}");
    let finalize_key = format!("bootstrap-finalize-{hash_prefix}");
    let mut target = repository
        .start_projection_rebuild(
            authority,
            ProjectionRebuildCommand {
                generation: generation.clone(),
                idempotency_key: rebuild_key,
                model_manifest: manifest.clone(),
            },
        )
        .await?;
    if target.status == "ready" {
        target = repository
            .activate_projection_generation(
                authority,
                &generation,
                &current.generation,
                &activate_key,
            )
            .await?;
    }
    if target.status == "canary" {
        target = repository
            .finalize_projection_generation(authority, &generation, &finalize_key)
            .await?;
    }
    if target.status != "active" || target.model_manifest.manifest_hash != manifest.manifest_hash {
        return Err(RepositoryError::Invariant(format!(
            "empty projection bootstrap did not converge: generation={} status={}",
            target.generation, target.status
        )));
    }
    info!(
        tenant_id = authority.tenant_id,
        generation = target.generation,
        manifest_hash = manifest.manifest_hash,
        "bootstrapped empty projection generation"
    );
    Ok(())
}

pub struct ProjectionWorker {
    repository: MemoryRepository,
    index: Arc<dyn RetrievalIndex>,
    authority: DemoAuthority,
    worker_id: String,
    index_provider: String,
    model_manifest_hash: String,
}

impl ProjectionWorker {
    pub fn new(
        repository: MemoryRepository,
        index: Arc<dyn RetrievalIndex>,
        authority: DemoAuthority,
        index_provider: String,
        model_manifest_hash: String,
    ) -> Self {
        Self {
            repository,
            index,
            authority,
            worker_id: format!("demo-worker-{}", Uuid::new_v4()),
            index_provider,
            model_manifest_hash,
        }
    }

    pub async fn run(self) {
        let mut next_expiry_sweep = Instant::now();
        loop {
            if Instant::now() >= next_expiry_sweep {
                match self
                    .repository
                    .expire_due(&self.authority, EXPIRY_SWEEP_BATCH_SIZE)
                    .await
                {
                    Ok(operations) => {
                        let expired_count = operations.len();
                        if expired_count > 0 {
                            info!(expired_count, "expired due memory records");
                        }
                        next_expiry_sweep = if expired_count == EXPIRY_SWEEP_BATCH_SIZE {
                            Instant::now()
                        } else {
                            Instant::now() + EXPIRY_SWEEP_INTERVAL
                        };
                    }
                    Err(error) => {
                        error!(error = %error, "memory expiry sweep failed");
                        next_expiry_sweep = Instant::now() + Duration::from_secs(1);
                    }
                }
            }
            match self
                .repository
                .claim_projection_for_provider(
                    &self.authority,
                    &self.worker_id,
                    &self.index_provider,
                )
                .await
            {
                Ok(Some(work)) => {
                    if let Err(error) = self.process(work).await {
                        error!(error = %error, "projection work failed");
                    }
                }
                Ok(None) => sleep(Duration::from_millis(100)).await,
                Err(error) => {
                    error!(error = %error, "failed to claim projection work");
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn process(&self, work: ProjectionWork) -> Result<(), RepositoryError> {
        if work.model_manifest_hash != self.model_manifest_hash {
            return self
                .repository
                .reschedule(
                    &self.authority,
                    &work,
                    ProjectionOperationStatus::Failed,
                    "EMBEDDING_MANIFEST_MISMATCH",
                )
                .await;
        }
        match work.operation_type {
            ProjectionOperationType::Upsert => self.process_upsert(&work).await,
            ProjectionOperationType::Delete => self.process_delete(&work).await,
        }
    }

    async fn process_upsert(&self, work: &ProjectionWork) -> Result<(), RepositoryError> {
        if work.current_head_state != "active"
            || work.current_head_revision != work.expected_head_revision
            || work.current_version_id != work.memory_version_id
        {
            return self
                .repository
                .complete_stale(&self.authority, work, "STALE_CANONICAL_HEAD")
                .await;
        }

        let existing = match self
            .lookup(work, Some(work.operation_id), work.memory_version_id)
            .await
        {
            Ok(hits) => hits,
            Err(error) => return self.reschedule_index_error(work, error).await,
        };
        if !existing.is_empty() {
            let keeper = existing
                .iter()
                .map(|hit| hit.provider_object_id.as_str())
                .min()
                .expect("existing is not empty")
                .to_owned();
            if let Err(error) = self.cleanup_record_projections(work, &keeper).await {
                return self.reschedule_index_error(work, error).await;
            }
            info!(operation_id = %work.operation_id, provider_object_id = %keeper, "projection reconciled");
            return self
                .repository
                .complete_ready(&self.authority, work, &keeper)
                .await;
        }

        if work.prior_status == ProjectionOperationStatus::Unknown && work.attempts < 3 {
            return self
                .repository
                .reschedule(
                    &self.authority,
                    work,
                    ProjectionOperationStatus::Unknown,
                    "UNKNOWN_RECONCILING",
                )
                .await;
        }

        let Some(version_id) = work.memory_version_id else {
            return self
                .repository
                .complete_stale(&self.authority, work, "VERSION_MISSING")
                .await;
        };
        let Some(content) = work.content.clone() else {
            return self
                .repository
                .complete_stale(&self.authority, work, "CONTENT_MISSING")
                .await;
        };
        let Some(content_hash) = work.content_hash.clone() else {
            return self
                .repository
                .complete_stale(&self.authority, work, "CONTENT_HASH_MISSING")
                .await;
        };
        match self
            .index
            .upsert(ProjectionUpsert {
                operation_id: work.operation_id,
                namespace_key: work.namespace_key.clone(),
                memory_record_id: work.memory_record_id,
                memory_version_id: version_id,
                head_revision: work.expected_head_revision as u64,
                generation: work.generation.clone(),
                content,
                content_hash,
            })
            .await
        {
            Ok(receipt) if receipt.status == ProjectionStatus::Ready => {
                let provider_object_id = receipt.provider_object_id.ok_or_else(|| {
                    RepositoryError::Invariant("ready receipt has no Provider object ID".into())
                })?;
                if let Err(error) = self
                    .cleanup_record_projections(work, &provider_object_id)
                    .await
                {
                    return self.reschedule_index_error(work, error).await;
                }
                self.repository
                    .complete_ready(&self.authority, work, &provider_object_id)
                    .await
            }
            Ok(_) => {
                self.repository
                    .reschedule(
                        &self.authority,
                        work,
                        ProjectionOperationStatus::Failed,
                        "UNEXPECTED_PROVIDER_STATUS",
                    )
                    .await
            }
            Err(error) => self.reschedule_index_error(work, error).await,
        }
    }

    async fn process_delete(&self, work: &ProjectionWork) -> Result<(), RepositoryError> {
        let valid_terminal_delete = matches!(
            work.current_head_state.as_str(),
            "expired" | "forgotten" | "erased"
        );
        let valid_contested_delete = work.current_head_state == "contested"
            && work.current_version_id == work.memory_version_id;
        if work.current_head_revision != work.expected_head_revision
            || (!valid_terminal_delete && !valid_contested_delete)
        {
            return self
                .repository
                .complete_stale(&self.authority, work, "DELETE_NOT_CURRENT")
                .await;
        }
        let generations = self
            .repository
            .projection_generations_for_cleanup(&self.authority, &self.model_manifest_hash)
            .await?;
        for generation in generations {
            let mut generation_work = work.clone();
            generation_work.generation = generation;
            let mut provider_ids = BTreeSet::new();
            if generation_work.generation == work.generation
                && let Some(provider_object_id) = &work.provider_object_id
            {
                provider_ids.insert(provider_object_id.clone());
            }
            match self.lookup(&generation_work, None, None).await {
                Ok(hits) => provider_ids.extend(hits.into_iter().map(|hit| hit.provider_object_id)),
                Err(error) => return self.reschedule_index_error(work, error).await,
            }
            for provider_object_id in provider_ids {
                if let Err(error) = self
                    .delete_provider_object(&generation_work, &provider_object_id)
                    .await
                {
                    return self.reschedule_index_error(work, error).await;
                }
            }
        }
        self.repository
            .complete_deleted(&self.authority, work)
            .await
    }

    async fn lookup(
        &self,
        work: &ProjectionWork,
        operation_id: Option<Uuid>,
        memory_version_id: Option<Uuid>,
    ) -> Result<Vec<memory_application::ProjectionHit>, IndexError> {
        self.index
            .lookup(ProjectionLookup {
                namespace_key: work.namespace_key.clone(),
                generation: work.generation.clone(),
                operation_id,
                memory_record_id: Some(work.memory_record_id),
                memory_version_id,
                limit: 50,
            })
            .await
    }

    async fn cleanup_record_projections(
        &self,
        work: &ProjectionWork,
        keeper_provider_object_id: &str,
    ) -> Result<(), IndexError> {
        let projections = self.lookup(work, None, None).await?;
        for projection in projections
            .into_iter()
            .filter(|hit| hit.provider_object_id != keeper_provider_object_id)
        {
            self.delete_provider_object(work, &projection.provider_object_id)
                .await?;
        }
        Ok(())
    }

    async fn delete_provider_object(
        &self,
        work: &ProjectionWork,
        provider_object_id: &str,
    ) -> Result<(), IndexError> {
        self.index
            .delete(ProjectionDelete {
                operation_id: work.operation_id,
                namespace_key: work.namespace_key.clone(),
                provider_object_id: provider_object_id.to_owned(),
                memory_record_id: work.memory_record_id,
                head_revision: work.expected_head_revision as u64,
                generation: work.generation.clone(),
            })
            .await?;
        Ok(())
    }

    async fn reschedule_index_error(
        &self,
        work: &ProjectionWork,
        error: IndexError,
    ) -> Result<(), RepositoryError> {
        let (status, code) = match error {
            IndexError::UnknownResult => (ProjectionOperationStatus::Unknown, "PROVIDER_UNKNOWN"),
            IndexError::Rejected { .. } => (ProjectionOperationStatus::Failed, "PROVIDER_REJECTED"),
            IndexError::InvalidResponse(_) => {
                (ProjectionOperationStatus::Failed, "PROVIDER_CONTRACT")
            }
            IndexError::Transport(_) => (ProjectionOperationStatus::Failed, "PROVIDER_TRANSPORT"),
        };
        warn!(operation_id = %work.operation_id, error = %error, "projection work rescheduled");
        if work.attempts >= 8 {
            return self
                .repository
                .complete_dead_letter(&self.authority, work, code)
                .await;
        }
        self.repository
            .reschedule(&self.authority, work, status, code)
            .await
    }
}

pub struct ConsolidationWorker {
    repository: MemoryRepository,
    extractor: Arc<dyn MemoryExtractor>,
    authority: DemoAuthority,
    worker_id: String,
}

impl ConsolidationWorker {
    pub fn new(
        repository: MemoryRepository,
        extractor: Arc<dyn MemoryExtractor>,
        authority: DemoAuthority,
    ) -> Self {
        Self {
            repository,
            extractor,
            authority,
            worker_id: format!("consolidation-worker-{}", Uuid::new_v4()),
        }
    }

    pub async fn run(self) {
        loop {
            match self.run_once().await {
                Ok(true) => {}
                Ok(false) => sleep(Duration::from_millis(250)).await,
                Err(error) => {
                    error!(error = %error, "failed to claim or update consolidation work");
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    pub async fn run_once(&self) -> Result<bool, RepositoryError> {
        let Some(work) = self
            .repository
            .claim_consolidation(&self.authority, &self.worker_id)
            .await?
        else {
            return Ok(false);
        };
        if let Err(failure) = self.process(&work).await {
            warn!(
                consolidation_id = %work.consolidation_id,
                error_code = failure.code,
                error = %failure.detail,
                "episode consolidation failed and will be retried"
            );
            self.repository
                .reschedule_consolidation(&self.authority, &work, failure.code)
                .await?;
        }
        Ok(true)
    }

    async fn process(&self, work: &ConsolidationWork) -> Result<(), ConsolidationFailure> {
        let source_content = render_consolidation_source(work)?;
        let (extractor_version, proposals, diagnostics) =
            match (&work.extractor_version, &work.proposals, &work.diagnostics) {
                (Some(version), Some(proposals), Some(diagnostics)) => {
                    (version.clone(), proposals.clone(), diagnostics.clone())
                }
                (None, None, None) => {
                    let mut allowed_memory_types = vec![
                        MemoryType::Preference.as_str().into(),
                        MemoryType::ProfileFact.as_str().into(),
                        MemoryType::EpisodicSummary.as_str().into(),
                    ];
                    if work.project_id.is_some() {
                        allowed_memory_types.push(MemoryType::SemanticFact.as_str().into());
                    }
                    let last_turn = work.turns.last().ok_or_else(|| ConsolidationFailure {
                        code: "CONSOLIDATION_EMPTY_COVERAGE",
                        detail: "claimed consolidation contained no completed turns".into(),
                    })?;
                    let output = self
                        .extractor
                        .extract(ExtractMemoryRequest {
                            source_ref: consolidation_source_ref(work),
                            content: source_content.clone(),
                            source_event_time: last_turn.completed_at,
                            timezone: last_turn.timezone.clone(),
                            max_candidates: 4,
                            allowed_memory_types,
                        })
                        .await
                        .map_err(ConsolidationFailure::extractor)?;
                    let proposals = sanitize_consolidation_proposals(output.proposals)?;
                    let extractor_version = self.extractor.version().to_owned();
                    self.repository
                        .store_consolidation_snapshot(
                            &self.authority,
                            work,
                            &extractor_version,
                            &proposals,
                            &output.diagnostics,
                        )
                        .await
                        .map_err(ConsolidationFailure::repository)?;
                    (extractor_version, proposals, output.diagnostics)
                }
                _ => {
                    return Err(ConsolidationFailure {
                        code: "CONSOLIDATION_SNAPSHOT_INVALID",
                        detail: "stored consolidation snapshot is incomplete".into(),
                    });
                }
            };
        let _ = diagnostics;

        let mut user_authority =
            DemoAuthority::user(self.authority.tenant_id.clone(), work.user_id.clone());
        if let Some(project_id) = &work.project_id {
            user_authority.project_ids.insert(project_id.clone());
        }
        let mut operation_ids = Vec::with_capacity(proposals.len());
        for (position, proposal) in proposals.into_iter().enumerate() {
            let memory_type = match MemoryType::map_input(&proposal.memory_type) {
                Ok(MemoryTypeMapping::Canonical(memory_type)) => memory_type,
                _ => {
                    return Err(ConsolidationFailure {
                        code: "CONSOLIDATION_CANDIDATE_TYPE",
                        detail: format!(
                            "consolidator returned non-canonical type {:?}",
                            proposal.memory_type
                        ),
                    });
                }
            };
            let (scope_kind, scope_id) = if memory_type == MemoryType::SemanticFact {
                let project_id = work
                    .project_id
                    .clone()
                    .ok_or_else(|| ConsolidationFailure {
                        code: "CONSOLIDATION_PROJECT_SCOPE",
                        detail: "semantic fact has no authorized project scope".into(),
                    })?;
                ("project".to_owned(), Some(project_id))
            } else {
                ("user".to_owned(), Some(work.user_id.clone()))
            };
            let candidate_key = format!("episode-candidate-{}-{position}", work.consolidation_id);
            let operation = self
                .repository
                .ingest_extracted(
                    &user_authority,
                    IngestCandidateCommand {
                        idempotency_key: candidate_key.clone(),
                        event_id: format!(
                            "episode-consolidation-{}-{position}",
                            work.consolidation_id
                        ),
                        source_type: "reflection".into(),
                        source_ref: format!(
                            "{}#candidate-{position}",
                            consolidation_source_ref(work)
                        ),
                        source_content: source_content.clone(),
                        memory_content: proposal.content,
                        occurred_at: work
                            .turns
                            .last()
                            .expect("claimed work has turns")
                            .completed_at,
                        event_time: proposal.event_time,
                        structured_content: proposal.structured_content,
                        memory_type: memory_type.as_str().into(),
                        semantic_key: proposal.semantic_key,
                        confidence: proposal.confidence,
                        importance: proposal.importance,
                        inference_kind: "derived".into(),
                        extractor_name: "episode-consolidator".into(),
                        extractor_version: extractor_version.clone(),
                        scope_kind,
                        scope_id,
                    },
                )
                .await
                .map_err(ConsolidationFailure::repository)?;
            operation_ids.push(operation.operation_id);
        }
        self.repository
            .complete_consolidation(&self.authority, work, &operation_ids)
            .await
            .map_err(ConsolidationFailure::repository)?;
        info!(
            consolidation_id = %work.consolidation_id,
            episode_id = %work.episode_id,
            candidate_count = operation_ids.len(),
            "episode consolidation completed"
        );
        Ok(())
    }
}

#[derive(Debug)]
struct ConsolidationFailure {
    code: &'static str,
    detail: String,
}

impl ConsolidationFailure {
    fn extractor(error: ExtractorError) -> Self {
        let code = match error {
            ExtractorError::Config(_) => "CONSOLIDATION_EXTRACTOR_CONFIG",
            ExtractorError::Transport(_) => "CONSOLIDATION_EXTRACTOR_TRANSPORT",
            ExtractorError::InvalidResponse(_) => "CONSOLIDATION_EXTRACTOR_CONTRACT",
        };
        Self {
            code,
            detail: error.to_string(),
        }
    }

    fn repository(error: RepositoryError) -> Self {
        Self {
            code: "CONSOLIDATION_REPOSITORY",
            detail: error.to_string(),
        }
    }
}

fn consolidation_source_ref(work: &ConsolidationWork) -> String {
    format!(
        "episode:{}:turns:{}-{}",
        work.episode_id, work.from_turn_sequence, work.through_turn_sequence
    )
}

fn sanitize_consolidation_proposals(
    mut proposals: Vec<memory_application::ExtractCandidateProposal>,
) -> Result<Vec<memory_application::ExtractCandidateProposal>, ConsolidationFailure> {
    let pipeline = SecurityPipeline::default();
    for proposal in &mut proposals {
        let content = pipeline.inspect(&proposal.content);
        let semantic_key = pipeline.inspect(&proposal.semantic_key);
        if content.contains_prompt_injection()
            || content.contains_secret()
            || content.action == SecurityAction::Reject
            || semantic_key.contains_prompt_injection()
            || semantic_key.contains_secret()
            || semantic_key.action == SecurityAction::Reject
        {
            return Err(ConsolidationFailure {
                code: "CONSOLIDATION_CANDIDATE_UNSAFE",
                detail: "consolidator returned an unsafe candidate".into(),
            });
        }
        proposal.content = content.redacted_text;
        proposal.semantic_key = semantic_key.redacted_text;
        if let Some(structured) = proposal.structured_content.take() {
            let raw = serde_json::to_value(structured).map_err(|error| ConsolidationFailure {
                code: "CONSOLIDATION_CANDIDATE_ENCODING",
                detail: error.to_string(),
            })?;
            let sanitized = pipeline.sanitize_json(&raw);
            if sanitized.contains_prompt_injection() || sanitized.contains_secret() {
                return Err(ConsolidationFailure {
                    code: "CONSOLIDATION_CANDIDATE_UNSAFE",
                    detail: "consolidator returned unsafe structured content".into(),
                });
            }
            proposal.structured_content =
                Some(serde_json::from_value(sanitized.value).map_err(|error| {
                    ConsolidationFailure {
                        code: "CONSOLIDATION_CANDIDATE_ENCODING",
                        detail: error.to_string(),
                    }
                })?);
        }
    }
    Ok(proposals)
}

fn render_consolidation_source(work: &ConsolidationWork) -> Result<String, ConsolidationFailure> {
    if work.turns.is_empty() {
        return Err(ConsolidationFailure {
            code: "CONSOLIDATION_EMPTY_COVERAGE",
            detail: "claimed consolidation contained no completed turns".into(),
        });
    }
    let per_turn_budget = 12_000usize.saturating_div(work.turns.len()).max(512);
    let turns = work
        .turns
        .iter()
        .map(|turn| {
            let tool_budget =
                (per_turn_budget / 5).saturating_div(turn.verified_tool_results.len().max(1));
            let tools = turn
                .verified_tool_results
                .iter()
                .map(|result| {
                    json!({
                        "tool_call_id": result.tool_call_id,
                        "tool_name": result.tool_name,
                        "content": truncate_utf8(&result.content, tool_budget),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "sequence_no": turn.sequence_no,
                "turn_id": turn.turn_id,
                "user_content": truncate_utf8(&turn.user_content, per_turn_budget * 2 / 5),
                "assistant_content": truncate_utf8(
                    &turn.assistant_content,
                    per_turn_budget * 2 / 5,
                ),
                "verified_tool_results": tools,
                "outcome": turn.outcome,
                "completed_at": turn.completed_at,
            })
        })
        .collect::<Vec<_>>();
    let rendered = serde_json::to_string(&json!({
        "schema_version": "lazytt-completed-turn-batch-v1",
        "trust": "untrusted_conversation_data",
        "instructions_executable": false,
        "episode_id": work.episode_id,
        "from_turn_sequence": work.from_turn_sequence,
        "through_turn_sequence": work.through_turn_sequence,
        "turns": turns,
    }))
    .map_err(|error| ConsolidationFailure {
        code: "CONSOLIDATION_SOURCE_ENCODING",
        detail: error.to_string(),
    })?;
    if rendered.len() > CONSOLIDATION_SOURCE_MAX_BYTES {
        return Err(ConsolidationFailure {
            code: "CONSOLIDATION_SOURCE_OVERSIZE",
            detail: format!(
                "bounded consolidation source exceeded {CONSOLIDATION_SOURCE_MAX_BYTES} bytes"
            ),
        });
    }
    Ok(rendered)
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let suffix = "...[truncated]";
    let target = max_bytes.saturating_sub(suffix.len());
    let mut boundary = target.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}{}", &value[..boundary], suffix)
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use memory_application::{
        ExtractCandidateProposal, ExtractMemoryOutput, ExtractMemoryRequest, ExtractionDiagnostics,
    };
    use memory_domain::{ConsolidationStatus, TurnOutcome};
    use memory_postgres::{AppendCompletedTurnCommand, DemoAuthority};

    use super::*;

    struct FakeExtractor;

    #[async_trait]
    impl MemoryExtractor for FakeExtractor {
        fn version(&self) -> &str {
            "fake-episode-consolidator-v1"
        }

        async fn extract(
            &self,
            request: ExtractMemoryRequest,
        ) -> Result<ExtractMemoryOutput, ExtractorError> {
            assert_eq!(request.max_candidates, 4);
            assert!(request.content.contains("lazytt-completed-turn-batch-v1"));
            assert!(request.content.contains("第 10 轮"));
            Ok(ExtractMemoryOutput {
                proposals: vec![ExtractCandidateProposal {
                    memory_type: "preference".into(),
                    semantic_key: "user.response_style.language".into(),
                    content: "用户偏好使用中文回答。".into(),
                    confidence: 0.95,
                    importance: 0.8,
                    event_time: None,
                    structured_content: None,
                }],
                diagnostics: ExtractionDiagnostics::legacy(1),
            })
        }
    }

    #[test]
    fn consolidation_source_is_bounded_and_excludes_hidden_messages() {
        let work = fake_work();
        let source = render_consolidation_source(&work).unwrap();
        assert!(source.len() <= CONSOLIDATION_SOURCE_MAX_BYTES);
        assert!(source.contains("untrusted_conversation_data"));
        assert!(!source.contains("hidden_reasoning"));
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn dynamic_tenant_projection_bootstrap_is_idempotent() {
        let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let authority = DemoAuthority::worker(format!("dynamic-tenant-{}", Uuid::new_v4()));
        let manifest = ProjectionModelManifest::new(
            "mem0".into(),
            "dynamic-test".into(),
            "dynamic-test-embedding".into(),
            "test-v1".into(),
            128,
            "l2".into(),
            "dynamic-test-v1".into(),
            true,
        )
        .unwrap();

        bootstrap_empty_projection_generation(&repository, &authority, &manifest)
            .await
            .unwrap();
        bootstrap_empty_projection_generation(&repository, &authority, &manifest)
            .await
            .unwrap();

        let current = repository
            .current_projection_generation_view(&authority)
            .await
            .unwrap();
        assert_eq!(current.status, "active");
        assert_eq!(
            current.model_manifest.manifest_hash,
            manifest.manifest_hash
        );
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn fake_extractor_consolidates_ten_completed_turns_through_governance() {
        let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let tenant_id = format!("episode-fake-e2e-{}", Uuid::new_v4());
        let authority = DemoAuthority::user(&tenant_id, "fake-user");
        let episode_id = Uuid::new_v4();
        let completed_at = Utc.with_ymd_and_hms(2026, 8, 12, 10, 0, 0).unwrap();
        for sequence in 1..=10 {
            let attempt_id = Uuid::new_v4();
            let receipt = repository
                .append_completed_turn(
                    &authority,
                    AppendCompletedTurnCommand {
                        idempotency_key: format!("fake-completed-turn-{attempt_id}"),
                        request_hash: format!("sha256:{:064x}", sequence),
                        episode_id,
                        turn_id: attempt_id,
                        task_id: Uuid::new_v4(),
                        attempt_id,
                        user_content: format!("第 {sequence} 轮，请继续用中文。"),
                        assistant_content: format!("第 {sequence} 轮已用中文回答。"),
                        verified_tool_results: Vec::new(),
                        outcome: TurnOutcome::Succeeded,
                        completed_at,
                        timezone: "Asia/Shanghai".into(),
                        project_id: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(receipt.sequence_no, sequence);
            if sequence < 10 {
                assert!(receipt.consolidation_id.is_none());
            } else {
                assert_eq!(
                    receipt.consolidation_status,
                    Some(ConsolidationStatus::Pending)
                );
            }
        }

        let worker = ConsolidationWorker::new(
            repository.clone(),
            Arc::new(FakeExtractor),
            DemoAuthority::worker(&tenant_id),
        );
        assert!(worker.run_once().await.unwrap());

        let status = repository
            .episode_status(&authority, episode_id)
            .await
            .unwrap();
        assert_eq!(status.episode.last_consolidated_turn_sequence, 10);
        assert_eq!(status.unconsolidated_turn_count, 0);
        assert_eq!(status.consolidations.len(), 1);
        assert_eq!(
            status.consolidations[0].status,
            ConsolidationStatus::Completed
        );
        assert_eq!(status.consolidations[0].proposal_count, Some(1));
        assert_eq!(status.consolidations[0].operation_ids.len(), 1);
    }

    fn fake_work() -> ConsolidationWork {
        let episode_id = Uuid::new_v4();
        let completed_at = Utc.with_ymd_and_hms(2026, 8, 12, 10, 0, 0).unwrap();
        ConsolidationWork {
            consolidation_id: Uuid::new_v4(),
            episode_id,
            user_id: "fake-user".into(),
            project_id: None,
            from_turn_sequence: 1,
            through_turn_sequence: 1,
            trigger: memory_domain::ConsolidationTrigger::Manual,
            attempts: 1,
            worker_id: "fake-worker".into(),
            turns: vec![memory_domain::CompletedTurn {
                episode_id,
                turn_id: Uuid::new_v4(),
                task_id: Uuid::new_v4(),
                attempt_id: Uuid::new_v4(),
                sequence_no: 1,
                user_content: "请用中文回答。".into(),
                assistant_content: "已经使用中文回答。".into(),
                verified_tool_results: Vec::new(),
                outcome: TurnOutcome::Succeeded,
                completed_at,
                timezone: "Asia/Shanghai".into(),
            }],
            extractor_version: None,
            proposals: None,
            diagnostics: None,
        }
    }
}
