use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use memory_application::{
    IndexError, ProjectionDelete, ProjectionLookup, ProjectionStatus, ProjectionUpsert,
    RetrievalIndex,
};
use memory_postgres::{
    DemoAuthority, MemoryRepository, ProjectionOperationStatus, ProjectionOperationType,
    ProjectionWork, RepositoryError,
};
use tokio::time::sleep;
use tracing::{error, info, warn};
use uuid::Uuid;

const EXPIRY_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const EXPIRY_SWEEP_BATCH_SIZE: usize = 100;

pub struct ProjectionWorker {
    repository: MemoryRepository,
    index: Arc<dyn RetrievalIndex>,
    authority: DemoAuthority,
    worker_id: String,
    model_manifest_hash: String,
}

impl ProjectionWorker {
    pub fn new(
        repository: MemoryRepository,
        index: Arc<dyn RetrievalIndex>,
        authority: DemoAuthority,
        model_manifest_hash: String,
    ) -> Self {
        Self {
            repository,
            index,
            authority,
            worker_id: format!("demo-worker-{}", Uuid::new_v4()),
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
                .claim_projection(&self.authority, &self.worker_id)
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
