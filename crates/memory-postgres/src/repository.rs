use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use memory_application::{
    CHINESE_TOKENIZER_VERSION, ChineseTokenizer, ExtractCandidateProposal,
    MIN_TRANSPARENT_RELEVANCE_SCORE, ProjectionHit, SECURITY_POLICY_VERSION, SecurityFinding,
    SecurityPipeline, TemporalWindow, is_memory_applicable_without_semantic_threshold,
    lexical_relevance_score,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{PgPool, Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::model::{
    AuthorizedMemory, CanonicalStatus, ConsumeContextPackCommand, ContextPackCommand,
    ContextPackItem, ContextPackManifestView, ContextPackView, ContextSection, CorrectionCommand,
    DemoAuthority, ErasureCommand, ErasureRecordView, ErasureRequestView, ErasureTargetKind,
    ExtractionSnapshotCommand, ExtractionSnapshotView, ForgetCommand, IngestCandidateCommand,
    IngestCommand, MemoryEventView, MemoryExportView, MemoryRecordView, MemoryScopeView,
    MemoryVersionView, OperationView, OutcomeReceiptCommand, OutcomeReceiptView,
    ProjectionGenerationView, ProjectionModelManifest, ProjectionOperationStatus,
    ProjectionOperationType, ProjectionRebuildCommand, ProjectionWork, RetrievalCandidateTraceView,
    RetrievalRunCommand, RetrievalRunView, ReviewDecisionCommand, ReviewOutcome,
};

pub const DEMO_GENERATION: &str = "demo-v0-generation-1";
pub const DEMO_CONTEXT_TOKENIZER_ID: &str = "demo-conservative-char-v1";
pub const DEMO_CONTEXT_POLICY_VERSION: &str = "demo-policy-v1";
pub const DEMO_CONTEXT_COMPILER_VERSION: &str = "demo-context-compiler-v1";
const DEMO_ID_NAMESPACE: Uuid = Uuid::from_u128(0x7bc8_5039_50f0_4ff1_8200_7e73_a455_f372);
const DEMO_CONTEXT_ITEM_OVERHEAD: i32 = 32;
static SECURITY_PIPELINE: LazyLock<SecurityPipeline> = LazyLock::new(SecurityPipeline::default);
static CHINESE_TOKENIZER: LazyLock<ChineseTokenizer> = LazyLock::new(ChineseTokenizer::default);
const SUPPORTED_CONTEXT_MEMORY_TYPES: [&str; 4] = [
    "preference",
    "profile_fact",
    "project_fact",
    "episodic_summary",
];
const SUPPORTED_EXTRACTED_MEMORY_TYPES: [&str; 4] = [
    "preference",
    "profile_fact",
    "project_fact",
    "episodic_summary",
];

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("resource not found")]
    NotFound,
    #[error("operation is not authorized for the requested memory scope")]
    Unauthorized,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("lease was lost before the worker could commit")]
    LeaseLost,
    #[error("repository invariant violated: {0}")]
    Invariant(String),
}

#[derive(Clone)]
pub struct MemoryRepository {
    pool: PgPool,
}

impl MemoryRepository {
    pub async fn connect(database_url: &str) -> Result<Self, RepositoryError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> Result<(), RepositoryError> {
        sqlx::migrate!("../../migrations").run(&self.pool).await?;
        Ok(())
    }

    pub async fn ready(&self) -> Result<(), RepositoryError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn begin_extraction_snapshot(
        &self,
        authority: &DemoAuthority,
        command: &ExtractionSnapshotCommand,
    ) -> Result<ExtractionSnapshotView, RepositoryError> {
        validate_extraction_snapshot_command(command)?;
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        sqlx::query(
            "INSERT INTO memory_core.extraction_operations(
                 tenant_id,idempotency_key,request_hash,extractor_version,status
             ) VALUES ($1,$2,$3,$4,'pending')
             ON CONFLICT (tenant_id,idempotency_key) DO NOTHING",
        )
        .bind(&authority.tenant_id)
        .bind(&command.idempotency_key)
        .bind(&command.request_hash)
        .bind(&command.extractor_version)
        .execute(&mut *tx)
        .await?;
        let view = load_extraction_snapshot_tx(
            &mut tx,
            &authority.tenant_id,
            &command.idempotency_key,
            true,
        )
        .await?;
        if view.0 != command.request_hash {
            return Err(RepositoryError::Conflict(
                "idempotency key was reused for a different extraction request".into(),
            ));
        }
        if view.2.is_none() && view.1 != command.extractor_version {
            return Err(RepositoryError::Conflict(
                "an unfinished extraction cannot move to a different extractor version".into(),
            ));
        }
        tx.commit().await?;
        Ok(ExtractionSnapshotView {
            extractor_version: view.1,
            proposals: view.2,
        })
    }

    pub async fn complete_extraction_snapshot(
        &self,
        authority: &DemoAuthority,
        command: &ExtractionSnapshotCommand,
        proposals: Vec<ExtractCandidateProposal>,
    ) -> Result<ExtractionSnapshotView, RepositoryError> {
        validate_extraction_snapshot_command(command)?;
        validate_extraction_proposals(&proposals)?;
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let existing = load_extraction_snapshot_tx(
            &mut tx,
            &authority.tenant_id,
            &command.idempotency_key,
            true,
        )
        .await?;
        if existing.0 != command.request_hash {
            return Err(RepositoryError::Conflict(
                "idempotency key was reused for a different extraction request".into(),
            ));
        }
        if let Some(stored) = existing.2 {
            tx.commit().await?;
            return Ok(ExtractionSnapshotView {
                extractor_version: existing.1,
                proposals: Some(stored),
            });
        }
        if existing.1 != command.extractor_version {
            return Err(RepositoryError::Conflict(
                "an unfinished extraction cannot move to a different extractor version".into(),
            ));
        }
        sqlx::query(
            "UPDATE memory_core.extraction_operations
             SET status='completed',proposals=$3,completed_at=now()
             WHERE tenant_id=$1 AND idempotency_key=$2 AND status='pending'",
        )
        .bind(&authority.tenant_id)
        .bind(&command.idempotency_key)
        .bind(serde_json::to_value(&proposals).map_err(|error| {
            RepositoryError::Invariant(format!("failed to encode extraction proposals: {error}"))
        })?)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ExtractionSnapshotView {
            extractor_version: command.extractor_version.clone(),
            proposals: Some(proposals),
        })
    }

    pub async fn ingest_explicit(
        &self,
        authority: &DemoAuthority,
        command: IngestCommand,
    ) -> Result<OperationView, RepositoryError> {
        let semantic_key = preference_semantic_key(&command.memory_content);
        self.ingest_candidate(
            authority,
            IngestCandidateCommand {
                idempotency_key: command.idempotency_key,
                event_id: command.event_id,
                source_type: command.source_type,
                source_ref: command.source_ref,
                source_content: command.source_content,
                memory_content: command.memory_content,
                occurred_at: command.occurred_at,
                memory_type: "preference".into(),
                semantic_key,
                confidence: 1.0,
                importance: 0.8,
                inference_kind: "explicit".into(),
                extractor_name: "demo-explicit-memory".into(),
                extractor_version: "1".into(),
                scope_kind: "user".into(),
                scope_id: Some(authority.user_id.clone()),
            },
        )
        .await
    }

    pub async fn ingest_extracted(
        &self,
        authority: &DemoAuthority,
        command: IngestCandidateCommand,
    ) -> Result<OperationView, RepositoryError> {
        self.ingest_candidate(authority, command).await
    }

    async fn ingest_candidate(
        &self,
        authority: &DemoAuthority,
        mut command: IngestCandidateCommand,
    ) -> Result<OperationView, RepositoryError> {
        validate_ingest_candidate(&command)?;
        let source_finding = SECURITY_PIPELINE.inspect(&command.source_content);
        ensure_safe_for_storage(&source_finding)?;
        let memory_finding = SECURITY_PIPELINE.inspect(&command.memory_content);
        ensure_safe_for_storage(&memory_finding)?;
        let semantic_key_finding = SECURITY_PIPELINE.inspect(&command.semantic_key);
        ensure_safe_for_storage(&semantic_key_finding)?;
        let mut security_flags = source_finding.flags.clone();
        security_flags.extend(memory_finding.flags.clone());
        security_flags.extend(semantic_key_finding.flags.clone());
        security_flags.sort();
        security_flags.dedup();
        command.memory_content = memory_finding.redacted_text;
        command.semantic_key = semantic_key_finding.redacted_text;
        if !authority.authorizes_scope(&command.scope_kind, command.scope_id.as_deref()) {
            return Err(RepositoryError::Unauthorized);
        }
        let scope_id = command.scope_id.as_deref();
        let classification = if !security_flags.is_empty() {
            "sensitive"
        } else if command.scope_kind == "project" {
            "internal"
        } else {
            "personal"
        };
        let source_hash = content_hash(&command.source_content);
        let source_id = stable_id(&format!("source:{}", command.idempotency_key));
        let candidate_id = stable_id(&format!("candidate:{}", command.idempotency_key));
        let decision_id = stable_id(&format!("decision:{}", command.idempotency_key));
        let record_id = stable_id(&format!("record:{}", command.idempotency_key));
        let version_id = stable_id(&format!("version:{}", command.idempotency_key));
        let memory_event_id = stable_id(&format!("memory-event:{}", command.idempotency_key));
        let operation_id = stable_id(&format!("projection:{}", command.idempotency_key));
        let outbox_id = stable_id(&format!("outbox:{}", command.idempotency_key));
        let version_hash = content_hash(&command.memory_content);
        let semantic_key = command.semantic_key.clone();

        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;

        let existing = sqlx::query(
            "SELECT content_hash,client_event_id,source_type,source_ref,occurred_at \
             FROM memory_core.source_events \
             WHERE tenant_id = $1 AND idempotency_key = $2",
        )
        .bind(&authority.tenant_id)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(existing) = existing {
            let existing_hash: String = existing.try_get("content_hash")?;
            let existing_event_id: String = existing.try_get("client_event_id")?;
            let existing_source_type: String = existing.try_get("source_type")?;
            let existing_source_ref: String = existing.try_get("source_ref")?;
            let existing_occurred_at: chrono::DateTime<chrono::Utc> =
                existing.try_get("occurred_at")?;
            let same_request = existing_hash == source_hash
                && existing_event_id == command.event_id
                && existing_source_type == command.source_type
                && existing_source_ref == command.source_ref
                && existing_occurred_at == command.occurred_at;
            if !same_request {
                return Err(RepositoryError::Conflict(
                    "Idempotency-Key was already used with a different request".into(),
                ));
            }
            let resolved_operation_id =
                load_ingest_operation_id_tx(&mut tx, &authority.tenant_id, source_id)
                    .await?
                    .unwrap_or(operation_id);
            let view =
                load_operation_tx(&mut tx, &authority.tenant_id, resolved_operation_id).await?;
            tx.commit().await?;
            return Ok(view);
        }

        let generation = projection_write_generation_tx(&mut tx, &authority.tenant_id).await?;
        let projection_idempotency_key =
            format!("mem0:{generation}:upsert:{record_id}:{version_id}:0");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "memory-semantic-key:{}:{}:{}:{}:{}",
                authority.tenant_id,
                command.scope_kind,
                scope_id.unwrap_or("_"),
                command.memory_type,
                semantic_key
            ))
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO memory_core.source_events (\
                 tenant_id, id, source_type, source_ref, content_hash, redacted_excerpt,\
                 classification, occurred_at, idempotency_key,client_event_id,\
                 redaction_policy_version,security_flags\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        )
        .bind(&authority.tenant_id)
        .bind(source_id)
        .bind(&command.source_type)
        .bind(&command.source_ref)
        .bind(&source_hash)
        .bind(&source_finding.redacted_text)
        .bind(classification)
        .bind(command.occurred_at)
        .bind(&command.idempotency_key)
        .bind(&command.event_id)
        .bind(SECURITY_POLICY_VERSION)
        .bind(&security_flags)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.memory_candidates (\
                 tenant_id,id,scope_kind,scope_id,memory_type,content,normalized_key,\
                 confidence,importance,inference_kind,extractor_name,extractor_version,\
                 policy_version,state\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,\
                       'demo-policy-v1','accepted')",
        )
        .bind(&authority.tenant_id)
        .bind(candidate_id)
        .bind(&command.scope_kind)
        .bind(scope_id)
        .bind(&command.memory_type)
        .bind(&command.memory_content)
        .bind(&semantic_key)
        .bind(command.confidence)
        .bind(command.importance)
        .bind(&command.inference_kind)
        .bind(&command.extractor_name)
        .bind(&command.extractor_version)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.candidate_sources \
             (tenant_id,candidate_id,source_event_id) VALUES ($1,$2,$3)",
        )
        .bind(&authority.tenant_id)
        .bind(candidate_id)
        .bind(source_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.memory_decisions (\
                 tenant_id,id,candidate_id,outcome,reason_code,actor_type,actor_id,policy_version\
             ) VALUES ($1,$2,$3,'accept',$4,\
                       'policy','demo-policy','demo-policy-v1')",
        )
        .bind(&authority.tenant_id)
        .bind(decision_id)
        .bind(candidate_id)
        .bind(if command.inference_kind == "explicit" {
            "EXPLICIT_LOW_RISK_PREFERENCE"
        } else if command.memory_type == "project_fact" {
            "AUTHORIZED_HIGH_CONFIDENCE_PROJECT_FACT"
        } else {
            "DERIVED_HIGH_CONFIDENCE_MEMORY"
        })
        .execute(&mut *tx)
        .await?;

        let existing_semantic_record = sqlx::query(
            "SELECT r.id,h.current_version_id,h.state,h.revision,v.content_hash \
             FROM memory_core.memory_records r \
             JOIN memory_core.memory_heads h \
               ON h.tenant_id=r.tenant_id AND h.memory_record_id=r.id \
             LEFT JOIN memory_core.memory_versions v \
               ON v.tenant_id=h.tenant_id AND v.id=h.current_version_id \
             WHERE r.tenant_id=$1 AND r.scope_kind=$2 \
               AND r.scope_id IS NOT DISTINCT FROM $3 \
               AND r.memory_type=$4 AND r.semantic_key=$5 \
               AND r.identity_retired_at IS NULL \
             FOR UPDATE OF h",
        )
        .bind(&authority.tenant_id)
        .bind(&command.scope_kind)
        .bind(scope_id)
        .bind(&command.memory_type)
        .bind(&semantic_key)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(existing) = existing_semantic_record {
            let existing_record_id: Uuid = existing.try_get("id")?;
            let existing_version_id = existing
                .try_get::<Option<Uuid>, _>("current_version_id")?
                .ok_or_else(|| {
                    RepositoryError::Invariant(
                        "live semantic identity has no current Version".into(),
                    )
                })?;
            let existing_state: String = existing.try_get("state")?;
            let existing_hash: String = existing.try_get("content_hash")?;
            if existing_state != "active" {
                return Err(RepositoryError::Conflict(format!(
                    "semantic memory is in state {existing_state:?}; review or correct it first"
                )));
            }
            if existing_hash != version_hash {
                let existing_revision: i64 = existing.try_get("revision")?;
                let contested_revision = existing_revision + 1;
                let provider_object_id = sqlx::query_scalar::<_, String>(
                    "SELECT provider_object_id FROM memory_core.projection_operations \
                     WHERE tenant_id=$1 AND memory_record_id=$2 AND memory_version_id=$3 \
                       AND generation=$4 AND status='ready' \
                       AND provider_object_id IS NOT NULL \
                     ORDER BY completed_at DESC NULLS LAST LIMIT 1",
                )
                .bind(&authority.tenant_id)
                .bind(existing_record_id)
                .bind(existing_version_id)
                .bind(&generation)
                .fetch_optional(&mut *tx)
                .await?;

                sqlx::query(
                    "UPDATE memory_core.memory_candidates \
                     SET state='needs_review',target_memory_record_id=$3 \
                     WHERE tenant_id=$1 AND id=$2",
                )
                .bind(&authority.tenant_id)
                .bind(candidate_id)
                .bind(existing_record_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE memory_core.memory_decisions \
                     SET outcome='needs_review',reason_code='SEMANTIC_CONFLICT_REQUIRES_REVIEW' \
                     WHERE tenant_id=$1 AND id=$2",
                )
                .bind(&authority.tenant_id)
                .bind(decision_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE memory_core.memory_heads \
                     SET state='contested',revision=$3 \
                     WHERE tenant_id=$1 AND memory_record_id=$2 AND revision=$4",
                )
                .bind(&authority.tenant_id)
                .bind(existing_record_id)
                .bind(contested_revision)
                .bind(existing_revision)
                .execute(&mut *tx)
                .await?;

                let event_sequence = sqlx::query_scalar::<_, i64>(
                    "SELECT COALESCE(max(sequence_no),0)+1 \
                     FROM memory_core.memory_events \
                     WHERE tenant_id=$1 AND memory_record_id=$2",
                )
                .bind(&authority.tenant_id)
                .bind(existing_record_id)
                .fetch_one(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO memory_core.memory_events (\
                         tenant_id,id,memory_record_id,sequence_no,event_type,memory_version_id,\
                         actor_type,actor_id,reason_code,payload\
                     ) VALUES ($1,$2,$3,$4,'memory_contested',$5,'policy','demo-policy',\
                               'SEMANTIC_CONFLICT_REQUIRES_REVIEW',$6)",
                )
                .bind(&authority.tenant_id)
                .bind(memory_event_id)
                .bind(existing_record_id)
                .bind(event_sequence)
                .bind(existing_version_id)
                .bind(json!({"candidate_id": candidate_id}))
                .execute(&mut *tx)
                .await?;

                sqlx::query(
                    "UPDATE memory_core.context_packs cp \
                     SET invalidated_at=COALESCE(cp.invalidated_at,now()),\
                         invalidation_reason=COALESCE(\
                             cp.invalidation_reason,'MEMORY_CONTESTED'\
                         ) \
                     WHERE cp.tenant_id=$1 AND cp.invalidated_at IS NULL \
                       AND EXISTS (\
                           SELECT 1 FROM memory_core.context_pack_items cpi \
                           WHERE cpi.tenant_id=cp.tenant_id \
                             AND cpi.context_pack_id=cp.id \
                             AND cpi.memory_record_id=$2\
                       )",
                )
                .bind(&authority.tenant_id)
                .bind(existing_record_id)
                .execute(&mut *tx)
                .await?;

                let projection_idempotency_key = format!(
                    "mem0:{generation}:contest-delete:{existing_record_id}:\
                     {existing_version_id}:{contested_revision}"
                );
                sqlx::query(
                    "INSERT INTO memory_core.projection_operations (\
                         tenant_id,id,memory_record_id,memory_version_id,head_revision,provider,\
                         generation,operation_type,idempotency_key,status,provider_object_id,\
                         content_hash\
                     ) VALUES ($1,$2,$3,$4,$5,'mem0',$6,'delete',$7,'pending',$8,$9)",
                )
                .bind(&authority.tenant_id)
                .bind(operation_id)
                .bind(existing_record_id)
                .bind(existing_version_id)
                .bind(contested_revision)
                .bind(&generation)
                .bind(&projection_idempotency_key)
                .bind(provider_object_id)
                .bind(&existing_hash)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO memory_core.outbox_events (\
                         tenant_id,id,aggregate_type,aggregate_id,aggregate_revision,event_type,\
                         payload,ordering_key,idempotency_key\
                     ) VALUES ($1,$2,'memory_record',$3,$4,'projection_delete',$5,$6,$7)",
                )
                .bind(&authority.tenant_id)
                .bind(outbox_id)
                .bind(existing_record_id)
                .bind(contested_revision)
                .bind(json!({"projection_operation_id": operation_id}))
                .bind(existing_record_id.to_string())
                .bind(format!("outbox:{operation_id}"))
                .execute(&mut *tx)
                .await?;
                insert_ingest_result_tx(
                    &mut tx,
                    &authority.tenant_id,
                    source_id,
                    candidate_id,
                    existing_record_id,
                    existing_version_id,
                    operation_id,
                    "needs_review",
                )
                .await?;
                let view = load_operation_tx(&mut tx, &authority.tenant_id, operation_id).await?;
                tx.commit().await?;
                return Ok(view);
            }

            sqlx::query(
                "UPDATE memory_core.memory_candidates SET target_memory_record_id=$3 \
                 WHERE tenant_id=$1 AND id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(candidate_id)
            .bind(existing_record_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE memory_core.memory_decisions \
                 SET reason_code='EXACT_DUPLICATE_SOURCE_ATTACHED' \
                 WHERE tenant_id=$1 AND id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(decision_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO memory_core.memory_version_sources \
                 (tenant_id,memory_version_id,source_event_id) VALUES ($1,$2,$3) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(&authority.tenant_id)
            .bind(existing_version_id)
            .bind(source_id)
            .execute(&mut *tx)
            .await?;
            let existing_operation_id = sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM memory_core.projection_operations \
                 WHERE tenant_id=$1 AND memory_record_id=$2 AND memory_version_id=$3 \
                   AND operation_type='upsert' \
                 ORDER BY created_at DESC,id DESC LIMIT 1",
            )
            .bind(&authority.tenant_id)
            .bind(existing_record_id)
            .bind(existing_version_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                RepositoryError::Invariant("deduplicated memory has no projection operation".into())
            })?;
            insert_ingest_result_tx(
                &mut tx,
                &authority.tenant_id,
                source_id,
                candidate_id,
                existing_record_id,
                existing_version_id,
                existing_operation_id,
                "source_attached",
            )
            .await?;
            let view =
                load_operation_tx(&mut tx, &authority.tenant_id, existing_operation_id).await?;
            tx.commit().await?;
            return Ok(view);
        }

        sqlx::query(
            "INSERT INTO memory_core.memory_records \
                 (tenant_id,id,scope_kind,scope_id,memory_type,semantic_key) \
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(&authority.tenant_id)
        .bind(record_id)
        .bind(&command.scope_kind)
        .bind(scope_id)
        .bind(&command.memory_type)
        .bind(&semantic_key)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE memory_core.memory_candidates SET target_memory_record_id=$3 \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(candidate_id)
        .bind(record_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.memory_versions (\
                 tenant_id,id,memory_record_id,version_no,content,content_hash,normalized_key,\
                 confidence,importance,inference_kind,classification,allowed_purposes,\
                 retention_policy_id,decision_id\
             ) VALUES ($1,$2,$3,1,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
        )
        .bind(&authority.tenant_id)
        .bind(version_id)
        .bind(record_id)
        .bind(&command.memory_content)
        .bind(&version_hash)
        .bind(&semantic_key)
        .bind(command.confidence)
        .bind(command.importance)
        .bind(&command.inference_kind)
        .bind(classification)
        .bind(vec!["answer_user".to_owned()])
        .bind(format!(
            "demo-{}-{}-v1",
            command.scope_kind, command.memory_type
        ))
        .bind(decision_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.memory_version_sources \
             (tenant_id,memory_version_id,source_event_id) VALUES ($1,$2,$3)",
        )
        .bind(&authority.tenant_id)
        .bind(version_id)
        .bind(source_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.memory_events (\
                 tenant_id,id,memory_record_id,sequence_no,event_type,memory_version_id,\
                 actor_type,actor_id,reason_code\
             ) VALUES ($1,$2,$3,1,'memory_accepted',$4,'policy','demo-policy',$5)",
        )
        .bind(&authority.tenant_id)
        .bind(memory_event_id)
        .bind(record_id)
        .bind(version_id)
        .bind(if command.inference_kind == "explicit" {
            "EXPLICIT_LOW_RISK_PREFERENCE"
        } else {
            "DERIVED_HIGH_CONFIDENCE_MEMORY"
        })
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.memory_heads \
             (tenant_id,memory_record_id,current_version_id,state,revision) \
             VALUES ($1,$2,$3,'active',0)",
        )
        .bind(&authority.tenant_id)
        .bind(record_id)
        .bind(version_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.projection_operations (\
                 tenant_id,id,memory_record_id,memory_version_id,head_revision,provider,\
                 generation,operation_type,idempotency_key,status,content_hash\
             ) VALUES ($1,$2,$3,$4,0,'mem0',$5,'upsert',$6,'pending',$7)",
        )
        .bind(&authority.tenant_id)
        .bind(operation_id)
        .bind(record_id)
        .bind(version_id)
        .bind(&generation)
        .bind(&projection_idempotency_key)
        .bind(&version_hash)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.outbox_events (\
                 tenant_id,id,aggregate_type,aggregate_id,aggregate_revision,event_type,\
                 payload,ordering_key,idempotency_key\
             ) VALUES ($1,$2,'memory_record',$3,0,'projection_upsert',$4,$5,$6)",
        )
        .bind(&authority.tenant_id)
        .bind(outbox_id)
        .bind(record_id)
        .bind(json!({"projection_operation_id": operation_id}))
        .bind(record_id.to_string())
        .bind(format!("outbox:{operation_id}"))
        .execute(&mut *tx)
        .await?;

        insert_ingest_result_tx(
            &mut tx,
            &authority.tenant_id,
            source_id,
            candidate_id,
            record_id,
            version_id,
            operation_id,
            "created",
        )
        .await?;

        let view = load_operation_tx(&mut tx, &authority.tenant_id, operation_id).await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn correct(
        &self,
        authority: &DemoAuthority,
        mut command: CorrectionCommand,
    ) -> Result<OperationView, RepositoryError> {
        let source_id = stable_id(&format!("correction-source:{}", command.idempotency_key));
        let decision_id = stable_id(&format!("correction-decision:{}", command.idempotency_key));
        let version_id = stable_id(&format!("correction-version:{}", command.idempotency_key));
        let event_id = stable_id(&format!("correction-event:{}", command.idempotency_key));
        let operation_id = stable_id(&format!(
            "correction-projection:{}",
            command.idempotency_key
        ));
        let outbox_id = stable_id(&format!("correction-outbox:{}", command.idempotency_key));
        let request_hash = correction_request_hash(&command);
        let security_finding = SECURITY_PIPELINE.inspect(&command.new_content);
        ensure_safe_for_storage(&security_finding)?;
        command.new_content = security_finding.redacted_text.clone();
        let version_hash = content_hash(&command.new_content);

        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "memory-correction-idempotency:{}:{}",
                authority.tenant_id, command.idempotency_key
            ))
            .execute(&mut *tx)
            .await?;
        if let Some(existing) = sqlx::query(
            "SELECT content_hash,source_type,source_ref \
             FROM memory_core.source_events \
             WHERE tenant_id=$1 AND idempotency_key=$2",
        )
        .bind(&authority.tenant_id)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?
        {
            let same_request = existing.try_get::<String, _>("content_hash")? == request_hash
                && existing.try_get::<String, _>("source_type")? == "user_correction"
                && existing.try_get::<String, _>("source_ref")? == command.source_ref;
            if !same_request {
                return Err(RepositoryError::Conflict(
                    "Idempotency-Key was already used with a different correction".into(),
                ));
            }
            let view = load_operation_tx(&mut tx, &authority.tenant_id, operation_id).await?;
            tx.commit().await?;
            return Ok(view);
        }

        let generation = projection_write_generation_tx(&mut tx, &authority.tenant_id).await?;
        let record = sqlx::query(
            "SELECT scope_kind,scope_id,semantic_key \
             FROM memory_core.memory_records \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepositoryError::NotFound)?;
        let scope_kind: String = record.try_get("scope_kind")?;
        let scope_id: Option<String> = record.try_get("scope_id")?;
        if !authority.authorizes_scope(&scope_kind, scope_id.as_deref()) {
            return Err(RepositoryError::NotFound);
        }
        let head = sqlx::query(
            "SELECT revision,state,current_version_id \
             FROM memory_core.memory_heads \
             WHERE tenant_id=$1 AND memory_record_id=$2 FOR UPDATE",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepositoryError::NotFound)?;
        let revision: i64 = head.try_get("revision")?;
        if revision != command.expected_revision {
            return Err(RepositoryError::Conflict(format!(
                "expected revision {}, current revision is {revision}",
                command.expected_revision
            )));
        }
        let state: String = head.try_get("state")?;
        if state != "active" {
            return Err(RepositoryError::Conflict(format!(
                "memory cannot be corrected from state {state:?}"
            )));
        }
        let previous_version_id: Uuid = head.try_get("current_version_id")?;
        let current_version = sqlx::query(
            "SELECT version_no,content_hash,confidence,importance,classification,\
                    allowed_purposes,consent_ref,retention_policy_id,retain_until \
             FROM memory_core.memory_versions \
             WHERE tenant_id=$1 AND memory_record_id=$2 AND id=$3",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .bind(previous_version_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| RepositoryError::Invariant("active Head has no current Version".into()))?;
        let previous_hash: String = current_version.try_get("content_hash")?;
        if previous_hash == version_hash {
            return Err(RepositoryError::Conflict(
                "correction content is identical to the current Version".into(),
            ));
        }
        let semantic_key: String = record.try_get("semantic_key")?;
        let new_revision = revision + 1;
        let new_version_no = current_version.try_get::<i32, _>("version_no")? + 1;
        let occurred_at = command.effective_from.unwrap_or_else(Utc::now);

        sqlx::query(
            "INSERT INTO memory_core.source_events (\
                 tenant_id,id,source_type,source_ref,content_hash,redacted_excerpt,\
                 classification,occurred_at,idempotency_key,client_event_id,\
                 redaction_policy_version,security_flags\
             ) VALUES ($1,$2,'user_correction',$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(&authority.tenant_id)
        .bind(source_id)
        .bind(&command.source_ref)
        .bind(&request_hash)
        .bind(&command.new_content)
        .bind(if security_finding.flags.is_empty() {
            "personal"
        } else {
            "sensitive"
        })
        .bind(occurred_at)
        .bind(&command.idempotency_key)
        .bind(format!("correction-{}", command.idempotency_key))
        .bind(SECURITY_POLICY_VERSION)
        .bind(&security_finding.flags)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.memory_decisions (\
                 tenant_id,id,candidate_id,outcome,reason_code,actor_type,actor_id,policy_version\
             ) VALUES ($1,$2,NULL,'accept','USER_EXPLICIT_CORRECTION','user',$3,\
                       'demo-policy-v1')",
        )
        .bind(&authority.tenant_id)
        .bind(decision_id)
        .bind(&authority.user_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.memory_versions (\
                 tenant_id,id,memory_record_id,version_no,content,content_hash,normalized_key,\
                 confidence,importance,inference_kind,effective_from,classification,\
                 allowed_purposes,consent_ref,retention_policy_id,retain_until,decision_id\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'explicit',$10,$11,$12,$13,$14,$15,$16)",
        )
        .bind(&authority.tenant_id)
        .bind(version_id)
        .bind(command.memory_record_id)
        .bind(new_version_no)
        .bind(&command.new_content)
        .bind(&version_hash)
        .bind(&semantic_key)
        .bind(current_version.try_get::<f64, _>("confidence")?)
        .bind(current_version.try_get::<f64, _>("importance")?)
        .bind(command.effective_from)
        .bind(current_version.try_get::<String, _>("classification")?)
        .bind(current_version.try_get::<Vec<String>, _>("allowed_purposes")?)
        .bind(current_version.try_get::<Option<String>, _>("consent_ref")?)
        .bind(current_version.try_get::<String, _>("retention_policy_id")?)
        .bind(current_version.try_get::<Option<DateTime<Utc>>, _>("retain_until")?)
        .bind(decision_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.memory_version_sources \
             (tenant_id,memory_version_id,source_event_id) VALUES ($1,$2,$3)",
        )
        .bind(&authority.tenant_id)
        .bind(version_id)
        .bind(source_id)
        .execute(&mut *tx)
        .await?;

        let event_sequence = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(max(sequence_no),0)+1 FROM memory_core.memory_events \
             WHERE tenant_id=$1 AND memory_record_id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO memory_core.memory_events (\
                 tenant_id,id,memory_record_id,sequence_no,event_type,memory_version_id,\
                 actor_type,actor_id,reason_code,payload\
             ) VALUES ($1,$2,$3,$4,'memory_corrected',$5,'user',$6,\
                       'USER_EXPLICIT_CORRECTION',$7)",
        )
        .bind(&authority.tenant_id)
        .bind(event_id)
        .bind(command.memory_record_id)
        .bind(event_sequence)
        .bind(version_id)
        .bind(&authority.user_id)
        .bind(json!({
            "reason_hash": content_hash(&command.reason),
            "supersedes_version_id": previous_version_id
        }))
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE memory_core.memory_heads \
             SET current_version_id=$3,state='active',revision=$4 \
             WHERE tenant_id=$1 AND memory_record_id=$2 AND revision=$5",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .bind(version_id)
        .bind(new_revision)
        .bind(revision)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE memory_core.context_packs cp \
             SET invalidated_at=COALESCE(cp.invalidated_at,now()),\
                 invalidation_reason=COALESCE(cp.invalidation_reason,'MEMORY_CORRECTED') \
             WHERE cp.tenant_id=$1 AND cp.invalidated_at IS NULL \
               AND EXISTS (\
                   SELECT 1 FROM memory_core.context_pack_items cpi \
                   WHERE cpi.tenant_id=cp.tenant_id \
                     AND cpi.context_pack_id=cp.id \
                     AND cpi.memory_record_id=$2\
               )",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .execute(&mut *tx)
        .await?;

        let projection_idempotency_key = format!(
            "mem0:{generation}:upsert:{}:{version_id}:{new_revision}",
            command.memory_record_id
        );
        sqlx::query(
            "INSERT INTO memory_core.projection_operations (\
                 tenant_id,id,memory_record_id,memory_version_id,head_revision,provider,\
                 generation,operation_type,idempotency_key,status,content_hash\
             ) VALUES ($1,$2,$3,$4,$5,'mem0',$6,'upsert',$7,'pending',$8)",
        )
        .bind(&authority.tenant_id)
        .bind(operation_id)
        .bind(command.memory_record_id)
        .bind(version_id)
        .bind(new_revision)
        .bind(&generation)
        .bind(&projection_idempotency_key)
        .bind(&version_hash)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.outbox_events (\
                 tenant_id,id,aggregate_type,aggregate_id,aggregate_revision,event_type,\
                 payload,ordering_key,idempotency_key\
             ) VALUES ($1,$2,'memory_record',$3,$4,'projection_upsert',$5,$6,$7)",
        )
        .bind(&authority.tenant_id)
        .bind(outbox_id)
        .bind(command.memory_record_id)
        .bind(new_revision)
        .bind(json!({"projection_operation_id": operation_id}))
        .bind(command.memory_record_id.to_string())
        .bind(format!("outbox:{operation_id}"))
        .execute(&mut *tx)
        .await?;

        let view = load_operation_tx(&mut tx, &authority.tenant_id, operation_id).await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn decide_candidate(
        &self,
        authority: &DemoAuthority,
        command: ReviewDecisionCommand,
    ) -> Result<OperationView, RepositoryError> {
        let decision_id = stable_id(&format!("review-decision:{}", command.idempotency_key));
        let version_id = stable_id(&format!("review-version:{}", command.idempotency_key));
        let event_id = stable_id(&format!("review-event:{}", command.idempotency_key));
        let operation_id = stable_id(&format!("review-projection:{}", command.idempotency_key));
        let outbox_id = stable_id(&format!("review-outbox:{}", command.idempotency_key));
        let request_hash = review_request_hash(&command);

        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "memory-review-idempotency:{}:{}",
                authority.tenant_id, command.idempotency_key
            ))
            .execute(&mut *tx)
            .await?;
        if let Some(existing) = sqlx::query(
            "SELECT request_hash,candidate_id,outcome,operation_id \
             FROM memory_core.memory_review_results \
             WHERE tenant_id=$1 AND idempotency_key=$2",
        )
        .bind(&authority.tenant_id)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?
        {
            let same_request = existing.try_get::<String, _>("request_hash")? == request_hash
                && existing.try_get::<Uuid, _>("candidate_id")? == command.candidate_id
                && existing.try_get::<String, _>("outcome")? == command.outcome.as_str();
            if !same_request {
                return Err(RepositoryError::Conflict(
                    "Idempotency-Key was already used with a different review decision".into(),
                ));
            }
            let existing_operation_id: Uuid = existing.try_get("operation_id")?;
            let view =
                load_operation_tx(&mut tx, &authority.tenant_id, existing_operation_id).await?;
            tx.commit().await?;
            return Ok(view);
        }

        let generation = projection_write_generation_tx(&mut tx, &authority.tenant_id).await?;
        let candidate_identity = sqlx::query(
            "SELECT c.target_memory_record_id,r.scope_kind,r.scope_id \
             FROM memory_core.memory_candidates c \
             JOIN memory_core.memory_records r \
               ON r.tenant_id=c.tenant_id AND r.id=c.target_memory_record_id \
             WHERE c.tenant_id=$1 AND c.id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(command.candidate_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepositoryError::NotFound)?;
        let scope_kind: String = candidate_identity.try_get("scope_kind")?;
        let scope_id: Option<String> = candidate_identity.try_get("scope_id")?;
        if !authority.authorizes_scope(&scope_kind, scope_id.as_deref()) {
            return Err(RepositoryError::NotFound);
        }
        let record_id: Uuid = candidate_identity.try_get("target_memory_record_id")?;
        let head = sqlx::query(
            "SELECT revision,state,current_version_id \
             FROM memory_core.memory_heads \
             WHERE tenant_id=$1 AND memory_record_id=$2 FOR UPDATE",
        )
        .bind(&authority.tenant_id)
        .bind(record_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepositoryError::NotFound)?;
        let candidate = sqlx::query(
            "SELECT revision,state,content,normalized_key,confidence,importance,inference_kind,\
                    target_memory_record_id \
             FROM memory_core.memory_candidates \
             WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
        )
        .bind(&authority.tenant_id)
        .bind(command.candidate_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepositoryError::NotFound)?;
        if candidate.try_get::<Uuid, _>("target_memory_record_id")? != record_id {
            return Err(RepositoryError::Conflict(
                "candidate target changed during review".into(),
            ));
        }
        let candidate_revision: i64 = candidate.try_get("revision")?;
        if candidate_revision != command.expected_candidate_revision {
            return Err(RepositoryError::Conflict(format!(
                "expected candidate revision {}, current revision is {candidate_revision}",
                command.expected_candidate_revision
            )));
        }
        let candidate_state: String = candidate.try_get("state")?;
        if candidate_state != "needs_review" {
            return Err(RepositoryError::Conflict(format!(
                "candidate cannot be decided from state {candidate_state:?}"
            )));
        }
        let head_state: String = head.try_get("state")?;
        if head_state != "contested" {
            return Err(RepositoryError::Conflict(format!(
                "candidate target Head is not contested; current state is {head_state:?}"
            )));
        }
        let current_version_id: Uuid = head.try_get("current_version_id")?;
        let head_revision: i64 = head.try_get("revision")?;
        let current_version = sqlx::query(
            "SELECT version_no,content,content_hash,classification,allowed_purposes,\
                    consent_ref,retention_policy_id,retain_until \
             FROM memory_core.memory_versions \
             WHERE tenant_id=$1 AND memory_record_id=$2 AND id=$3",
        )
        .bind(&authority.tenant_id)
        .bind(record_id)
        .bind(current_version_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            RepositoryError::Invariant("contested Head has no current Version".into())
        })?;
        let current_version_no: i32 = current_version.try_get("version_no")?;
        let resolved_revision = head_revision + 1;
        let reason_code = match command.outcome {
            ReviewOutcome::Accept => "REVIEW_ACCEPTED_CONFLICTING_VALUE",
            ReviewOutcome::Reject => "REVIEW_REJECTED_CONFLICTING_VALUE",
        };

        sqlx::query(
            "INSERT INTO memory_core.memory_decisions (\
                 tenant_id,id,candidate_id,outcome,reason_code,actor_type,actor_id,policy_version\
             ) VALUES ($1,$2,$3,$4,$5,'reviewer',$6,'demo-policy-v1')",
        )
        .bind(&authority.tenant_id)
        .bind(decision_id)
        .bind(command.candidate_id)
        .bind(command.outcome.as_str())
        .bind(reason_code)
        .bind(&authority.user_id)
        .execute(&mut *tx)
        .await?;
        let final_candidate_state = match command.outcome {
            ReviewOutcome::Accept => "accepted",
            ReviewOutcome::Reject => "rejected",
        };
        let updated = sqlx::query(
            "UPDATE memory_core.memory_candidates \
             SET state=$3,revision=revision+1 \
             WHERE tenant_id=$1 AND id=$2 AND revision=$4 AND state='needs_review'",
        )
        .bind(&authority.tenant_id)
        .bind(command.candidate_id)
        .bind(final_candidate_state)
        .bind(candidate_revision)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(RepositoryError::Conflict(
                "candidate revision changed during review".into(),
            ));
        }

        let (resolved_version_id, resolved_content, resolved_hash) = match command.outcome {
            ReviewOutcome::Accept => {
                let candidate_content: String = candidate.try_get("content")?;
                let candidate_hash = content_hash(&candidate_content);
                let effective_from = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
                    "SELECT max(se.occurred_at) \
                     FROM memory_core.candidate_sources cs \
                     JOIN memory_core.source_events se \
                       ON se.tenant_id=cs.tenant_id AND se.id=cs.source_event_id \
                     WHERE cs.tenant_id=$1 AND cs.candidate_id=$2",
                )
                .bind(&authority.tenant_id)
                .bind(command.candidate_id)
                .fetch_one(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO memory_core.memory_versions (\
                         tenant_id,id,memory_record_id,version_no,content,content_hash,\
                         normalized_key,confidence,importance,inference_kind,effective_from,\
                         classification,allowed_purposes,consent_ref,retention_policy_id,\
                         retain_until,decision_id\
                     ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
                )
                .bind(&authority.tenant_id)
                .bind(version_id)
                .bind(record_id)
                .bind(current_version_no + 1)
                .bind(&candidate_content)
                .bind(&candidate_hash)
                .bind(candidate.try_get::<Option<String>, _>("normalized_key")?)
                .bind(candidate.try_get::<f64, _>("confidence")?)
                .bind(candidate.try_get::<f64, _>("importance")?)
                .bind(candidate.try_get::<String, _>("inference_kind")?)
                .bind(effective_from)
                .bind(current_version.try_get::<String, _>("classification")?)
                .bind(current_version.try_get::<Vec<String>, _>("allowed_purposes")?)
                .bind(current_version.try_get::<Option<String>, _>("consent_ref")?)
                .bind(current_version.try_get::<String, _>("retention_policy_id")?)
                .bind(current_version.try_get::<Option<DateTime<Utc>>, _>("retain_until")?)
                .bind(decision_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO memory_core.memory_version_sources \
                     (tenant_id,memory_version_id,source_event_id) \
                     SELECT tenant_id,$3,source_event_id \
                     FROM memory_core.candidate_sources \
                     WHERE tenant_id=$1 AND candidate_id=$2",
                )
                .bind(&authority.tenant_id)
                .bind(command.candidate_id)
                .bind(version_id)
                .execute(&mut *tx)
                .await?;
                (version_id, candidate_content, candidate_hash)
            }
            ReviewOutcome::Reject => (
                current_version_id,
                current_version.try_get::<String, _>("content")?,
                current_version.try_get::<String, _>("content_hash")?,
            ),
        };

        sqlx::query(
            "UPDATE memory_core.memory_heads \
             SET current_version_id=$3,state='active',revision=$4 \
             WHERE tenant_id=$1 AND memory_record_id=$2 AND revision=$5 \
               AND state='contested'",
        )
        .bind(&authority.tenant_id)
        .bind(record_id)
        .bind(resolved_version_id)
        .bind(resolved_revision)
        .bind(head_revision)
        .execute(&mut *tx)
        .await?;
        let event_sequence = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(max(sequence_no),0)+1 FROM memory_core.memory_events \
             WHERE tenant_id=$1 AND memory_record_id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(record_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO memory_core.memory_events (\
                 tenant_id,id,memory_record_id,sequence_no,event_type,memory_version_id,\
                 actor_type,actor_id,reason_code,payload\
             ) VALUES ($1,$2,$3,$4,'memory_contest_resolved',$5,'reviewer',$6,$7,$8)",
        )
        .bind(&authority.tenant_id)
        .bind(event_id)
        .bind(record_id)
        .bind(event_sequence)
        .bind(resolved_version_id)
        .bind(&authority.user_id)
        .bind(reason_code)
        .bind(json!({
            "candidate_id": command.candidate_id,
            "outcome": command.outcome.as_str(),
            "reason_hash": content_hash(&command.reason)
        }))
        .execute(&mut *tx)
        .await?;

        let projection_idempotency_key = format!(
            "mem0:{generation}:review-upsert:{record_id}:\
             {resolved_version_id}:{resolved_revision}"
        );
        sqlx::query(
            "INSERT INTO memory_core.projection_operations (\
                 tenant_id,id,memory_record_id,memory_version_id,head_revision,provider,\
                 generation,operation_type,idempotency_key,status,content_hash\
             ) VALUES ($1,$2,$3,$4,$5,'mem0',$6,'upsert',$7,'pending',$8)",
        )
        .bind(&authority.tenant_id)
        .bind(operation_id)
        .bind(record_id)
        .bind(resolved_version_id)
        .bind(resolved_revision)
        .bind(&generation)
        .bind(&projection_idempotency_key)
        .bind(&resolved_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO memory_core.outbox_events (\
                 tenant_id,id,aggregate_type,aggregate_id,aggregate_revision,event_type,\
                 payload,ordering_key,idempotency_key\
             ) VALUES ($1,$2,'memory_record',$3,$4,'projection_upsert',$5,$6,$7)",
        )
        .bind(&authority.tenant_id)
        .bind(outbox_id)
        .bind(record_id)
        .bind(resolved_revision)
        .bind(json!({"projection_operation_id": operation_id}))
        .bind(record_id.to_string())
        .bind(format!("outbox:{operation_id}"))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO memory_core.memory_review_results (\
                 tenant_id,idempotency_key,request_hash,candidate_id,outcome,\
                 memory_record_id,memory_version_id,operation_id\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(&authority.tenant_id)
        .bind(&command.idempotency_key)
        .bind(&request_hash)
        .bind(command.candidate_id)
        .bind(command.outcome.as_str())
        .bind(record_id)
        .bind(resolved_version_id)
        .bind(operation_id)
        .execute(&mut *tx)
        .await?;

        let view = load_operation_tx(&mut tx, &authority.tenant_id, operation_id).await?;
        if view.memory_version_id != Some(resolved_version_id) || resolved_content.trim().is_empty()
        {
            return Err(RepositoryError::Invariant(
                "review resolution produced an invalid current Version".into(),
            ));
        }
        tx.commit().await?;
        Ok(view)
    }

    pub async fn forget(
        &self,
        authority: &DemoAuthority,
        command: ForgetCommand,
    ) -> Result<OperationView, RepositoryError> {
        let operation_id = stable_id(&format!(
            "forget-projection:{}:{}",
            command.memory_record_id, command.idempotency_key
        ));
        let outbox_id = stable_id(&format!(
            "forget-outbox:{}:{}",
            command.memory_record_id, command.idempotency_key
        ));
        let event_id = stable_id(&format!(
            "forget-event:{}:{}",
            command.memory_record_id, command.idempotency_key
        ));

        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        if operation_exists_tx(&mut tx, &authority.tenant_id, operation_id).await? {
            let view = load_operation_tx(&mut tx, &authority.tenant_id, operation_id).await?;
            tx.commit().await?;
            return Ok(view);
        }

        let generation = invalidate_projection_maintenance_tx(
            &mut tx,
            &authority.tenant_id,
            "GOVERNANCE_MUTATION_DURING_REBUILD",
        )
        .await?;
        let row = sqlx::query(
            "SELECT h.revision,h.state,h.current_version_id,r.scope_kind,r.scope_id,\
                    v.content_hash \
             FROM memory_core.memory_heads h \
             JOIN memory_core.memory_records r \
               ON r.tenant_id=h.tenant_id AND r.id=h.memory_record_id \
             LEFT JOIN memory_core.memory_versions v \
               ON v.tenant_id=h.tenant_id AND v.id=h.current_version_id \
             WHERE h.tenant_id=$1 AND h.memory_record_id=$2 FOR UPDATE OF h",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepositoryError::NotFound)?;
        let revision: i64 = row.try_get("revision")?;
        let state: String = row.try_get("state")?;
        let scope_kind: String = row.try_get("scope_kind")?;
        let scope_id: Option<String> = row.try_get("scope_id")?;
        if !authority.authorizes_scope(&scope_kind, scope_id.as_deref()) {
            return Err(RepositoryError::NotFound);
        }
        if revision != command.expected_revision {
            return Err(RepositoryError::Conflict(format!(
                "expected revision {}, current revision is {revision}",
                command.expected_revision
            )));
        }
        if state != "active" {
            return Err(RepositoryError::Conflict(format!(
                "memory cannot be forgotten from state {state:?}"
            )));
        }
        let version_id: Uuid = row
            .try_get::<Option<Uuid>, _>("current_version_id")?
            .ok_or_else(|| RepositoryError::Invariant("active Head has no Version".into()))?;
        let version_hash: String = row.try_get("content_hash")?;
        let new_revision = revision + 1;
        let provider_object_id = sqlx::query_scalar::<_, String>(
            "SELECT provider_object_id FROM memory_core.projection_operations \
             WHERE tenant_id=$1 AND memory_record_id=$2 AND memory_version_id=$3 \
               AND generation=$4 AND status='ready' AND provider_object_id IS NOT NULL \
             ORDER BY completed_at DESC NULLS LAST LIMIT 1",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .bind(version_id)
        .bind(&generation)
        .fetch_optional(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE memory_core.memory_heads \
             SET current_version_id=NULL,state='forgotten',revision=$3 \
             WHERE tenant_id=$1 AND memory_record_id=$2 AND revision=$4",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .bind(new_revision)
        .bind(revision)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE memory_core.memory_records SET identity_retired_at=now() \
             WHERE tenant_id=$1 AND id=$2 AND identity_retired_at IS NULL",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.tombstones (\
                 tenant_id,memory_record_id,tombstone_revision,minimum_generation,reason_code\
             ) VALUES ($1,$2,$3,$4,'USER_REQUESTED_FORGET')",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .bind(new_revision)
        .bind(&generation)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.memory_events (\
                 tenant_id,id,memory_record_id,sequence_no,event_type,memory_version_id,\
                 actor_type,actor_id,reason_code,payload\
             ) VALUES ($1,$2,$3,$4,'memory_forgotten',$5,'user',$6,\
                       'USER_REQUESTED_FORGET',$7)",
        )
        .bind(&authority.tenant_id)
        .bind(event_id)
        .bind(command.memory_record_id)
        .bind(new_revision + 1)
        .bind(version_id)
        .bind(&authority.user_id)
        .bind(json!({"reason": command.reason}))
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE memory_core.context_packs cp \
             SET invalidated_at=COALESCE(cp.invalidated_at,now()),\
                 invalidation_reason=COALESCE(cp.invalidation_reason,'MEMORY_FORGOTTEN') \
             WHERE cp.tenant_id=$1 AND cp.invalidated_at IS NULL \
               AND EXISTS (\
                   SELECT 1 FROM memory_core.context_pack_items cpi \
                   WHERE cpi.tenant_id=cp.tenant_id \
                     AND cpi.context_pack_id=cp.id \
                     AND cpi.memory_record_id=$2\
               )",
        )
        .bind(&authority.tenant_id)
        .bind(command.memory_record_id)
        .execute(&mut *tx)
        .await?;

        let projection_idempotency_key = format!(
            "mem0:{generation}:delete:{}:{version_id}:{new_revision}",
            command.memory_record_id
        );
        sqlx::query(
            "INSERT INTO memory_core.projection_operations (\
                 tenant_id,id,memory_record_id,memory_version_id,head_revision,provider,\
                 generation,operation_type,idempotency_key,status,provider_object_id,content_hash\
             ) VALUES ($1,$2,$3,$4,$5,'mem0',$6,'delete',$7,'pending',$8,$9)",
        )
        .bind(&authority.tenant_id)
        .bind(operation_id)
        .bind(command.memory_record_id)
        .bind(version_id)
        .bind(new_revision)
        .bind(&generation)
        .bind(projection_idempotency_key)
        .bind(provider_object_id)
        .bind(version_hash)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO memory_core.outbox_events (\
                 tenant_id,id,aggregate_type,aggregate_id,aggregate_revision,event_type,\
                 payload,ordering_key,idempotency_key\
             ) VALUES ($1,$2,'memory_record',$3,$4,'projection_delete',$5,$6,$7)",
        )
        .bind(&authority.tenant_id)
        .bind(outbox_id)
        .bind(command.memory_record_id)
        .bind(new_revision)
        .bind(json!({"projection_operation_id": operation_id}))
        .bind(command.memory_record_id.to_string())
        .bind(format!("outbox:{operation_id}"))
        .execute(&mut *tx)
        .await?;

        let view = load_operation_tx(&mut tx, &authority.tenant_id, operation_id).await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn operation(
        &self,
        authority: &DemoAuthority,
        operation_id: Uuid,
    ) -> Result<OperationView, RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let authorized = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(\
                 SELECT 1 \
                 FROM memory_core.projection_operations po \
                 JOIN memory_core.memory_records r \
                   ON r.tenant_id=po.tenant_id AND r.id=po.memory_record_id \
                 WHERE po.tenant_id=$1 AND po.id=$2 \
                   AND (\
                       (r.scope_kind='tenant' AND $3) \
                       OR (r.scope_kind='user' AND r.scope_id=$4) \
                       OR (r.scope_kind='project' AND r.scope_id=ANY($5)) \
                       OR (r.scope_kind='agent' AND r.scope_id=ANY($6)) \
                       OR EXISTS (\
                           SELECT 1 \
                           FROM memory_core.erasure_request_records err \
                           JOIN memory_core.erasure_requests er \
                             ON er.tenant_id=err.tenant_id \
                            AND er.id=err.erasure_request_id \
                           WHERE err.tenant_id=po.tenant_id \
                             AND err.projection_operation_id=po.id \
                             AND er.requested_by_hash=$7\
                       )\
                   )\
             )",
        )
        .bind(&authority.tenant_id)
        .bind(operation_id)
        .bind(authority.tenant_scope)
        .bind(&authority.user_id)
        .bind(authority.project_ids.iter().cloned().collect::<Vec<_>>())
        .bind(authority.agent_ids.iter().cloned().collect::<Vec<_>>())
        .bind(principal_hash(authority))
        .fetch_one(&mut *tx)
        .await?;
        if !authorized {
            return Err(RepositoryError::NotFound);
        }
        let view = load_operation_tx(&mut tx, &authority.tenant_id, operation_id).await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn inspect_memory(
        &self,
        authority: &DemoAuthority,
        memory_record_id: Uuid,
    ) -> Result<MemoryRecordView, RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let view = load_memory_record_view_tx(&mut tx, authority, memory_record_id).await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn export_memories(
        &self,
        authority: &DemoAuthority,
    ) -> Result<MemoryExportView, RepositoryError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let generated_at = sqlx::query_scalar::<_, DateTime<Utc>>("SELECT transaction_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        let record_ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT r.id \
             FROM memory_core.memory_records r \
             JOIN memory_core.memory_heads h \
               ON h.tenant_id=r.tenant_id AND h.memory_record_id=r.id \
             WHERE r.tenant_id=$1 \
               AND (\
                    (r.scope_kind='tenant' AND $2) \
                 OR (r.scope_kind='user' AND r.scope_id=$3) \
                 OR (r.scope_kind='project' AND r.scope_id=ANY($4)) \
                 OR (r.scope_kind='agent' AND r.scope_id=ANY($5))\
               ) \
               AND h.state<>'erased' \
             ORDER BY r.created_at,r.id",
        )
        .bind(&authority.tenant_id)
        .bind(authority.tenant_scope)
        .bind(&authority.user_id)
        .bind(authority.project_ids.iter().cloned().collect::<Vec<_>>())
        .bind(authority.agent_ids.iter().cloned().collect::<Vec<_>>())
        .fetch_all(&mut *tx)
        .await?;
        let mut records = Vec::with_capacity(record_ids.len());
        for memory_record_id in record_ids {
            records.push(load_memory_record_view_tx(&mut tx, authority, memory_record_id).await?);
        }
        let view = MemoryExportView {
            format_version: "lazytt-memory-export-v1".into(),
            generated_at,
            record_count: records.len(),
            records,
        };
        tx.commit().await?;
        Ok(view)
    }

    pub async fn expire_due(
        &self,
        authority: &DemoAuthority,
        limit: usize,
    ) -> Result<Vec<OperationView>, RepositoryError> {
        if !(1..=1_000).contains(&limit) {
            return Err(RepositoryError::Conflict(
                "expiry sweep limit must be between 1 and 1000".into(),
            ));
        }
        let limit = i64::try_from(limit)
            .map_err(|_| RepositoryError::Invariant("expiry sweep limit overflow".into()))?;
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        lock_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        ensure_default_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        if projection_maintenance_in_progress_tx(&mut tx, &authority.tenant_id).await? {
            tx.commit().await?;
            return Ok(Vec::new());
        }
        let generation = active_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        let due = sqlx::query(
            "SELECT h.memory_record_id,h.revision,h.current_version_id,v.content_hash,\
                    CASE \
                      WHEN v.retain_until IS NOT NULL \
                       AND v.retain_until<=transaction_timestamp() \
                       AND (v.effective_to IS NULL OR v.retain_until<=v.effective_to) \
                      THEN 'RETENTION_EXPIRED' \
                      ELSE 'EFFECTIVE_PERIOD_ENDED' \
                    END AS reason_code \
             FROM memory_core.memory_heads h \
             JOIN memory_core.memory_versions v \
               ON v.tenant_id=h.tenant_id AND v.id=h.current_version_id \
             WHERE h.tenant_id=$1 AND h.state='active' \
               AND (\
                    (v.retain_until IS NOT NULL \
                     AND v.retain_until<=transaction_timestamp()) \
                 OR (v.effective_to IS NOT NULL \
                     AND v.effective_to<=transaction_timestamp())\
               ) \
             ORDER BY LEAST(\
                    COALESCE(v.retain_until,'infinity'::timestamptz),\
                    COALESCE(v.effective_to,'infinity'::timestamptz)\
                 ),h.memory_record_id \
             LIMIT $2 FOR UPDATE OF h SKIP LOCKED",
        )
        .bind(&authority.tenant_id)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        let mut operations = Vec::with_capacity(due.len());
        for row in due {
            let memory_record_id: Uuid = row.try_get("memory_record_id")?;
            let memory_version_id: Uuid = row.try_get("current_version_id")?;
            let content_hash: String = row.try_get("content_hash")?;
            let reason_code: String = row.try_get("reason_code")?;
            let current_revision: i64 = row.try_get("revision")?;
            let expired_revision = current_revision + 1;
            let operation_id = stable_id(&format!(
                "expiry-projection:{memory_record_id}:{memory_version_id}:{expired_revision}"
            ));
            let event_id = stable_id(&format!(
                "expiry-event:{memory_record_id}:{memory_version_id}:{expired_revision}"
            ));
            let outbox_id = stable_id(&format!(
                "expiry-outbox:{memory_record_id}:{memory_version_id}:{expired_revision}"
            ));
            let provider_object_id = sqlx::query_scalar::<_, String>(
                "SELECT provider_object_id FROM memory_core.projection_operations \
                 WHERE tenant_id=$1 AND memory_record_id=$2 AND memory_version_id=$3 \
                   AND generation=$4 AND status='ready' \
                   AND provider_object_id IS NOT NULL \
                 ORDER BY completed_at DESC NULLS LAST LIMIT 1",
            )
            .bind(&authority.tenant_id)
            .bind(memory_record_id)
            .bind(memory_version_id)
            .bind(&generation)
            .fetch_optional(&mut *tx)
            .await?;

            let updated = sqlx::query(
                "UPDATE memory_core.memory_heads \
                 SET state='expired',revision=$3 \
                 WHERE tenant_id=$1 AND memory_record_id=$2 \
                   AND revision=$4 AND state='active'",
            )
            .bind(&authority.tenant_id)
            .bind(memory_record_id)
            .bind(expired_revision)
            .bind(current_revision)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(RepositoryError::Conflict(
                    "memory Head changed during expiry sweep".into(),
                ));
            }
            sqlx::query(
                "UPDATE memory_core.memory_records \
                 SET identity_retired_at=COALESCE(identity_retired_at,transaction_timestamp()) \
                 WHERE tenant_id=$1 AND id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(memory_record_id)
            .execute(&mut *tx)
            .await?;
            let event_sequence = sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(max(sequence_no),0)+1 \
                 FROM memory_core.memory_events \
                 WHERE tenant_id=$1 AND memory_record_id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(memory_record_id)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO memory_core.memory_events (\
                     tenant_id,id,memory_record_id,sequence_no,event_type,memory_version_id,\
                     actor_type,actor_id,reason_code,payload\
                 ) VALUES ($1,$2,$3,$4,'memory_expired',$5,'system','expiry-worker',$6,$7)",
            )
            .bind(&authority.tenant_id)
            .bind(event_id)
            .bind(memory_record_id)
            .bind(event_sequence)
            .bind(memory_version_id)
            .bind(&reason_code)
            .bind(json!({"projection_operation_id": operation_id}))
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE memory_core.context_packs cp \
                 SET invalidated_at=COALESCE(cp.invalidated_at,transaction_timestamp()),\
                     invalidation_reason=COALESCE(cp.invalidation_reason,'MEMORY_EXPIRED') \
                 WHERE cp.tenant_id=$1 AND cp.invalidated_at IS NULL \
                   AND EXISTS (\
                       SELECT 1 FROM memory_core.context_pack_items cpi \
                       WHERE cpi.tenant_id=cp.tenant_id \
                         AND cpi.context_pack_id=cp.id \
                         AND cpi.memory_record_id=$2\
                   )",
            )
            .bind(&authority.tenant_id)
            .bind(memory_record_id)
            .execute(&mut *tx)
            .await?;
            let projection_idempotency_key = format!(
                "mem0:{generation}:expire:{memory_record_id}:\
                 {memory_version_id}:{expired_revision}"
            );
            sqlx::query(
                "INSERT INTO memory_core.projection_operations (\
                     tenant_id,id,memory_record_id,memory_version_id,head_revision,provider,\
                     generation,operation_type,idempotency_key,status,provider_object_id,\
                     content_hash\
                 ) VALUES ($1,$2,$3,$4,$5,'mem0',$6,'delete',$7,'pending',$8,$9)",
            )
            .bind(&authority.tenant_id)
            .bind(operation_id)
            .bind(memory_record_id)
            .bind(memory_version_id)
            .bind(expired_revision)
            .bind(&generation)
            .bind(projection_idempotency_key)
            .bind(provider_object_id)
            .bind(content_hash)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO memory_core.outbox_events (\
                     tenant_id,id,aggregate_type,aggregate_id,aggregate_revision,event_type,\
                     payload,ordering_key,idempotency_key\
                 ) VALUES ($1,$2,'memory_record',$3,$4,'projection_delete',$5,$6,$7)",
            )
            .bind(&authority.tenant_id)
            .bind(outbox_id)
            .bind(memory_record_id)
            .bind(expired_revision)
            .bind(json!({"projection_operation_id": operation_id}))
            .bind(memory_record_id.to_string())
            .bind(format!("outbox:{operation_id}"))
            .execute(&mut *tx)
            .await?;
            operations.push(load_operation_tx(&mut tx, &authority.tenant_id, operation_id).await?);
        }
        tx.commit().await?;
        Ok(operations)
    }

    pub async fn create_erasure_request(
        &self,
        authority: &DemoAuthority,
        command: ErasureCommand,
    ) -> Result<ErasureRequestView, RepositoryError> {
        let request_id = stable_id(&format!("erasure-request:{}", command.idempotency_key));
        let request_hash = erasure_request_hash(&command);
        let requester_hash = principal_hash(authority);
        let target_ref = match command.target_kind {
            ErasureTargetKind::MemoryRecord => {
                let record_id = Uuid::parse_str(command.target_id.trim()).map_err(|_| {
                    RepositoryError::Conflict("memory_record erasure target must be a UUID".into())
                })?;
                record_id.to_string()
            }
            ErasureTargetKind::User => {
                if command.target_id != authority.user_id {
                    return Err(RepositoryError::NotFound);
                }
                requester_hash.clone()
            }
            ErasureTargetKind::Project => {
                if !authority.project_ids.contains(command.target_id.trim()) {
                    return Err(RepositoryError::NotFound);
                }
                content_hash(&format!(
                    "erasure-project\u{0}{}\u{0}{}",
                    authority.tenant_id,
                    command.target_id.trim()
                ))
            }
        };

        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "memory-erasure-idempotency:{}:{}",
                authority.tenant_id, command.idempotency_key
            ))
            .execute(&mut *tx)
            .await?;
        if let Some(existing) = sqlx::query(
            "SELECT request_hash,requested_by_hash \
             FROM memory_core.erasure_requests \
             WHERE tenant_id=$1 AND idempotency_key=$2",
        )
        .bind(&authority.tenant_id)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?
        {
            if existing.try_get::<String, _>("request_hash")? != request_hash
                || existing.try_get::<String, _>("requested_by_hash")? != requester_hash
            {
                return Err(RepositoryError::Conflict(
                    "Idempotency-Key was already used with a different erasure request".into(),
                ));
            }
            let view = load_erasure_request_tx(&mut tx, authority, request_id).await?;
            tx.commit().await?;
            return Ok(view);
        }

        let generation = invalidate_projection_maintenance_tx(
            &mut tx,
            &authority.tenant_id,
            "ERASURE_DURING_REBUILD",
        )
        .await?;
        let mut record_ids = match command.target_kind {
            ErasureTargetKind::MemoryRecord => {
                let record_id = Uuid::parse_str(&target_ref).map_err(|_| {
                    RepositoryError::Invariant("validated erasure target is not a UUID".into())
                })?;
                sqlx::query_scalar::<_, Uuid>(
                    "SELECT r.id FROM memory_core.memory_records r \
                     JOIN memory_core.memory_heads h \
                       ON h.tenant_id=r.tenant_id AND h.memory_record_id=r.id \
                     WHERE r.tenant_id=$1 AND r.id=$2 \
                       AND (\
                            (r.scope_kind='tenant' AND $3) \
                         OR (r.scope_kind='user' AND r.scope_id=$4) \
                         OR (r.scope_kind='project' AND r.scope_id=ANY($5)) \
                         OR (r.scope_kind='agent' AND r.scope_id=ANY($6))\
                       ) \
                       AND h.state<>'erased'",
                )
                .bind(&authority.tenant_id)
                .bind(record_id)
                .bind(authority.tenant_scope)
                .bind(&authority.user_id)
                .bind(authority.project_ids.iter().cloned().collect::<Vec<_>>())
                .bind(authority.agent_ids.iter().cloned().collect::<Vec<_>>())
                .fetch_optional(&mut *tx)
                .await?
                .into_iter()
                .collect::<Vec<_>>()
            }
            ErasureTargetKind::User => {
                sqlx::query_scalar::<_, Uuid>(
                    "SELECT r.id FROM memory_core.memory_records r \
                     JOIN memory_core.memory_heads h \
                       ON h.tenant_id=r.tenant_id AND h.memory_record_id=r.id \
                     WHERE r.tenant_id=$1 AND r.scope_kind='user' AND r.scope_id=$2 \
                       AND h.state<>'erased' ORDER BY r.id",
                )
                .bind(&authority.tenant_id)
                .bind(&authority.user_id)
                .fetch_all(&mut *tx)
                .await?
            }
            ErasureTargetKind::Project => {
                sqlx::query_scalar::<_, Uuid>(
                    "SELECT r.id FROM memory_core.memory_records r \
                 JOIN memory_core.memory_heads h \
                   ON h.tenant_id=r.tenant_id AND h.memory_record_id=r.id \
                 WHERE r.tenant_id=$1 AND r.scope_kind='project' AND r.scope_id=$2 \
                   AND h.state<>'erased' ORDER BY r.id",
                )
                .bind(&authority.tenant_id)
                .bind(command.target_id.trim())
                .fetch_all(&mut *tx)
                .await?
            }
        };
        record_ids.sort_unstable();
        if command.target_kind == ErasureTargetKind::MemoryRecord && record_ids.is_empty() {
            return Err(RepositoryError::NotFound);
        }
        let record_count = i32::try_from(record_ids.len())
            .map_err(|_| RepositoryError::Invariant("erasure target is too large".into()))?;
        let initial_status = if record_ids.is_empty() {
            "complete"
        } else {
            "processing"
        };
        let initial_completed_at = record_ids.is_empty().then_some(Utc::now());
        let handler_status = json!({
            "canonical": "complete",
            "projection": if record_ids.is_empty() { "complete" } else { "pending" },
            "completed_records": 0,
            "failed_records": 0
        });
        sqlx::query(
            "INSERT INTO memory_core.erasure_requests (\
                 tenant_id,id,target_kind,target_id,reason_code,status,deadline_at,\
                 handler_status,completed_at,idempotency_key,request_hash,requested_by_hash,\
                 record_count\
             ) VALUES ($1,$2,$3,$4,'USER_REQUESTED_ERASURE',$5,now()+interval '24 hours',\
                       $6,$7,$8,$9,$10,$11)",
        )
        .bind(&authority.tenant_id)
        .bind(request_id)
        .bind(command.target_kind.as_str())
        .bind(&target_ref)
        .bind(initial_status)
        .bind(&handler_status)
        .bind(initial_completed_at)
        .bind(&command.idempotency_key)
        .bind(&request_hash)
        .bind(&requester_hash)
        .bind(record_count)
        .execute(&mut *tx)
        .await?;

        for record_id in record_ids {
            let head = sqlx::query(
                "SELECT revision,state,current_version_id \
                 FROM memory_core.memory_heads \
                 WHERE tenant_id=$1 AND memory_record_id=$2 FOR UPDATE",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| RepositoryError::Invariant("erasure target lost its Head".into()))?;
            let head_revision: i64 = head.try_get("revision")?;
            let current_version_id: Option<Uuid> = head.try_get("current_version_id")?;
            let erased_revision = head_revision + 1;
            let current_hash = if let Some(current_version_id) = current_version_id {
                sqlx::query_scalar::<_, String>(
                    "SELECT content_hash FROM memory_core.memory_versions \
                     WHERE tenant_id=$1 AND memory_record_id=$2 AND id=$3",
                )
                .bind(&authority.tenant_id)
                .bind(record_id)
                .bind(current_version_id)
                .fetch_optional(&mut *tx)
                .await?
            } else {
                None
            };
            let provider_object_id = sqlx::query_scalar::<_, String>(
                "SELECT provider_object_id FROM memory_core.projection_operations \
                 WHERE tenant_id=$1 AND memory_record_id=$2 AND generation=$3 \
                   AND status='ready' AND provider_object_id IS NOT NULL \
                 ORDER BY completed_at DESC NULLS LAST LIMIT 1",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .bind(&generation)
            .fetch_optional(&mut *tx)
            .await?;
            let open_candidate_ids = sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM memory_core.memory_candidates \
                 WHERE tenant_id=$1 AND target_memory_record_id=$2 \
                   AND state='needs_review' ORDER BY id",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .fetch_all(&mut *tx)
            .await?;
            for candidate_id in open_candidate_ids {
                let erasure_decision_id = stable_id(&format!(
                    "erasure-review-reject:{request_id}:{candidate_id}"
                ));
                sqlx::query(
                    "INSERT INTO memory_core.memory_decisions (\
                         tenant_id,id,candidate_id,outcome,reason_code,actor_type,actor_id,\
                         policy_version\
                     ) VALUES ($1,$2,$3,'reject','ERASURE_CANCELLED_REVIEW','system',$4,\
                               'demo-policy-v1')",
                )
                .bind(&authority.tenant_id)
                .bind(erasure_decision_id)
                .bind(candidate_id)
                .bind(request_id.to_string())
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query(
                "UPDATE memory_core.memory_candidates \
                 SET state=CASE WHEN state='needs_review' THEN 'rejected' ELSE state END,\
                     revision=revision+CASE WHEN state='needs_review' THEN 1 ELSE 0 END,\
                     content=NULL,normalized_key=NULL,\
                     content_erased_at=COALESCE(content_erased_at,now()) \
                 WHERE tenant_id=$1 AND target_memory_record_id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE memory_core.memory_heads \
                 SET current_version_id=NULL,state='erased',revision=$3 \
                 WHERE tenant_id=$1 AND memory_record_id=$2 AND revision=$4",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .bind(erased_revision)
            .bind(head_revision)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE memory_core.memory_records \
                 SET semantic_key=$3,scope_id=$4,identity_retired_at=COALESCE(\
                     identity_retired_at,now()\
                 ) \
                 WHERE tenant_id=$1 AND id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .bind(format!("erased:{record_id}"))
            .bind(format!("erased-user:{}", stable_id(&record_id.to_string())))
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO memory_core.tombstones (\
                     tenant_id,memory_record_id,tombstone_revision,minimum_generation,\
                     reason_code\
                 ) VALUES ($1,$2,$3,$4,'USER_REQUESTED_ERASURE') \
                 ON CONFLICT (tenant_id,memory_record_id) DO UPDATE SET \
                     tombstone_revision=GREATEST(\
                         memory_core.tombstones.tombstone_revision,\
                         EXCLUDED.tombstone_revision\
                     ),reason_code='USER_REQUESTED_ERASURE'",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .bind(erased_revision)
            .bind(&generation)
            .execute(&mut *tx)
            .await?;
            let event_sequence = sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(max(sequence_no),0)+1 FROM memory_core.memory_events \
                 WHERE tenant_id=$1 AND memory_record_id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .fetch_one(&mut *tx)
            .await?;
            let event_id = stable_id(&format!("erasure-event:{request_id}:{record_id}"));
            sqlx::query(
                "INSERT INTO memory_core.memory_events (\
                     tenant_id,id,memory_record_id,sequence_no,event_type,memory_version_id,\
                     actor_type,actor_id,reason_code,payload\
                 ) VALUES ($1,$2,$3,$4,'memory_erased',$5,'system',$6,\
                           'USER_REQUESTED_ERASURE','{}'::jsonb)",
            )
            .bind(&authority.tenant_id)
            .bind(event_id)
            .bind(record_id)
            .bind(event_sequence)
            .bind(current_version_id)
            .bind(request_id.to_string())
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE memory_core.context_packs cp \
                 SET invalidated_at=COALESCE(cp.invalidated_at,now()),\
                     invalidation_reason=COALESCE(cp.invalidation_reason,'MEMORY_ERASED') \
                 WHERE cp.tenant_id=$1 AND EXISTS (\
                     SELECT 1 FROM memory_core.context_pack_items cpi \
                     WHERE cpi.tenant_id=cp.tenant_id AND cpi.context_pack_id=cp.id \
                       AND cpi.memory_record_id=$2\
                 )",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE memory_core.context_pack_items \
                 SET content_snapshot=NULL,source_refs='[]'::jsonb \
                 WHERE tenant_id=$1 AND memory_record_id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE memory_core.memory_versions \
                 SET content=NULL,content_hash='erased:' || id::text,normalized_key=NULL,\
                     effective_from=NULL,effective_to=NULL,consent_ref=NULL,retain_until=NULL,\
                     content_erased_at=COALESCE(content_erased_at,now()) \
                 WHERE tenant_id=$1 AND memory_record_id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .execute(&mut *tx)
            .await?;
            let source_ids = sqlx::query_scalar::<_, Uuid>(
                "SELECT DISTINCT source_event_id FROM (\
                     SELECT mvs.source_event_id \
                     FROM memory_core.memory_version_sources mvs \
                     JOIN memory_core.memory_versions v \
                       ON v.tenant_id=mvs.tenant_id AND v.id=mvs.memory_version_id \
                     WHERE v.tenant_id=$1 AND v.memory_record_id=$2 \
                     UNION \
                     SELECT cs.source_event_id \
                     FROM memory_core.candidate_sources cs \
                     JOIN memory_core.memory_candidates c \
                       ON c.tenant_id=cs.tenant_id AND c.id=cs.candidate_id \
                     WHERE c.tenant_id=$1 AND c.target_memory_record_id=$2\
                 ) erased_sources",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .fetch_all(&mut *tx)
            .await?;
            if !source_ids.is_empty() {
                sqlx::query(
                    "UPDATE memory_core.source_events \
                     SET source_ref='erased:' || id::text,\
                         content_hash='erased:' || id::text,\
                         redacted_excerpt=NULL,classification='internal',consent_ref=NULL,\
                         occurred_at=recorded_at,client_event_id='erased-' || id::text \
                     WHERE tenant_id=$1 AND id=ANY($2)",
                )
                .bind(&authority.tenant_id)
                .bind(&source_ids)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query(
                "UPDATE memory_core.memory_events \
                 SET actor_id='erased-actor',payload='{}'::jsonb \
                 WHERE tenant_id=$1 AND memory_record_id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE memory_core.memory_decisions d \
                 SET actor_id='erased-actor' \
                 WHERE d.tenant_id=$1 AND (EXISTS (\
                         SELECT 1 FROM memory_core.memory_candidates c \
                         WHERE c.tenant_id=d.tenant_id AND c.id=d.candidate_id \
                           AND c.target_memory_record_id=$2\
                     ) OR EXISTS (\
                         SELECT 1 FROM memory_core.memory_versions v \
                         WHERE v.tenant_id=d.tenant_id AND v.decision_id=d.id \
                           AND v.memory_record_id=$2\
                     ))",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM memory_search.retrieval_documents \
                 WHERE tenant_id=$1 AND memory_record_id=$2",
            )
            .bind(&authority.tenant_id)
            .bind(record_id)
            .execute(&mut *tx)
            .await?;

            let operation_id = stable_id(&format!("erasure-projection:{request_id}:{record_id}"));
            let outbox_id = stable_id(&format!("erasure-outbox:{request_id}:{record_id}"));
            let projection_idempotency_key =
                format!("mem0:{generation}:erase:{record_id}:{erased_revision}");
            sqlx::query(
                "INSERT INTO memory_core.projection_operations (\
                     tenant_id,id,memory_record_id,memory_version_id,head_revision,provider,\
                     generation,operation_type,idempotency_key,status,provider_object_id,\
                     content_hash\
                 ) VALUES ($1,$2,$3,$4,$5,'mem0',$6,'delete',$7,'pending',$8,$9)",
            )
            .bind(&authority.tenant_id)
            .bind(operation_id)
            .bind(record_id)
            .bind(current_version_id)
            .bind(erased_revision)
            .bind(&generation)
            .bind(&projection_idempotency_key)
            .bind(provider_object_id)
            .bind(current_hash)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO memory_core.outbox_events (\
                     tenant_id,id,aggregate_type,aggregate_id,aggregate_revision,event_type,\
                     payload,ordering_key,idempotency_key\
                 ) VALUES ($1,$2,'memory_record',$3,$4,'projection_delete',$5,$6,$7)",
            )
            .bind(&authority.tenant_id)
            .bind(outbox_id)
            .bind(record_id)
            .bind(erased_revision)
            .bind(json!({"projection_operation_id": operation_id}))
            .bind(record_id.to_string())
            .bind(format!("outbox:{operation_id}"))
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO memory_core.erasure_request_records (\
                     tenant_id,erasure_request_id,memory_record_id,projection_operation_id,status\
                 ) VALUES ($1,$2,$3,$4,'pending')",
            )
            .bind(&authority.tenant_id)
            .bind(request_id)
            .bind(record_id)
            .bind(operation_id)
            .execute(&mut *tx)
            .await?;
        }

        let view = load_erasure_request_tx(&mut tx, authority, request_id).await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn erasure_request(
        &self,
        authority: &DemoAuthority,
        erasure_request_id: Uuid,
    ) -> Result<ErasureRequestView, RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let view = load_erasure_request_tx(&mut tx, authority, erasure_request_id).await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn current_projection_generation(
        &self,
        authority: &DemoAuthority,
    ) -> Result<String, RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        ensure_default_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        let generation = current_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        tx.commit().await?;
        Ok(generation)
    }

    pub async fn current_projection_generation_view(
        &self,
        authority: &DemoAuthority,
    ) -> Result<ProjectionGenerationView, RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        ensure_default_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        let generation = current_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        let view =
            load_projection_generation_tx(&mut tx, &authority.tenant_id, &generation).await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn projection_generation(
        &self,
        authority: &DemoAuthority,
        generation: &str,
    ) -> Result<ProjectionGenerationView, RepositoryError> {
        validate_generation_name(generation)?;
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        ensure_default_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        refresh_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        let view = load_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn start_projection_rebuild(
        &self,
        authority: &DemoAuthority,
        command: ProjectionRebuildCommand,
    ) -> Result<ProjectionGenerationView, RepositoryError> {
        validate_generation_name(&command.generation)?;
        command
            .model_manifest
            .validate_integrity()
            .map_err(RepositoryError::Conflict)?;
        if !(16..=128).contains(&command.idempotency_key.len()) {
            return Err(RepositoryError::Conflict(
                "rebuild Idempotency-Key must contain 16 to 128 characters".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        lock_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        ensure_default_projection_generation_tx(&mut tx, &authority.tenant_id).await?;

        if let Some(existing) = sqlx::query(
            "SELECT generation,model_manifest_hash FROM memory_core.projection_generations \
             WHERE tenant_id=$1 AND idempotency_key=$2",
        )
        .bind(&authority.tenant_id)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?
        {
            let existing_generation: String = existing.try_get("generation")?;
            let existing_manifest_hash: String = existing.try_get("model_manifest_hash")?;
            if existing_generation != command.generation
                || existing_manifest_hash != command.model_manifest.manifest_hash
            {
                return Err(RepositoryError::Conflict(
                    "Idempotency-Key was already used for a different projection generation or model manifest"
                        .into(),
                ));
            }
            refresh_projection_generation_tx(&mut tx, &authority.tenant_id, &command.generation)
                .await?;
            let view =
                load_projection_generation_tx(&mut tx, &authority.tenant_id, &command.generation)
                    .await?;
            tx.commit().await?;
            return Ok(view);
        }

        let source_generation =
            active_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        if source_generation == command.generation {
            return Err(RepositoryError::Conflict(
                "rebuild generation must differ from the active generation".into(),
            ));
        }
        if projection_maintenance_in_progress_tx(&mut tx, &authority.tenant_id).await? {
            return Err(RepositoryError::Conflict(
                "another projection rebuild or switch is already in progress".into(),
            ));
        }
        if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM memory_core.projection_generations \
             WHERE tenant_id=$1 AND generation=$2)",
        )
        .bind(&authority.tenant_id)
        .bind(&command.generation)
        .fetch_one(&mut *tx)
        .await?
        {
            return Err(RepositoryError::Conflict(
                "projection generation already exists; choose a new generation".into(),
            ));
        }

        let rows = sqlx::query(
            "SELECT h.memory_record_id,h.current_version_id,h.revision,r.scope_kind,r.scope_id,\
                    v.content_hash \
             FROM memory_core.memory_heads h \
             JOIN memory_core.memory_records r \
               ON r.tenant_id=h.tenant_id AND r.id=h.memory_record_id \
             JOIN memory_core.memory_versions v \
               ON v.tenant_id=h.tenant_id AND v.id=h.current_version_id \
             WHERE h.tenant_id=$1 AND h.state='active' AND v.content IS NOT NULL \
               AND (v.effective_from IS NULL OR v.effective_from<=transaction_timestamp()) \
               AND (v.effective_to IS NULL OR v.effective_to>transaction_timestamp()) \
               AND (v.retain_until IS NULL OR v.retain_until>transaction_timestamp()) \
               AND NOT EXISTS (SELECT 1 FROM memory_core.tombstones t \
                   WHERE t.tenant_id=h.tenant_id AND t.memory_record_id=h.memory_record_id) \
             ORDER BY h.memory_record_id FOR SHARE OF h",
        )
        .bind(&authority.tenant_id)
        .fetch_all(&mut *tx)
        .await?;
        let expected_records = i32::try_from(rows.len())
            .map_err(|_| RepositoryError::Invariant("rebuild record count overflowed".into()))?;
        let initial_status = if rows.is_empty() { "ready" } else { "building" };
        let completed_at = rows.is_empty().then_some(Utc::now());
        sqlx::query(
            "INSERT INTO memory_core.projection_generations (\
                 tenant_id,generation,status,source_generation,expected_records,ready_records,\
                 failed_records,idempotency_key,requested_by_hash,completed_at,index_provider,\
                 embedding_provider,embedding_model,embedding_revision,embedding_dimensions,\
                 embedding_normalization,embedding_instruction_version,semantic_embedding,\
                 model_manifest_hash\
             ) VALUES ($1,$2,$3,$4,$5,0,0,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
        )
        .bind(&authority.tenant_id)
        .bind(&command.generation)
        .bind(initial_status)
        .bind(&source_generation)
        .bind(expected_records)
        .bind(&command.idempotency_key)
        .bind(principal_hash(authority))
        .bind(completed_at)
        .bind(&command.model_manifest.index_provider)
        .bind(&command.model_manifest.embedding_provider)
        .bind(&command.model_manifest.embedding_model)
        .bind(&command.model_manifest.embedding_revision)
        .bind(command.model_manifest.embedding_dimensions)
        .bind(&command.model_manifest.embedding_normalization)
        .bind(&command.model_manifest.embedding_instruction_version)
        .bind(command.model_manifest.semantic_embedding)
        .bind(&command.model_manifest.manifest_hash)
        .execute(&mut *tx)
        .await?;

        for row in rows {
            let memory_record_id: Uuid = row.try_get("memory_record_id")?;
            let memory_version_id: Uuid = row.try_get("current_version_id")?;
            let head_revision: i64 = row.try_get("revision")?;
            let content_hash: String = row.try_get("content_hash")?;
            let operation_id = stable_id(&format!(
                "rebuild-projection:{}:{}:{memory_record_id}:{memory_version_id}:{head_revision}",
                authority.tenant_id, command.generation
            ));
            let outbox_id = stable_id(&format!("rebuild-outbox:{operation_id}"));
            let idempotency_key = format!(
                "mem0:{}:rebuild-upsert:{memory_record_id}:{memory_version_id}:{head_revision}",
                command.generation
            );
            sqlx::query(
                "INSERT INTO memory_core.projection_operations (\
                     tenant_id,id,memory_record_id,memory_version_id,head_revision,provider,\
                     generation,operation_type,idempotency_key,status,content_hash\
                 ) VALUES ($1,$2,$3,$4,$5,'mem0',$6,'upsert',$7,'pending',$8)",
            )
            .bind(&authority.tenant_id)
            .bind(operation_id)
            .bind(memory_record_id)
            .bind(memory_version_id)
            .bind(head_revision)
            .bind(&command.generation)
            .bind(idempotency_key)
            .bind(content_hash)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO memory_core.outbox_events (\
                     tenant_id,id,aggregate_type,aggregate_id,aggregate_revision,event_type,\
                     payload,ordering_key,idempotency_key\
                 ) VALUES ($1,$2,'memory_record',$3,$4,'projection_rebuild_upsert',$5,$6,$7)",
            )
            .bind(&authority.tenant_id)
            .bind(outbox_id)
            .bind(memory_record_id)
            .bind(head_revision)
            .bind(json!({"projection_operation_id": operation_id}))
            .bind(memory_record_id.to_string())
            .bind(format!("outbox:{operation_id}"))
            .execute(&mut *tx)
            .await?;
        }

        let view =
            load_projection_generation_tx(&mut tx, &authority.tenant_id, &command.generation)
                .await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn activate_projection_generation(
        &self,
        authority: &DemoAuthority,
        generation: &str,
        expected_active_generation: &str,
        idempotency_key: &str,
    ) -> Result<ProjectionGenerationView, RepositoryError> {
        validate_generation_name(generation)?;
        validate_generation_name(expected_active_generation)?;
        validate_generation_transition_idempotency_key(idempotency_key)?;
        let request_hash = content_hash(&format!(
            "activate\u{0}{generation}\u{0}{expected_active_generation}"
        ));
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        lock_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        ensure_default_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        if let Some(replayed) = replay_projection_generation_transition_tx(
            &mut tx,
            &authority.tenant_id,
            idempotency_key,
            "activate",
            generation,
            &request_hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(replayed);
        }
        refresh_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        let target =
            load_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        if target.status == "canary" {
            record_projection_generation_transition_tx(
                &mut tx,
                &authority.tenant_id,
                idempotency_key,
                "activate",
                generation,
                &request_hash,
                &target,
            )
            .await?;
            tx.commit().await?;
            return Ok(target);
        }
        if target.status != "ready"
            || target.ready_records != target.expected_records
            || target.failed_records != 0
        {
            return Err(RepositoryError::Conflict(
                "projection generation is not fully ready for activation".into(),
            ));
        }
        let active = active_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        if active != expected_active_generation {
            return Err(RepositoryError::Conflict(format!(
                "expected active generation {expected_active_generation:?}, current is {active:?}"
            )));
        }
        validate_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        sqlx::query(
            "UPDATE memory_core.projection_generations \
             SET status='standby',retired_at=NULL \
             WHERE tenant_id=$1 AND generation=$2 AND status='active'",
        )
        .bind(&authority.tenant_id)
        .bind(&active)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE memory_core.projection_generations \
             SET status='canary',activated_at=now() \
             WHERE tenant_id=$1 AND generation=$2 AND status='ready'",
        )
        .bind(&authority.tenant_id)
        .bind(generation)
        .execute(&mut *tx)
        .await?;
        cancel_projection_generation_work_tx(
            &mut tx,
            &authority.tenant_id,
            generation,
            "REBUILD_ABORTED",
        )
        .await?;
        let view = load_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        record_projection_generation_transition_tx(
            &mut tx,
            &authority.tenant_id,
            idempotency_key,
            "activate",
            generation,
            &request_hash,
            &view,
        )
        .await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn finalize_projection_generation(
        &self,
        authority: &DemoAuthority,
        generation: &str,
        idempotency_key: &str,
    ) -> Result<ProjectionGenerationView, RepositoryError> {
        validate_generation_name(generation)?;
        validate_generation_transition_idempotency_key(idempotency_key)?;
        let request_hash = content_hash(&format!("finalize\u{0}{generation}"));
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        lock_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        if let Some(replayed) = replay_projection_generation_transition_tx(
            &mut tx,
            &authority.tenant_id,
            idempotency_key,
            "finalize",
            generation,
            &request_hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(replayed);
        }
        validate_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        let target =
            load_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        if target.status == "active" {
            record_projection_generation_transition_tx(
                &mut tx,
                &authority.tenant_id,
                idempotency_key,
                "finalize",
                generation,
                &request_hash,
                &target,
            )
            .await?;
            tx.commit().await?;
            return Ok(target);
        }
        if target.status != "canary" {
            return Err(RepositoryError::Conflict(
                "only the canary projection generation can be finalized".into(),
            ));
        }
        sqlx::query(
            "UPDATE memory_core.projection_generations \
             SET status='retired',retired_at=now() \
             WHERE tenant_id=$1 AND status='standby'",
        )
        .bind(&authority.tenant_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE memory_core.projection_generations SET status='active' \
             WHERE tenant_id=$1 AND generation=$2 AND status='canary'",
        )
        .bind(&authority.tenant_id)
        .bind(generation)
        .execute(&mut *tx)
        .await?;
        let view = load_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        record_projection_generation_transition_tx(
            &mut tx,
            &authority.tenant_id,
            idempotency_key,
            "finalize",
            generation,
            &request_hash,
            &view,
        )
        .await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn rollback_projection_generation(
        &self,
        authority: &DemoAuthority,
        generation: &str,
        idempotency_key: &str,
    ) -> Result<ProjectionGenerationView, RepositoryError> {
        validate_generation_name(generation)?;
        validate_generation_transition_idempotency_key(idempotency_key)?;
        let request_hash = content_hash(&format!("rollback\u{0}{generation}"));
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        lock_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        if let Some(replayed) = replay_projection_generation_transition_tx(
            &mut tx,
            &authority.tenant_id,
            idempotency_key,
            "rollback",
            generation,
            &request_hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(replayed);
        }
        let target =
            load_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        if target.status != "canary" {
            return Err(RepositoryError::Conflict(
                "only the canary projection generation can be rolled back".into(),
            ));
        }
        let restored = sqlx::query_scalar::<_, String>(
            "UPDATE memory_core.projection_generations \
             SET status='active',activated_at=now(),retired_at=NULL \
             WHERE tenant_id=$1 AND status='standby' RETURNING generation",
        )
        .bind(&authority.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            RepositoryError::Invariant("canary generation has no standby rollback target".into())
        })?;
        sqlx::query(
            "UPDATE memory_core.projection_generations \
             SET status='retired',retired_at=now(),last_error_code='ROLLED_BACK' \
             WHERE tenant_id=$1 AND generation=$2 AND status='canary'",
        )
        .bind(&authority.tenant_id)
        .bind(generation)
        .execute(&mut *tx)
        .await?;
        let view = load_projection_generation_tx(&mut tx, &authority.tenant_id, &restored).await?;
        record_projection_generation_transition_tx(
            &mut tx,
            &authority.tenant_id,
            idempotency_key,
            "rollback",
            generation,
            &request_hash,
            &view,
        )
        .await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn abort_projection_rebuild(
        &self,
        authority: &DemoAuthority,
        generation: &str,
        idempotency_key: &str,
    ) -> Result<ProjectionGenerationView, RepositoryError> {
        validate_generation_name(generation)?;
        validate_generation_transition_idempotency_key(idempotency_key)?;
        let request_hash = content_hash(&format!("abort\u{0}{generation}"));
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        lock_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        if let Some(replayed) = replay_projection_generation_transition_tx(
            &mut tx,
            &authority.tenant_id,
            idempotency_key,
            "abort",
            generation,
            &request_hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(replayed);
        }
        let target =
            load_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        if target.status == "failed" {
            record_projection_generation_transition_tx(
                &mut tx,
                &authority.tenant_id,
                idempotency_key,
                "abort",
                generation,
                &request_hash,
                &target,
            )
            .await?;
            tx.commit().await?;
            return Ok(target);
        }
        if !matches!(target.status.as_str(), "building" | "ready") {
            return Err(RepositoryError::Conflict(
                "only a building or ready generation can be aborted".into(),
            ));
        }
        sqlx::query(
            "UPDATE memory_core.projection_generations \
             SET status='failed',completed_at=now(),last_error_code='REBUILD_ABORTED' \
             WHERE tenant_id=$1 AND generation=$2",
        )
        .bind(&authority.tenant_id)
        .bind(generation)
        .execute(&mut *tx)
        .await?;
        let view = load_projection_generation_tx(&mut tx, &authority.tenant_id, generation).await?;
        record_projection_generation_transition_tx(
            &mut tx,
            &authority.tenant_id,
            idempotency_key,
            "abort",
            generation,
            &request_hash,
            &view,
        )
        .await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn projection_generations_for_cleanup(
        &self,
        authority: &DemoAuthority,
        model_manifest_hash: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        ensure_default_projection_generation_tx(&mut tx, &authority.tenant_id).await?;
        let generations = sqlx::query_scalar::<_, String>(
            "SELECT generation FROM memory_core.projection_generations \
             WHERE tenant_id=$1 AND model_manifest_hash=$2 ORDER BY generation",
        )
        .bind(&authority.tenant_id)
        .bind(model_manifest_hash)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(generations)
    }

    pub async fn claim_projection(
        &self,
        authority: &DemoAuthority,
        worker_id: &str,
    ) -> Result<Option<ProjectionWork>, RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let outbox = sqlx::query(
            "SELECT o.id,o.payload FROM memory_core.outbox_events o \
             WHERE o.tenant_id=$1 AND o.available_at <= now() \
               AND (o.status='pending' OR (o.status='leased' AND o.lease_until < now())) \
               AND NOT EXISTS (\
                   SELECT 1 FROM memory_core.outbox_events earlier \
                   WHERE earlier.tenant_id=o.tenant_id \
                     AND earlier.ordering_key=o.ordering_key \
                     AND earlier.status IN ('pending','leased') \
                     AND (earlier.created_at,earlier.id)<(o.created_at,o.id)\
               ) \
             ORDER BY o.created_at,o.id FOR UPDATE OF o SKIP LOCKED LIMIT 1",
        )
        .bind(&authority.tenant_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(outbox) = outbox else {
            tx.commit().await?;
            return Ok(None);
        };
        let outbox_id: Uuid = outbox.try_get("id")?;
        let payload: serde_json::Value = outbox.try_get("payload")?;
        let operation_id = payload
            .get("projection_operation_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| RepositoryError::Invariant("outbox has no operation ID".into()))
            .and_then(|value| {
                Uuid::parse_str(value).map_err(|_| {
                    RepositoryError::Invariant("outbox operation ID is invalid".into())
                })
            })?;

        let row = sqlx::query(
            "SELECT po.operation_type,po.status,po.memory_record_id,po.memory_version_id,\
                    po.head_revision,po.generation,po.content_hash,po.provider_object_id,\
                    po.attempts,r.scope_kind,r.scope_id,h.revision AS current_head_revision,\
                    h.state AS current_head_state,h.current_version_id,v.content,\
                    pg.model_manifest_hash \
             FROM memory_core.projection_operations po \
             JOIN memory_core.projection_generations pg \
               ON pg.tenant_id=po.tenant_id AND pg.generation=po.generation \
             JOIN memory_core.memory_records r \
               ON r.tenant_id=po.tenant_id AND r.id=po.memory_record_id \
             JOIN memory_core.memory_heads h \
               ON h.tenant_id=po.tenant_id AND h.memory_record_id=po.memory_record_id \
             LEFT JOIN memory_core.memory_versions v \
               ON v.tenant_id=po.tenant_id AND v.id=po.memory_version_id \
             WHERE po.tenant_id=$1 AND po.id=$2 FOR UPDATE OF po",
        )
        .bind(&authority.tenant_id)
        .bind(operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| RepositoryError::Invariant("outbox operation is missing".into()))?;
        let prior_status_raw: String = row.try_get("status")?;
        let prior_status = ProjectionOperationStatus::try_from(prior_status_raw.as_str())
            .map_err(RepositoryError::Invariant)?;
        let operation_type_raw: String = row.try_get("operation_type")?;
        let operation_type = ProjectionOperationType::try_from(operation_type_raw.as_str())
            .map_err(RepositoryError::Invariant)?;
        let scope_kind: String = row.try_get("scope_kind")?;
        let scope_id: Option<String> = row.try_get("scope_id")?;
        let namespace_key = namespace_key(&authority.tenant_id, &scope_kind, scope_id.as_deref());

        sqlx::query(
            "UPDATE memory_core.outbox_events SET status='leased',lease_owner=$3,\
                    lease_until=now()+interval '15 seconds',attempts=attempts+1 \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(outbox_id)
        .bind(worker_id)
        .execute(&mut *tx)
        .await?;
        let lease_version = sqlx::query_scalar::<_, i64>(
            "UPDATE memory_core.projection_operations \
             SET status='leased',lease_owner=$3,lease_until=now()+interval '15 seconds',\
                 attempts=attempts+1,lease_version=lease_version+1,updated_at=now() \
             WHERE tenant_id=$1 AND id=$2 RETURNING lease_version",
        )
        .bind(&authority.tenant_id)
        .bind(operation_id)
        .bind(worker_id)
        .fetch_one(&mut *tx)
        .await?;

        let work = ProjectionWork {
            outbox_id,
            operation_id,
            operation_type,
            prior_status,
            memory_record_id: row.try_get("memory_record_id")?,
            memory_version_id: row.try_get("memory_version_id")?,
            expected_head_revision: row.try_get("head_revision")?,
            current_head_revision: row.try_get("current_head_revision")?,
            current_head_state: row.try_get("current_head_state")?,
            current_version_id: row.try_get("current_version_id")?,
            generation: row.try_get("generation")?,
            model_manifest_hash: row.try_get("model_manifest_hash")?,
            namespace_key,
            content: row.try_get("content")?,
            content_hash: row.try_get("content_hash")?,
            provider_object_id: row.try_get("provider_object_id")?,
            attempts: row.try_get::<i32, _>("attempts")? + 1,
            lease_version,
            lease_owner: worker_id.to_owned(),
        };
        tx.commit().await?;
        Ok(Some(work))
    }

    pub async fn complete_ready(
        &self,
        authority: &DemoAuthority,
        work: &ProjectionWork,
        provider_object_id: &str,
    ) -> Result<(), RepositoryError> {
        let version_id = work
            .memory_version_id
            .ok_or_else(|| RepositoryError::Invariant("upsert has no Version".into()))?;
        let content = work
            .content
            .as_deref()
            .ok_or_else(|| RepositoryError::Invariant("upsert has no content".into()))?;
        let hash = work
            .content_hash
            .as_deref()
            .ok_or_else(|| RepositoryError::Invariant("upsert has no content hash".into()))?;
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        fence_operation(
            &mut tx,
            authority,
            work,
            "ready",
            Some(provider_object_id),
            None,
        )
        .await?;
        sqlx::query(
            "INSERT INTO memory_search.retrieval_documents (\
                 tenant_id,memory_record_id,memory_version_id,namespace_key,generation,\
                 head_revision,content,content_hash\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) \
             ON CONFLICT (tenant_id,memory_version_id,generation) DO UPDATE SET \
                 namespace_key=EXCLUDED.namespace_key,head_revision=EXCLUDED.head_revision,\
                 content=EXCLUDED.content,content_hash=EXCLUDED.content_hash,indexed_at=now()",
        )
        .bind(&authority.tenant_id)
        .bind(work.memory_record_id)
        .bind(version_id)
        .bind(&work.namespace_key)
        .bind(&work.generation)
        .bind(work.expected_head_revision)
        .bind(content)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
        replace_retrieval_terms_tx(
            &mut tx,
            &authority.tenant_id,
            work.memory_record_id,
            version_id,
            &work.generation,
            content,
        )
        .await?;
        sqlx::query(
            "DELETE FROM memory_search.retrieval_documents \
             WHERE tenant_id=$1 AND memory_record_id=$2 AND generation=$3 \
               AND memory_version_id<>$4",
        )
        .bind(&authority.tenant_id)
        .bind(work.memory_record_id)
        .bind(&work.generation)
        .bind(version_id)
        .execute(&mut *tx)
        .await?;
        refresh_projection_generation_tx(&mut tx, &authority.tenant_id, &work.generation).await?;
        complete_outbox(&mut tx, authority, work).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn complete_deleted(
        &self,
        authority: &DemoAuthority,
        work: &ProjectionWork,
    ) -> Result<(), RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        fence_operation(
            &mut tx,
            authority,
            work,
            "deleted",
            work.provider_object_id.as_deref(),
            None,
        )
        .await?;
        sqlx::query(
            "DELETE FROM memory_search.retrieval_documents \
             WHERE tenant_id=$1 AND memory_record_id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(work.memory_record_id)
        .execute(&mut *tx)
        .await?;
        update_erasure_projection_status_tx(
            &mut tx,
            authority,
            work.operation_id,
            "complete",
            None,
        )
        .await?;
        complete_outbox(&mut tx, authority, work).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn complete_stale(
        &self,
        authority: &DemoAuthority,
        work: &ProjectionWork,
        reason: &str,
    ) -> Result<(), RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        fence_operation(&mut tx, authority, work, "stale", None, Some(reason)).await?;
        refresh_projection_generation_tx(&mut tx, &authority.tenant_id, &work.generation).await?;
        complete_outbox(&mut tx, authority, work).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn reschedule(
        &self,
        authority: &DemoAuthority,
        work: &ProjectionWork,
        status: ProjectionOperationStatus,
        error_code: &str,
    ) -> Result<(), RepositoryError> {
        let exponent = work.attempts.clamp(0, 7) as u32;
        let backoff_ms = (250_i64.saturating_mul(1_i64 << exponent)).min(30_000);
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let result = sqlx::query(
            "UPDATE memory_core.projection_operations \
             SET status=$5,last_error_code=$6,lease_owner=NULL,lease_until=NULL,\
                 available_at=now()+($7::bigint * interval '1 millisecond'),updated_at=now() \
             WHERE tenant_id=$1 AND id=$2 AND lease_owner=$3 AND lease_version=$4",
        )
        .bind(&authority.tenant_id)
        .bind(work.operation_id)
        .bind(&work.lease_owner)
        .bind(work.lease_version)
        .bind(status.as_str())
        .bind(error_code)
        .bind(backoff_ms)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::LeaseLost);
        }
        sqlx::query(
            "UPDATE memory_core.outbox_events \
             SET status='pending',available_at=now()+($4::bigint * interval '1 millisecond'),\
                 lease_owner=NULL,lease_until=NULL \
             WHERE tenant_id=$1 AND id=$2 AND lease_owner=$3",
        )
        .bind(&authority.tenant_id)
        .bind(work.outbox_id)
        .bind(&work.lease_owner)
        .bind(backoff_ms)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn complete_dead_letter(
        &self,
        authority: &DemoAuthority,
        work: &ProjectionWork,
        error_code: &str,
    ) -> Result<(), RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        fence_operation(
            &mut tx,
            authority,
            work,
            "dead_letter",
            None,
            Some(error_code),
        )
        .await?;
        let result = sqlx::query(
            "UPDATE memory_core.outbox_events \
             SET status='dead_letter',lease_owner=NULL,lease_until=NULL \
             WHERE tenant_id=$1 AND id=$2 AND lease_owner=$3",
        )
        .bind(&authority.tenant_id)
        .bind(work.outbox_id)
        .bind(&work.lease_owner)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::LeaseLost);
        }
        update_erasure_projection_status_tx(
            &mut tx,
            authority,
            work.operation_id,
            "failed",
            Some(error_code),
        )
        .await?;
        refresh_projection_generation_tx(&mut tx, &authority.tenant_id, &work.generation).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn authorized_hits(
        &self,
        authority: &DemoAuthority,
        hits: &[ProjectionHit],
    ) -> Result<Vec<AuthorizedMemory>, RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let authorized = authorized_hits_tx(&mut tx, authority, "answer_user", hits).await?;
        tx.commit().await?;
        Ok(authorized)
    }

    pub async fn search_local_projection(
        &self,
        authority: &DemoAuthority,
        scope_kind: &str,
        scope_id: Option<&str>,
        query: &str,
        generation: &str,
        limit: usize,
    ) -> Result<Vec<ProjectionHit>, RepositoryError> {
        let namespace_key = authorized_namespace_key(authority, scope_kind, scope_id)
            .ok_or(RepositoryError::NotFound)?;
        if query.trim().is_empty() || limit == 0 || limit > 200 {
            return Err(RepositoryError::Invariant(
                "local projection search requires a query and limit between 1 and 200".into(),
            ));
        }
        let scan_limit = i64::try_from(limit.saturating_mul(20).clamp(limit, 1_000))
            .map_err(|_| RepositoryError::Invariant("local search limit overflowed".into()))?;
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let rows = sqlx::query(
            "SELECT memory_record_id,memory_version_id,head_revision,generation,\
                    content,content_hash \
             FROM memory_search.retrieval_documents \
             WHERE tenant_id=$1 AND namespace_key=$2 AND generation=$3 \
             ORDER BY \
                 CASE WHEN strpos(lower(content),lower($4))>0 THEN 1 ELSE 0 END DESC,\
                 ts_rank_cd(search_vector,plainto_tsquery('simple',$4)) DESC,\
                 indexed_at DESC,memory_version_id \
             LIMIT $5",
        )
        .bind(&authority.tenant_id)
        .bind(&namespace_key)
        .bind(generation)
        .bind(query)
        .bind(scan_limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut hits = rows
            .into_iter()
            .filter_map(|row| {
                let head_revision = row.try_get::<i64, _>("head_revision").ok()?;
                let score =
                    lexical_relevance_score(query, &row.try_get::<String, _>("content").ok()?);
                if head_revision < 0 || score <= 0.0 {
                    return None;
                }
                Some(ProjectionHit {
                    provider_object_id: format!(
                        "local-fts:{}",
                        row.try_get::<Uuid, _>("memory_version_id").ok()?
                    ),
                    memory_record_id: row.try_get("memory_record_id").ok()?,
                    memory_version_id: row.try_get("memory_version_id").ok()?,
                    head_revision: u64::try_from(head_revision).ok()?,
                    generation: row.try_get("generation").ok()?,
                    content_hash: row.try_get("content_hash").ok()?,
                    provider_score: score,
                })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .provider_score
                .total_cmp(&left.provider_score)
                .then_with(|| left.memory_record_id.cmp(&right.memory_record_id))
                .then_with(|| left.memory_version_id.cmp(&right.memory_version_id))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    pub async fn search_bm25_projection(
        &self,
        authority: &DemoAuthority,
        scope_kind: &str,
        scope_id: Option<&str>,
        query: &str,
        generation: &str,
        limit: usize,
    ) -> Result<Vec<ProjectionHit>, RepositoryError> {
        let namespace_key = authorized_namespace_key(authority, scope_kind, scope_id)
            .ok_or(RepositoryError::NotFound)?;
        if query.trim().is_empty() || limit == 0 || limit > 200 {
            return Err(RepositoryError::Invariant(
                "BM25 projection search requires a query and limit between 1 and 200".into(),
            ));
        }
        let mut terms = CHINESE_TOKENIZER.tokenize(query);
        terms.sort();
        terms.dedup();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit)
            .map_err(|_| RepositoryError::Invariant("BM25 search limit overflowed".into()))?;
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let rows = sqlx::query(
            "WITH corpus_docs AS (\
                 SELECT memory_record_id,memory_version_id,head_revision,generation,content_hash \
                 FROM memory_search.retrieval_documents \
                 WHERE tenant_id=$1 AND namespace_key=$2 AND generation=$3\
             ), corpus AS (\
                 SELECT count(*)::float8 AS n FROM corpus_docs\
             ), doc_lengths AS (\
                 SELECT rt.memory_version_id,max(rt.document_length)::float8 AS dl \
                 FROM memory_search.retrieval_terms rt \
                 JOIN corpus_docs cd ON cd.memory_version_id=rt.memory_version_id \
                 WHERE rt.tenant_id=$1 AND rt.generation=$3 \
                 GROUP BY rt.memory_version_id\
             ), average AS (\
                 SELECT COALESCE(avg(dl),1.0)::float8 AS avgdl FROM doc_lengths\
             ), dfs AS (\
                 SELECT rt.term,count(DISTINCT rt.memory_version_id)::float8 AS df \
                 FROM memory_search.retrieval_terms rt \
                 JOIN corpus_docs cd ON cd.memory_version_id=rt.memory_version_id \
                 WHERE rt.tenant_id=$1 AND rt.generation=$3 AND rt.term=ANY($4::text[]) \
                 GROUP BY rt.term\
             ) \
             SELECT cd.memory_record_id,cd.memory_version_id,cd.head_revision,cd.generation,\
                    cd.content_hash,\
                    sum(\
                        ln(1.0+((corpus.n-dfs.df+0.5)/(dfs.df+0.5))) \
                        *((rt.term_frequency::float8*2.2)\
                          /(rt.term_frequency::float8+1.2\
                            *(0.25+0.75*rt.document_length::float8\
                              /NULLIF(average.avgdl,0))))\
                    )::float8 AS bm25_score \
             FROM memory_search.retrieval_terms rt \
             JOIN corpus_docs cd ON cd.memory_version_id=rt.memory_version_id \
             JOIN dfs ON dfs.term=rt.term \
             CROSS JOIN corpus CROSS JOIN average \
             WHERE rt.tenant_id=$1 AND rt.generation=$3 AND rt.term=ANY($4::text[]) \
             GROUP BY cd.memory_record_id,cd.memory_version_id,cd.head_revision,cd.generation,\
                      cd.content_hash,corpus.n,average.avgdl \
             ORDER BY bm25_score DESC,cd.memory_record_id,cd.memory_version_id \
             LIMIT $5",
        )
        .bind(&authority.tenant_id)
        .bind(&namespace_key)
        .bind(generation)
        .bind(&terms)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let maximum_score = rows
            .iter()
            .filter_map(|row| row.try_get::<f64, _>("bm25_score").ok())
            .fold(0.0_f64, f64::max);
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let raw_score = row.try_get::<f64, _>("bm25_score").ok()?;
                let head_revision = row.try_get::<i64, _>("head_revision").ok()?;
                if raw_score <= 0.0 || head_revision < 0 {
                    return None;
                }
                let memory_version_id = row.try_get::<Uuid, _>("memory_version_id").ok()?;
                Some(ProjectionHit {
                    provider_object_id: format!("local-bm25:{memory_version_id}"),
                    memory_record_id: row.try_get("memory_record_id").ok()?,
                    memory_version_id,
                    head_revision: u64::try_from(head_revision).ok()?,
                    generation: row.try_get("generation").ok()?,
                    content_hash: row.try_get("content_hash").ok()?,
                    provider_score: if maximum_score > 0.0 {
                        (raw_score / maximum_score).clamp(0.0, 1.0)
                    } else {
                        0.0
                    },
                })
            })
            .collect())
    }

    pub async fn search_temporal_projection(
        &self,
        authority: &DemoAuthority,
        scope_kind: &str,
        scope_id: Option<&str>,
        window: &TemporalWindow,
        generation: &str,
        limit: usize,
    ) -> Result<Vec<ProjectionHit>, RepositoryError> {
        let namespace_key = authorized_namespace_key(authority, scope_kind, scope_id)
            .ok_or(RepositoryError::NotFound)?;
        if window.start >= window.end || limit == 0 || limit > 200 {
            return Err(RepositoryError::Invariant(
                "temporal projection search requires a valid window and limit between 1 and 200"
                    .into(),
            ));
        }
        let limit = i64::try_from(limit)
            .map_err(|_| RepositoryError::Invariant("temporal search limit overflowed".into()))?;
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let rows = sqlx::query(
            "SELECT rd.memory_record_id,rd.memory_version_id,rd.head_revision,rd.generation,\
                    rd.content_hash,\
                    max(1.0/(1.0+abs(extract(epoch FROM (se.occurred_at-$5::timestamptz)))\
                                  /86400.0))::float8 AS temporal_score \
             FROM memory_search.retrieval_documents rd \
             JOIN memory_core.memory_version_sources mvs \
               ON mvs.tenant_id=rd.tenant_id \
              AND mvs.memory_version_id=rd.memory_version_id \
             JOIN memory_core.source_events se \
               ON se.tenant_id=mvs.tenant_id AND se.id=mvs.source_event_id \
             WHERE rd.tenant_id=$1 AND rd.namespace_key=$2 AND rd.generation=$3 \
               AND se.occurred_at>=$4 AND se.occurred_at<$5 \
             GROUP BY rd.memory_record_id,rd.memory_version_id,rd.head_revision,rd.generation,\
                      rd.content_hash \
             ORDER BY temporal_score DESC,rd.memory_record_id,rd.memory_version_id \
             LIMIT $6",
        )
        .bind(&authority.tenant_id)
        .bind(&namespace_key)
        .bind(generation)
        .bind(window.start)
        .bind(window.end)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let head_revision = row.try_get::<i64, _>("head_revision").ok()?;
                let score = row.try_get::<f64, _>("temporal_score").ok()?;
                if head_revision < 0 || score <= 0.0 {
                    return None;
                }
                let memory_version_id = row.try_get::<Uuid, _>("memory_version_id").ok()?;
                Some(ProjectionHit {
                    provider_object_id: format!("local-temporal:{memory_version_id}"),
                    memory_record_id: row.try_get("memory_record_id").ok()?,
                    memory_version_id,
                    head_revision: u64::try_from(head_revision).ok()?,
                    generation: row.try_get("generation").ok()?,
                    content_hash: row.try_get("content_hash").ok()?,
                    provider_score: score.clamp(0.0, 1.0),
                })
            })
            .collect())
    }

    pub async fn record_retrieval_run(
        &self,
        authority: &DemoAuthority,
        command: &RetrievalRunCommand,
    ) -> Result<RetrievalRunView, RepositoryError> {
        validate_retrieval_run(authority, command)?;
        let retrieval_run_id = Uuid::new_v4();
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        sqlx::query(
            "INSERT INTO memory_core.retrieval_runs (\
                 tenant_id,id,task_id,attempt_id,purpose,query_hash,requested_limit,\
                 candidate_limit,reference_time,timezone,temporal_start,temporal_end,\
                 temporal_precision,temporal_source_expression,projection_generation,\
                 ranking_version,degraded,degradation_reason\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)",
        )
        .bind(&authority.tenant_id)
        .bind(retrieval_run_id)
        .bind(command.task_id)
        .bind(command.attempt_id)
        .bind(&command.purpose)
        .bind(&command.query_hash)
        .bind(command.requested_limit)
        .bind(command.candidate_limit)
        .bind(command.reference_time)
        .bind(&command.timezone)
        .bind(command.temporal_window.as_ref().map(|window| window.start))
        .bind(command.temporal_window.as_ref().map(|window| window.end))
        .bind(
            command
                .temporal_window
                .as_ref()
                .map(|window| window.precision.as_str()),
        )
        .bind(
            command
                .temporal_window
                .as_ref()
                .map(|window| window.source_expression.as_str()),
        )
        .bind(&command.projection_generation)
        .bind(&command.ranking_version)
        .bind(command.degraded)
        .bind(&command.degradation_reason)
        .execute(&mut *tx)
        .await?;

        for candidate in &command.candidates {
            let receipt_id = stable_id(&format!(
                "retrieval-candidate:{retrieval_run_id}:{}:{}",
                candidate.channel, candidate.memory_version_id
            ));
            sqlx::query(
                "INSERT INTO memory_core.retrieval_candidate_receipts (\
                     tenant_id,id,retrieval_run_id,scope_kind,scope_id,channel,memory_record_id,\
                     memory_version_id,channel_rank,raw_score,channel_weight,rrf_contribution,\
                     fused_score,preauthorized,rerank_score,postauthorized,final_rank,included,\
                     exclusion_reason\
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)",
            )
            .bind(&authority.tenant_id)
            .bind(receipt_id)
            .bind(retrieval_run_id)
            .bind(&candidate.scope_kind)
            .bind(&candidate.scope_id)
            .bind(&candidate.channel)
            .bind(candidate.memory_record_id)
            .bind(candidate.memory_version_id)
            .bind(candidate.channel_rank)
            .bind(candidate.raw_score)
            .bind(candidate.channel_weight)
            .bind(candidate.rrf_contribution)
            .bind(candidate.fused_score)
            .bind(candidate.preauthorized)
            .bind(candidate.rerank_score)
            .bind(candidate.postauthorized)
            .bind(candidate.final_rank)
            .bind(candidate.included)
            .bind(&candidate.exclusion_reason)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.retrieval_run(authority, retrieval_run_id).await
    }

    pub async fn retrieval_run(
        &self,
        authority: &DemoAuthority,
        retrieval_run_id: Uuid,
    ) -> Result<RetrievalRunView, RepositoryError> {
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        let row = sqlx::query(
            "SELECT task_id,attempt_id,purpose,query_hash,requested_limit,candidate_limit,\
                    reference_time,timezone,temporal_start,temporal_end,temporal_precision,\
                    temporal_source_expression,projection_generation,ranking_version,degraded,\
                    degradation_reason,created_at \
             FROM memory_core.retrieval_runs WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(retrieval_run_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RepositoryError::NotFound)?;
        let candidate_rows = sqlx::query(
            "SELECT scope_kind,scope_id,channel,memory_record_id,memory_version_id,channel_rank,\
                    raw_score,channel_weight,rrf_contribution,fused_score,preauthorized,\
                    rerank_score,postauthorized,final_rank,included,exclusion_reason \
             FROM memory_core.retrieval_candidate_receipts \
             WHERE tenant_id=$1 AND retrieval_run_id=$2 \
             ORDER BY channel,channel_rank,memory_record_id,memory_version_id",
        )
        .bind(&authority.tenant_id)
        .bind(retrieval_run_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let temporal_start = row.try_get::<Option<DateTime<Utc>>, _>("temporal_start")?;
        let temporal_end = row.try_get::<Option<DateTime<Utc>>, _>("temporal_end")?;
        let temporal_precision = row.try_get::<Option<String>, _>("temporal_precision")?;
        let temporal_source_expression =
            row.try_get::<Option<String>, _>("temporal_source_expression")?;
        let timezone = row.try_get::<String, _>("timezone")?;
        let temporal_window = match (
            temporal_start,
            temporal_end,
            temporal_precision,
            temporal_source_expression,
        ) {
            (Some(start), Some(end), Some(precision), Some(source_expression)) => {
                Some(TemporalWindow {
                    start,
                    end,
                    precision,
                    timezone: timezone.clone(),
                    source_expression,
                })
            }
            (None, None, None, None) => None,
            _ => {
                return Err(RepositoryError::Invariant(
                    "retrieval run contains a partial temporal window".into(),
                ));
            }
        };
        Ok(RetrievalRunView {
            retrieval_run_id,
            task_id: row.try_get("task_id")?,
            attempt_id: row.try_get("attempt_id")?,
            purpose: row.try_get("purpose")?,
            query_hash: row.try_get("query_hash")?,
            requested_limit: row.try_get("requested_limit")?,
            candidate_limit: row.try_get("candidate_limit")?,
            reference_time: row.try_get("reference_time")?,
            timezone,
            temporal_window,
            projection_generation: row.try_get("projection_generation")?,
            ranking_version: row.try_get("ranking_version")?,
            degraded: row.try_get("degraded")?,
            degradation_reason: row.try_get("degradation_reason")?,
            candidates: candidate_rows
                .into_iter()
                .map(|candidate| {
                    Ok(RetrievalCandidateTraceView {
                        scope_kind: candidate.try_get("scope_kind")?,
                        scope_id: candidate.try_get("scope_id")?,
                        channel: candidate.try_get("channel")?,
                        memory_record_id: candidate.try_get("memory_record_id")?,
                        memory_version_id: candidate.try_get("memory_version_id")?,
                        channel_rank: candidate.try_get("channel_rank")?,
                        raw_score: candidate.try_get("raw_score")?,
                        channel_weight: candidate.try_get("channel_weight")?,
                        rrf_contribution: candidate.try_get("rrf_contribution")?,
                        fused_score: candidate.try_get("fused_score")?,
                        preauthorized: candidate.try_get("preauthorized")?,
                        rerank_score: candidate.try_get("rerank_score")?,
                        postauthorized: candidate.try_get("postauthorized")?,
                        final_rank: candidate.try_get("final_rank")?,
                        included: candidate.try_get("included")?,
                        exclusion_reason: candidate.try_get("exclusion_reason")?,
                    })
                })
                .collect::<Result<Vec<_>, sqlx::Error>>()?,
            created_at: row.try_get("created_at")?,
        })
    }

    pub async fn compile_context_pack(
        &self,
        authority: &DemoAuthority,
        command: &ContextPackCommand,
        hits: &[ProjectionHit],
    ) -> Result<ContextPackManifestView, RepositoryError> {
        let memory_types = validated_context_memory_types(&command.memory_types)?;
        let authority_snapshot_id = authority_snapshot_id(authority, &command.purpose);
        let query_hash = context_pack_request_hash(command);
        let pack_id = stable_id(&format!(
            "context-pack:{}:{}:{}:{}:{}:{}",
            authority.tenant_id,
            command.attempt_id,
            command.purpose,
            query_hash,
            DEMO_CONTEXT_COMPILER_VERSION,
            authority_snapshot_id
        ));

        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        if context_pack_exists_tx(&mut tx, &authority.tenant_id, pack_id).await? {
            let pack =
                load_context_pack_manifest_tx(&mut tx, &authority.tenant_id, pack_id).await?;
            tx.commit().await?;
            return Ok(pack);
        }

        let mut authorized = authorized_pinned_versions_tx(
            &mut tx,
            authority,
            &command.purpose,
            &command.pinned_memory_version_ids,
        )
        .await?
        .into_iter()
        .filter(|memory| memory_types.binary_search(&memory.memory_type).is_ok())
        .collect::<Vec<_>>();
        let mut seen_records = authorized
            .iter()
            .map(|memory| memory.memory_record_id)
            .collect::<HashSet<_>>();
        authorized.extend(
            authorized_hits_tx(&mut tx, authority, &command.purpose, hits)
                .await?
                .into_iter()
                .filter(|memory| memory_types.binary_search(&memory.memory_type).is_ok())
                .filter(|memory| {
                    memory.score >= MIN_TRANSPARENT_RELEVANCE_SCORE
                        || is_memory_applicable_without_semantic_threshold(
                            &command.query,
                            &memory.memory_type,
                        )
                })
                .filter(|memory| seen_records.insert(memory.memory_record_id)),
        );
        authorized.truncate(command.limit);
        let evaluated = evaluate_context_budget(authorized, command.token_budget);
        if command.memory_required && !evaluated.iter().any(|candidate| candidate.injected) {
            return Err(RepositoryError::Conflict(
                "required memory was unavailable within the authorized token budget".into(),
            ));
        }

        let inserted = sqlx::query(
            "INSERT INTO memory_core.context_packs (\
                 tenant_id,id,task_id,attempt_id,purpose,authority_snapshot_id,\
                 policy_version,compiler_version,ranking_version,projection_generation,query_hash,\
                 tokenizer_id,token_budget,estimated_tokens,expires_at\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,\
                       now() + interval '5 minutes') \
             ON CONFLICT DO NOTHING \
             RETURNING created_at,expires_at",
        )
        .bind(&authority.tenant_id)
        .bind(pack_id)
        .bind(command.task_id)
        .bind(command.attempt_id)
        .bind(&command.purpose)
        .bind(&authority_snapshot_id)
        .bind(DEMO_CONTEXT_POLICY_VERSION)
        .bind(DEMO_CONTEXT_COMPILER_VERSION)
        .bind(&command.ranking_version)
        .bind(&command.projection_generation)
        .bind(&query_hash)
        .bind(&command.tokenizer_id)
        .bind(command.token_budget)
        .bind(
            evaluated
                .iter()
                .filter(|candidate| candidate.injected)
                .map(|candidate| candidate.estimated_tokens)
                .sum::<i32>(),
        )
        .fetch_optional(&mut *tx)
        .await?;

        if inserted.is_none() {
            let pack =
                load_context_pack_manifest_tx(&mut tx, &authority.tenant_id, pack_id).await?;
            tx.commit().await?;
            return Ok(pack);
        }

        let mut position = 0_i32;
        for candidate in &evaluated {
            let (section_kind, inclusion_reason) =
                context_item_metadata(&candidate.memory.memory_type)?;
            if candidate.injected {
                sqlx::query(
                    "INSERT INTO memory_core.context_pack_items (\
                         tenant_id,context_pack_id,position,memory_record_id,memory_version_id,\
                         section_kind,inclusion_reason,content_snapshot,source_refs\
                     ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                )
                .bind(&authority.tenant_id)
                .bind(pack_id)
                .bind(position)
                .bind(candidate.memory.memory_record_id)
                .bind(candidate.memory.memory_version_id)
                .bind(section_kind)
                .bind(inclusion_reason)
                .bind(&candidate.memory.content)
                .bind(json!(&candidate.memory.source_refs))
                .execute(&mut *tx)
                .await?;
                position += 1;
            }

            let receipt_id = stable_id(&format!(
                "usage-receipt:{pack_id}:{}",
                candidate.memory.memory_version_id
            ));
            sqlx::query(
                "INSERT INTO memory_core.usage_receipts (\
                     tenant_id,id,context_pack_id,memory_version_id,result,reason_code,\
                     retrieval_score,rank\
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(&authority.tenant_id)
            .bind(receipt_id)
            .bind(pack_id)
            .bind(candidate.memory.memory_version_id)
            .bind(if candidate.injected {
                "injected"
            } else {
                "omitted"
            })
            .bind(if candidate.injected {
                inclusion_reason
            } else {
                "TOKEN_BUDGET"
            })
            .bind(candidate.memory.score)
            .bind(candidate.rank)
            .execute(&mut *tx)
            .await?;
        }

        let pack = load_context_pack_manifest_tx(&mut tx, &authority.tenant_id, pack_id).await?;
        tx.commit().await?;
        Ok(pack)
    }

    pub async fn consume_context_pack(
        &self,
        authority: &DemoAuthority,
        context_pack_id: Uuid,
        command: &ConsumeContextPackCommand,
    ) -> Result<ContextPackView, RepositoryError> {
        let request_hash = context_pack_consume_request_hash(context_pack_id, command);
        let expected_authority_snapshot = authority_snapshot_id(authority, &command.purpose);
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "context-pack-consume:{}:{}",
                authority.tenant_id, command.idempotency_key
            ))
            .execute(&mut *tx)
            .await?;
        if let Some(existing_pack_id) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM memory_core.context_packs \
             WHERE tenant_id=$1 AND consume_idempotency_key=$2",
        )
        .bind(&authority.tenant_id)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?
            && existing_pack_id != context_pack_id
        {
            return Err(RepositoryError::Conflict(
                "Idempotency-Key was already used to consume another context pack".into(),
            ));
        }

        let initial =
            load_context_pack_metadata_tx(&mut tx, &authority.tenant_id, context_pack_id, false)
                .await?;
        validate_context_pack_binding(&initial, command, &expected_authority_snapshot)?;

        let memory_record_ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT memory_record_id FROM memory_core.context_pack_items \
             WHERE tenant_id=$1 AND context_pack_id=$2 \
             ORDER BY memory_record_id",
        )
        .bind(&authority.tenant_id)
        .bind(context_pack_id)
        .fetch_all(&mut *tx)
        .await?;
        if !memory_record_ids.is_empty() {
            sqlx::query(
                "SELECT memory_record_id FROM memory_core.memory_heads \
                 WHERE tenant_id=$1 AND memory_record_id=ANY($2) \
                 ORDER BY memory_record_id FOR SHARE",
            )
            .bind(&authority.tenant_id)
            .bind(&memory_record_ids)
            .fetch_all(&mut *tx)
            .await?;
        }

        let locked =
            load_context_pack_metadata_tx(&mut tx, &authority.tenant_id, context_pack_id, true)
                .await?;
        validate_context_pack_binding(&locked, command, &expected_authority_snapshot)?;

        if locked.invalidated_at.is_some() || locked.expires_at <= Utc::now() {
            return Err(RepositoryError::Conflict(
                "context pack expired or was invalidated; start a new Attempt".into(),
            ));
        }

        if locked.consumed_at.is_some()
            && (locked.consume_idempotency_key.as_deref() != Some(&command.idempotency_key)
                || locked.consume_request_hash.as_deref() != Some(&request_hash))
        {
            return Err(RepositoryError::Conflict(
                "context pack was already consumed by a different request".into(),
            ));
        }

        let sections = match load_current_context_sections_tx(
            &mut tx,
            authority,
            &locked.purpose,
            context_pack_id,
        )
        .await
        {
            Ok(sections) => sections,
            Err(error @ RepositoryError::Conflict(_)) => {
                sqlx::query(
                    "UPDATE memory_core.context_packs \
                     SET invalidated_at=COALESCE(invalidated_at,now()),\
                         invalidation_reason=COALESCE(invalidation_reason,'AUTHORITY_CHANGED') \
                     WHERE tenant_id=$1 AND id=$2",
                )
                .bind(&authority.tenant_id)
                .bind(context_pack_id)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        if let Some(consumed_at) = locked.consumed_at {
            let omission_summary =
                load_omission_summary_tx(&mut tx, &authority.tenant_id, context_pack_id).await?;
            let view = locked.into_view(consumed_at, sections, omission_summary);
            tx.commit().await?;
            return Ok(view);
        }
        let consumed_at = sqlx::query_scalar::<_, DateTime<Utc>>(
            "UPDATE memory_core.context_packs \
             SET consumed_at=now(),consume_idempotency_key=$3,consume_request_hash=$4 \
             WHERE tenant_id=$1 AND id=$2 AND consumed_at IS NULL \
             RETURNING consumed_at",
        )
        .bind(&authority.tenant_id)
        .bind(context_pack_id)
        .bind(&command.idempotency_key)
        .bind(&request_hash)
        .fetch_one(&mut *tx)
        .await?;
        let omission_summary =
            load_omission_summary_tx(&mut tx, &authority.tenant_id, context_pack_id).await?;
        let view = locked.into_view(consumed_at, sections, omission_summary);
        tx.commit().await?;
        Ok(view)
    }

    pub async fn record_outcome(
        &self,
        authority: &DemoAuthority,
        context_pack_id: Uuid,
        command: &OutcomeReceiptCommand,
    ) -> Result<OutcomeReceiptView, RepositoryError> {
        validate_outcome_receipt_command(command)?;
        let request_hash = outcome_receipt_request_hash(context_pack_id, command);
        let mut tx = self.pool.begin().await?;
        set_tenant(&mut tx, &authority.tenant_id).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "outcome-receipt:{}:{}",
                authority.tenant_id, command.idempotency_key
            ))
            .execute(&mut *tx)
            .await?;

        if let Some(existing) = sqlx::query(
            "SELECT context_pack_id,request_hash FROM memory_core.outcome_receipts \
             WHERE tenant_id=$1 AND idempotency_key=$2",
        )
        .bind(&authority.tenant_id)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?
        {
            if existing.try_get::<Uuid, _>("context_pack_id")? != context_pack_id
                || existing.try_get::<String, _>("request_hash")? != request_hash
            {
                return Err(RepositoryError::Conflict(
                    "Outcome Idempotency-Key was already used with a different request".into(),
                ));
            }
            let view =
                load_outcome_receipt_tx(&mut tx, &authority.tenant_id, context_pack_id).await?;
            tx.commit().await?;
            return Ok(view);
        }

        let pack =
            load_context_pack_metadata_tx(&mut tx, &authority.tenant_id, context_pack_id, true)
                .await?;
        if pack.task_id != command.task_id
            || pack.attempt_id != command.attempt_id
            || pack.purpose != command.purpose
        {
            return Err(RepositoryError::Conflict(
                "outcome is bound to a different Task, Attempt or purpose".into(),
            ));
        }
        if pack.authority_snapshot_id != authority_snapshot_id(authority, &command.purpose) {
            return Err(RepositoryError::Conflict(
                "outcome authority snapshot is no longer current".into(),
            ));
        }
        if pack.consumed_at.is_none() {
            return Err(RepositoryError::Conflict(
                "an Outcome Receipt requires a consumed Context Pack".into(),
            ));
        }
        if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM memory_core.outcome_receipts \
             WHERE tenant_id=$1 AND context_pack_id=$2)",
        )
        .bind(&authority.tenant_id)
        .bind(context_pack_id)
        .fetch_one(&mut *tx)
        .await?
        {
            return Err(RepositoryError::Conflict(
                "the Context Pack already has an Outcome Receipt".into(),
            ));
        }

        let receipt_id = stable_id(&format!(
            "outcome-receipt:{}:{context_pack_id}",
            authority.tenant_id
        ));
        let model_request_id_hash = command.model_request_id.as_deref().map(content_hash);
        sqlx::query(
            "INSERT INTO memory_core.outcome_receipts (\
                 tenant_id,id,context_pack_id,task_id,attempt_id,purpose,outcome,model_id,\
                 model_request_id_hash,latency_ms,prompt_tokens,completion_tokens,total_tokens,\
                 error_code,idempotency_key,request_hash\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
        )
        .bind(&authority.tenant_id)
        .bind(receipt_id)
        .bind(context_pack_id)
        .bind(command.task_id)
        .bind(command.attempt_id)
        .bind(&command.purpose)
        .bind(&command.outcome)
        .bind(&command.model_id)
        .bind(model_request_id_hash)
        .bind(command.latency_ms)
        .bind(command.prompt_tokens)
        .bind(command.completion_tokens)
        .bind(command.total_tokens)
        .bind(&command.error_code)
        .bind(&command.idempotency_key)
        .bind(&request_hash)
        .execute(&mut *tx)
        .await?;
        let view = load_outcome_receipt_tx(&mut tx, &authority.tenant_id, context_pack_id).await?;
        tx.commit().await?;
        Ok(view)
    }
}

async fn load_memory_record_view_tx(
    tx: &mut Transaction<'_, Postgres>,
    authority: &DemoAuthority,
    memory_record_id: Uuid,
) -> Result<MemoryRecordView, RepositoryError> {
    let record = sqlx::query(
        "SELECT r.scope_kind,r.scope_id,r.memory_type,r.semantic_key,\
                r.identity_retired_at,h.state,h.revision,h.current_version_id \
         FROM memory_core.memory_records r \
         JOIN memory_core.memory_heads h \
           ON h.tenant_id=r.tenant_id AND h.memory_record_id=r.id \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(&authority.tenant_id)
    .bind(memory_record_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RepositoryError::NotFound)?;
    let scope_kind: String = record.try_get("scope_kind")?;
    let scope_id: Option<String> = record.try_get("scope_id")?;
    if !authority.authorizes_scope(&scope_kind, scope_id.as_deref()) {
        return Err(RepositoryError::NotFound);
    }

    let version_rows = sqlx::query(
        "SELECT v.id,v.version_no,v.content,v.content_hash,v.normalized_key,v.confidence,\
                v.importance,v.inference_kind,v.effective_from,v.effective_to,\
                v.classification,v.allowed_purposes,v.consent_ref,v.retention_policy_id,\
                v.retain_until,v.created_at,v.content_erased_at,\
                COALESCE(ARRAY(\
                    SELECT DISTINCT se.source_ref \
                    FROM memory_core.memory_version_sources mvs \
                    JOIN memory_core.source_events se \
                      ON se.tenant_id=mvs.tenant_id AND se.id=mvs.source_event_id \
                    WHERE mvs.tenant_id=v.tenant_id AND mvs.memory_version_id=v.id \
                    ORDER BY se.source_ref\
                ),ARRAY[]::text[]) AS source_refs \
         FROM memory_core.memory_versions v \
         WHERE v.tenant_id=$1 AND v.memory_record_id=$2 \
         ORDER BY v.version_no",
    )
    .bind(&authority.tenant_id)
    .bind(memory_record_id)
    .fetch_all(&mut **tx)
    .await?;
    let versions = version_rows
        .into_iter()
        .map(|row| {
            Ok(MemoryVersionView {
                memory_version_id: row.try_get("id")?,
                version_no: row.try_get("version_no")?,
                content: row.try_get("content")?,
                content_hash: row.try_get("content_hash")?,
                normalized_key: row.try_get("normalized_key")?,
                confidence: row.try_get("confidence")?,
                importance: row.try_get("importance")?,
                inference_kind: row.try_get("inference_kind")?,
                source_refs: row.try_get("source_refs")?,
                effective_from: row.try_get("effective_from")?,
                effective_to: row.try_get("effective_to")?,
                classification: row.try_get("classification")?,
                allowed_purposes: row.try_get("allowed_purposes")?,
                consent_ref: row.try_get("consent_ref")?,
                retention_policy_id: row.try_get("retention_policy_id")?,
                retain_until: row.try_get("retain_until")?,
                created_at: row.try_get("created_at")?,
                content_erased_at: row.try_get("content_erased_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    if versions
        .iter()
        .any(|version| version.source_refs.is_empty())
    {
        return Err(RepositoryError::Invariant(
            "Memory Version history contains an unprovenanced Version".into(),
        ));
    }

    let event_rows = sqlx::query(
        "SELECT sequence_no,event_type,memory_version_id,actor_type,actor_id,\
                reason_code,payload,occurred_at \
         FROM memory_core.memory_events \
         WHERE tenant_id=$1 AND memory_record_id=$2 ORDER BY sequence_no",
    )
    .bind(&authority.tenant_id)
    .bind(memory_record_id)
    .fetch_all(&mut **tx)
    .await?;
    let events = event_rows
        .into_iter()
        .map(|row| {
            Ok(MemoryEventView {
                sequence_no: row.try_get("sequence_no")?,
                event_type: row.try_get("event_type")?,
                memory_version_id: row.try_get("memory_version_id")?,
                actor_type: row.try_get("actor_type")?,
                actor_id: row.try_get("actor_id")?,
                reason_code: row.try_get("reason_code")?,
                payload: row.try_get("payload")?,
                occurred_at: row.try_get("occurred_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(MemoryRecordView {
        memory_record_id,
        scope: MemoryScopeView {
            kind: scope_kind,
            id: scope_id,
        },
        memory_type: record.try_get("memory_type")?,
        semantic_key: record.try_get("semantic_key")?,
        head_state: record.try_get("state")?,
        revision: record.try_get("revision")?,
        current_version_id: record.try_get("current_version_id")?,
        identity_retired_at: record.try_get("identity_retired_at")?,
        versions,
        events,
    })
}

#[derive(Debug)]
struct EvaluatedContextMemory {
    memory: AuthorizedMemory,
    estimated_tokens: i32,
    rank: i32,
    injected: bool,
}

async fn authorized_hits_tx(
    tx: &mut Transaction<'_, Postgres>,
    authority: &DemoAuthority,
    purpose: &str,
    hits: &[ProjectionHit],
) -> Result<Vec<AuthorizedMemory>, RepositoryError> {
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    let version_ids: Vec<Uuid> = hits.iter().map(|hit| hit.memory_version_id).collect();
    let rows = sqlx::query(
        "SELECT r.id AS memory_record_id,v.id AS memory_version_id,v.content,\
                v.content_hash,v.effective_from,r.memory_type,h.revision,\
                COALESCE((\
                    SELECT array_agg(DISTINCT se.source_ref ORDER BY se.source_ref) \
                    FROM memory_core.memory_version_sources mvs \
                    JOIN memory_core.source_events se \
                      ON se.tenant_id=mvs.tenant_id AND se.id=mvs.source_event_id \
                    WHERE mvs.tenant_id=v.tenant_id AND mvs.memory_version_id=v.id \
                      AND se.source_ref IS NOT NULL\
                ),ARRAY[]::text[]) AS source_refs \
         FROM memory_core.memory_versions v \
         JOIN memory_core.memory_records r \
           ON r.tenant_id=v.tenant_id AND r.id=v.memory_record_id \
         JOIN memory_core.memory_heads h \
           ON h.tenant_id=v.tenant_id AND h.memory_record_id=v.memory_record_id \
         WHERE v.tenant_id=$1 AND v.id=ANY($2) AND h.state='active' \
           AND h.current_version_id=v.id \
           AND (\
                (r.scope_kind='tenant' AND $3) \
             OR (r.scope_kind='user' AND r.scope_id=$4) \
             OR (r.scope_kind='project' AND r.scope_id=ANY($5)) \
             OR (r.scope_kind='agent' AND r.scope_id=ANY($6))\
           ) \
           AND $7=ANY(v.allowed_purposes) \
           AND (v.effective_from IS NULL OR v.effective_from<=transaction_timestamp()) \
           AND (v.effective_to IS NULL OR v.effective_to>transaction_timestamp()) \
           AND (v.retain_until IS NULL OR v.retain_until>transaction_timestamp()) \
           AND NOT EXISTS (SELECT 1 FROM memory_core.tombstones t \
               WHERE t.tenant_id=v.tenant_id AND t.memory_record_id=v.memory_record_id) \
         FOR SHARE OF h",
    )
    .bind(&authority.tenant_id)
    .bind(&version_ids)
    .bind(authority.tenant_scope)
    .bind(&authority.user_id)
    .bind(authority.project_ids.iter().cloned().collect::<Vec<_>>())
    .bind(authority.agent_ids.iter().cloned().collect::<Vec<_>>())
    .bind(purpose)
    .fetch_all(&mut **tx)
    .await?;

    let mut canonical = HashMap::new();
    for row in rows {
        let version_id: Uuid = row.try_get("memory_version_id")?;
        canonical.insert(
            version_id,
            (
                row.try_get::<Uuid, _>("memory_record_id")?,
                row.try_get::<Option<String>, _>("content")?,
                row.try_get::<String, _>("content_hash")?,
                row.try_get::<String, _>("memory_type")?,
                row.try_get::<i64, _>("revision")?,
                row.try_get::<Vec<String>, _>("source_refs")?,
                row.try_get("effective_from")?,
            ),
        );
    }

    let mut seen_records = HashSet::new();
    let mut authorized = Vec::new();
    for hit in hits {
        let Some((record_id, content, hash, memory_type, revision, source_refs, effective_from)) =
            canonical.get(&hit.memory_version_id)
        else {
            continue;
        };
        if *record_id != hit.memory_record_id
            || *revision < 0
            || *revision as u64 != hit.head_revision
            || hash != &hit.content_hash
            || source_refs.is_empty()
            || !seen_records.insert(*record_id)
        {
            continue;
        }
        let Some(content) = content.clone() else {
            continue;
        };
        authorized.push(AuthorizedMemory {
            memory_record_id: *record_id,
            memory_version_id: hit.memory_version_id,
            content,
            memory_type: memory_type.clone(),
            score: hit.provider_score.clamp(0.0, 1.0),
            source_refs: source_refs.clone(),
            effective_from: *effective_from,
        });
    }
    Ok(authorized)
}

async fn authorized_pinned_versions_tx(
    tx: &mut Transaction<'_, Postgres>,
    authority: &DemoAuthority,
    purpose: &str,
    version_ids: &[Uuid],
) -> Result<Vec<AuthorizedMemory>, RepositoryError> {
    if version_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT r.id AS memory_record_id,v.id AS memory_version_id,v.content,\
                v.effective_from,r.memory_type,\
                COALESCE((\
                    SELECT array_agg(DISTINCT se.source_ref ORDER BY se.source_ref) \
                    FROM memory_core.memory_version_sources mvs \
                    JOIN memory_core.source_events se \
                      ON se.tenant_id=mvs.tenant_id AND se.id=mvs.source_event_id \
                    WHERE mvs.tenant_id=v.tenant_id AND mvs.memory_version_id=v.id \
                      AND se.source_ref IS NOT NULL\
                ),ARRAY[]::text[]) AS source_refs \
         FROM memory_core.memory_versions v \
         JOIN memory_core.memory_records r \
           ON r.tenant_id=v.tenant_id AND r.id=v.memory_record_id \
         JOIN memory_core.memory_heads h \
           ON h.tenant_id=v.tenant_id AND h.memory_record_id=v.memory_record_id \
         WHERE v.tenant_id=$1 AND v.id=ANY($2) AND h.state='active' \
           AND h.current_version_id=v.id \
           AND (\
                (r.scope_kind='tenant' AND $3) \
             OR (r.scope_kind='user' AND r.scope_id=$4) \
             OR (r.scope_kind='project' AND r.scope_id=ANY($5)) \
             OR (r.scope_kind='agent' AND r.scope_id=ANY($6))\
           ) \
           AND $7=ANY(v.allowed_purposes) \
           AND (v.effective_from IS NULL OR v.effective_from<=transaction_timestamp()) \
           AND (v.effective_to IS NULL OR v.effective_to>transaction_timestamp()) \
           AND (v.retain_until IS NULL OR v.retain_until>transaction_timestamp()) \
           AND NOT EXISTS (SELECT 1 FROM memory_core.tombstones t \
               WHERE t.tenant_id=v.tenant_id AND t.memory_record_id=v.memory_record_id) \
         ORDER BY array_position($2::uuid[],v.id) \
         FOR SHARE OF h",
    )
    .bind(&authority.tenant_id)
    .bind(version_ids)
    .bind(authority.tenant_scope)
    .bind(&authority.user_id)
    .bind(authority.project_ids.iter().cloned().collect::<Vec<_>>())
    .bind(authority.agent_ids.iter().cloned().collect::<Vec<_>>())
    .bind(purpose)
    .fetch_all(&mut **tx)
    .await?;

    let mut seen_records = HashSet::new();
    let mut memories = Vec::new();
    for row in rows {
        let memory_record_id: Uuid = row.try_get("memory_record_id")?;
        let content: Option<String> = row.try_get("content")?;
        let source_refs: Vec<String> = row.try_get("source_refs")?;
        if !seen_records.insert(memory_record_id) || source_refs.is_empty() {
            continue;
        }
        let Some(content) = content else {
            continue;
        };
        memories.push(AuthorizedMemory {
            memory_record_id,
            memory_version_id: row.try_get("memory_version_id")?,
            content,
            memory_type: row.try_get("memory_type")?,
            score: 1.0,
            source_refs,
            effective_from: row.try_get("effective_from")?,
        });
    }
    Ok(memories)
}

fn evaluate_context_budget(
    memories: Vec<AuthorizedMemory>,
    token_budget: i32,
) -> Vec<EvaluatedContextMemory> {
    let mut remaining = token_budget.max(0);
    memories
        .into_iter()
        .enumerate()
        .map(|(index, memory)| {
            let estimated_tokens = estimate_context_item_tokens(&memory.content);
            let injected = estimated_tokens <= remaining;
            if injected {
                remaining -= estimated_tokens;
            }
            EvaluatedContextMemory {
                memory,
                estimated_tokens,
                rank: i32::try_from(index + 1).unwrap_or(i32::MAX),
                injected,
            }
        })
        .collect()
}

fn estimate_context_item_tokens(content: &str) -> i32 {
    i32::try_from(content.chars().count())
        .unwrap_or(i32::MAX)
        .saturating_add(DEMO_CONTEXT_ITEM_OVERHEAD)
}

fn authority_snapshot_id(authority: &DemoAuthority, purpose: &str) -> String {
    let digest = Sha256::digest(
        format!(
            "{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{purpose}\u{0}typed-scope-v2",
            authority.tenant_id,
            authority.user_id,
            authority.tenant_scope,
            authority
                .project_ids
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\u{1f}"),
            authority
                .agent_ids
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\u{1f}")
        )
        .as_bytes(),
    );
    format!("authz-{}", hex::encode(digest))
}

fn context_pack_request_hash(command: &ContextPackCommand) -> String {
    let mut memory_types = command.memory_types.clone();
    memory_types.sort();
    let base = format!(
        "{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        command.task_id,
        command.attempt_id,
        command.purpose,
        command.query,
        command.limit,
        memory_types.join("\u{1f}"),
        command.ranking_version,
        command.projection_generation,
        command.token_budget,
        command.tokenizer_id,
        command.memory_required,
        command
            .pinned_memory_version_ids
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>()
            .join("\u{1f}")
    );
    content_hash(&format!(
        "{base}\u{0}{}",
        command.temporal_window_hash.as_deref().unwrap_or_default()
    ))
}

fn validated_context_memory_types(requested: &[String]) -> Result<Vec<String>, RepositoryError> {
    if requested.is_empty() || requested.len() > SUPPORTED_CONTEXT_MEMORY_TYPES.len() {
        return Err(RepositoryError::Invariant(
            "context pack memory_types must contain one to four supported values".into(),
        ));
    }
    let mut normalized = requested.to_vec();
    normalized.sort();
    if normalized.windows(2).any(|pair| pair[0] == pair[1])
        || normalized
            .iter()
            .any(|memory_type| !SUPPORTED_CONTEXT_MEMORY_TYPES.contains(&memory_type.as_str()))
    {
        return Err(RepositoryError::Invariant(
            "context pack memory_types contained a duplicate or unsupported value".into(),
        ));
    }
    Ok(normalized)
}

fn context_item_metadata(
    memory_type: &str,
) -> Result<(&'static str, &'static str), RepositoryError> {
    match memory_type {
        "preference" => Ok(("user_preference", "AUTHORIZED_USER_PREFERENCE")),
        "profile_fact" | "project_fact" => Ok(("current_fact", "AUTHORIZED_CURRENT_FACT")),
        "episodic_summary" => Ok(("relevant_experience", "AUTHORIZED_RELEVANT_EXPERIENCE")),
        _ => Err(RepositoryError::Invariant(format!(
            "unsupported context memory type {memory_type}"
        ))),
    }
}

fn correction_request_hash(command: &CorrectionCommand) -> String {
    content_hash(&format!(
        "{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        command.memory_record_id,
        command.expected_revision,
        command.new_content,
        command.reason,
        command.source_ref,
        command
            .effective_from
            .map(|value| value.to_rfc3339())
            .unwrap_or_default()
    ))
}

fn ensure_safe_for_storage(finding: &SecurityFinding) -> Result<(), RepositoryError> {
    if finding.contains_prompt_injection() {
        return Err(RepositoryError::Validation(
            "prompt-injection content cannot enter the canonical ledger".into(),
        ));
    }
    if finding.contains_secret() {
        return Err(RepositoryError::Validation(
            "credential or secret-like content cannot enter the canonical ledger".into(),
        ));
    }
    Ok(())
}

fn review_request_hash(command: &ReviewDecisionCommand) -> String {
    content_hash(&format!(
        "{}\u{0}{}\u{0}{}\u{0}{}",
        command.candidate_id,
        command.expected_candidate_revision,
        command.outcome.as_str(),
        command.reason
    ))
}

fn erasure_request_hash(command: &ErasureCommand) -> String {
    content_hash(&format!(
        "{}\u{0}{}\u{0}{}",
        command.target_kind.as_str(),
        command.target_id,
        command.reason
    ))
}

fn principal_hash(authority: &DemoAuthority) -> String {
    content_hash(&format!(
        "principal\u{0}{}\u{0}{}",
        authority.tenant_id, authority.user_id
    ))
}

fn context_pack_consume_request_hash(
    context_pack_id: Uuid,
    command: &ConsumeContextPackCommand,
) -> String {
    content_hash(&format!(
        "{context_pack_id}\u{0}{}\u{0}{}\u{0}{}",
        command.task_id, command.attempt_id, command.purpose
    ))
}

fn validate_outcome_receipt_command(
    command: &OutcomeReceiptCommand,
) -> Result<(), RepositoryError> {
    let valid_idempotency = (16..=128).contains(&command.idempotency_key.len())
        && command.idempotency_key.trim() == command.idempotency_key
        && !command
            .idempotency_key
            .bytes()
            .any(|byte| byte.is_ascii_control());
    let valid_model_id = (1..=256).contains(&command.model_id.len())
        && command.model_id.trim() == command.model_id
        && !command.model_id.bytes().any(|byte| byte.is_ascii_control());
    let valid_model_request_id = command.model_request_id.as_deref().is_none_or(|value| {
        (1..=512).contains(&value.len())
            && value.trim() == value
            && !value.bytes().any(|byte| byte.is_ascii_control())
    });
    let valid_error_code = command.error_code.as_deref().is_none_or(|value| {
        (1..=128).contains(&value.len())
            && value.trim() == value
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    });
    let valid_tokens = [
        command.prompt_tokens,
        command.completion_tokens,
        command.total_tokens,
    ]
    .into_iter()
    .flatten()
    .all(|value| (0..=1_000_000_000).contains(&value));
    let valid_total = match (
        command.prompt_tokens,
        command.completion_tokens,
        command.total_tokens,
    ) {
        (Some(prompt), Some(completion), Some(total)) => total >= prompt.saturating_add(completion),
        _ => true,
    };
    let valid_outcome = match command.outcome.as_str() {
        "succeeded" => command.error_code.is_none(),
        "failed" => command.error_code.is_some(),
        "cancelled" => true,
        _ => false,
    };
    if !valid_idempotency
        || !(1..=64).contains(&command.purpose.len())
        || command.purpose.trim() != command.purpose
        || !valid_model_id
        || !valid_model_request_id
        || !(0..=86_400_000).contains(&command.latency_ms)
        || !valid_tokens
        || !valid_total
        || !valid_error_code
        || !valid_outcome
    {
        return Err(RepositoryError::Conflict(
            "Outcome Receipt fields violate the runtime contract".into(),
        ));
    }
    Ok(())
}

fn outcome_receipt_request_hash(context_pack_id: Uuid, command: &OutcomeReceiptCommand) -> String {
    content_hash(
        &[
            context_pack_id.to_string(),
            command.task_id.to_string(),
            command.attempt_id.to_string(),
            command.purpose.clone(),
            command.outcome.clone(),
            command.model_id.clone(),
            command
                .model_request_id
                .as_deref()
                .unwrap_or_default()
                .to_owned(),
            command.latency_ms.to_string(),
            command.prompt_tokens.unwrap_or(-1).to_string(),
            command.completion_tokens.unwrap_or(-1).to_string(),
            command.total_tokens.unwrap_or(-1).to_string(),
            command.error_code.as_deref().unwrap_or_default().to_owned(),
        ]
        .join("\u{0}"),
    )
}

async fn context_pack_exists_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    context_pack_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM memory_core.context_packs \
         WHERE tenant_id=$1 AND id=$2)",
    )
    .bind(tenant_id)
    .bind(context_pack_id)
    .fetch_one(&mut **tx)
    .await
}

async fn load_outcome_receipt_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    context_pack_id: Uuid,
) -> Result<OutcomeReceiptView, RepositoryError> {
    let row = sqlx::query(
        "SELECT id,context_pack_id,task_id,attempt_id,purpose,outcome,model_id,\
                model_request_id_hash,latency_ms,prompt_tokens,completion_tokens,total_tokens,\
                error_code,recorded_at \
         FROM memory_core.outcome_receipts \
         WHERE tenant_id=$1 AND context_pack_id=$2",
    )
    .bind(tenant_id)
    .bind(context_pack_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RepositoryError::NotFound)?;
    Ok(OutcomeReceiptView {
        outcome_receipt_id: row.try_get("id")?,
        context_pack_id: row.try_get("context_pack_id")?,
        task_id: row.try_get("task_id")?,
        attempt_id: row.try_get("attempt_id")?,
        purpose: row.try_get("purpose")?,
        outcome: row.try_get("outcome")?,
        model_id: row.try_get("model_id")?,
        model_request_id_hash: row.try_get("model_request_id_hash")?,
        latency_ms: row.try_get("latency_ms")?,
        prompt_tokens: row.try_get("prompt_tokens")?,
        completion_tokens: row.try_get("completion_tokens")?,
        total_tokens: row.try_get("total_tokens")?,
        error_code: row.try_get("error_code")?,
        recorded_at: row.try_get("recorded_at")?,
    })
}

#[derive(Debug)]
struct ContextPackMetadata {
    context_pack_id: Uuid,
    task_id: Uuid,
    attempt_id: Uuid,
    purpose: String,
    authority_snapshot_id: String,
    policy_version: String,
    compiler_version: String,
    ranking_version: String,
    projection_generation: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    invalidated_at: Option<DateTime<Utc>>,
    tokenizer_id: String,
    token_budget: i32,
    estimated_tokens: i32,
    consumed_at: Option<DateTime<Utc>>,
    consume_idempotency_key: Option<String>,
    consume_request_hash: Option<String>,
}

impl ContextPackMetadata {
    fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            context_pack_id: row.try_get("id")?,
            task_id: row.try_get("task_id")?,
            attempt_id: row.try_get("attempt_id")?,
            purpose: row.try_get("purpose")?,
            authority_snapshot_id: row.try_get("authority_snapshot_id")?,
            policy_version: row.try_get("policy_version")?,
            compiler_version: row.try_get("compiler_version")?,
            ranking_version: row.try_get("ranking_version")?,
            projection_generation: row.try_get("projection_generation")?,
            created_at: row.try_get("created_at")?,
            expires_at: row.try_get("expires_at")?,
            invalidated_at: row.try_get("invalidated_at")?,
            tokenizer_id: row.try_get("tokenizer_id")?,
            token_budget: row.try_get("token_budget")?,
            estimated_tokens: row.try_get("estimated_tokens")?,
            consumed_at: row.try_get("consumed_at")?,
            consume_idempotency_key: row.try_get("consume_idempotency_key")?,
            consume_request_hash: row.try_get("consume_request_hash")?,
        })
    }

    fn into_view(
        self,
        consumed_at: DateTime<Utc>,
        sections: Vec<ContextSection>,
        omission_summary: BTreeMap<String, i32>,
    ) -> ContextPackView {
        ContextPackView {
            context_pack_id: self.context_pack_id,
            task_id: self.task_id,
            attempt_id: self.attempt_id,
            purpose: self.purpose,
            authority_snapshot_id: self.authority_snapshot_id,
            policy_version: self.policy_version,
            compiler_version: self.compiler_version,
            ranking_version: self.ranking_version,
            projection_generation: self.projection_generation,
            created_at: self.created_at,
            expires_at: self.expires_at,
            consumed_at,
            tokenizer_id: self.tokenizer_id,
            token_budget: self.token_budget,
            estimated_tokens: self.estimated_tokens,
            sections,
            omission_summary,
        }
    }
}

async fn load_context_pack_metadata_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    context_pack_id: Uuid,
    for_update: bool,
) -> Result<ContextPackMetadata, RepositoryError> {
    let row = if for_update {
        sqlx::query(
            "SELECT id,task_id,attempt_id,purpose,authority_snapshot_id,policy_version,\
                    compiler_version,ranking_version,projection_generation,created_at,expires_at,\
                    invalidated_at,tokenizer_id,token_budget,estimated_tokens,consumed_at,\
                    consume_idempotency_key,consume_request_hash \
             FROM memory_core.context_packs \
             WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
        )
        .bind(tenant_id)
        .bind(context_pack_id)
        .fetch_optional(&mut **tx)
        .await?
    } else {
        sqlx::query(
            "SELECT id,task_id,attempt_id,purpose,authority_snapshot_id,policy_version,\
                    compiler_version,ranking_version,projection_generation,created_at,expires_at,\
                    invalidated_at,tokenizer_id,token_budget,estimated_tokens,consumed_at,\
                    consume_idempotency_key,consume_request_hash \
             FROM memory_core.context_packs \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant_id)
        .bind(context_pack_id)
        .fetch_optional(&mut **tx)
        .await?
    }
    .ok_or(RepositoryError::NotFound)?;
    Ok(ContextPackMetadata::from_row(&row)?)
}

fn validate_context_pack_binding(
    metadata: &ContextPackMetadata,
    command: &ConsumeContextPackCommand,
    expected_authority_snapshot: &str,
) -> Result<(), RepositoryError> {
    if !(16..=128).contains(&command.idempotency_key.len()) {
        return Err(RepositoryError::Conflict(
            "consume Idempotency-Key must contain 16 to 128 characters".into(),
        ));
    }
    if metadata.task_id != command.task_id
        || metadata.attempt_id != command.attempt_id
        || metadata.purpose != command.purpose
    {
        return Err(RepositoryError::Conflict(
            "context pack is bound to a different Task, Attempt or purpose".into(),
        ));
    }
    if metadata.authority_snapshot_id != expected_authority_snapshot {
        return Err(RepositoryError::Conflict(
            "context pack authority snapshot is no longer current".into(),
        ));
    }
    Ok(())
}

async fn load_context_pack_manifest_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    context_pack_id: Uuid,
) -> Result<ContextPackManifestView, RepositoryError> {
    let metadata = load_context_pack_metadata_tx(tx, tenant_id, context_pack_id, false).await?;
    if metadata.invalidated_at.is_some() || metadata.expires_at <= Utc::now() {
        return Err(RepositoryError::Conflict(
            "context pack expired or was invalidated; start a new Attempt".into(),
        ));
    }
    let item_count = context_pack_item_count_tx(tx, tenant_id, context_pack_id).await?;
    let omission_summary = load_omission_summary_tx(tx, tenant_id, context_pack_id).await?;
    Ok(ContextPackManifestView {
        context_pack_id: metadata.context_pack_id,
        task_id: metadata.task_id,
        attempt_id: metadata.attempt_id,
        purpose: metadata.purpose,
        authority_snapshot_id: metadata.authority_snapshot_id,
        policy_version: metadata.policy_version,
        compiler_version: metadata.compiler_version,
        ranking_version: metadata.ranking_version,
        projection_generation: metadata.projection_generation,
        created_at: metadata.created_at,
        expires_at: metadata.expires_at,
        tokenizer_id: metadata.tokenizer_id,
        token_budget: metadata.token_budget,
        estimated_tokens: metadata.estimated_tokens,
        item_count,
        delivery_status: if metadata.consumed_at.is_some() {
            "consumed".into()
        } else {
            "compiled".into()
        },
        consumed_at: metadata.consumed_at,
        omission_summary,
    })
}

async fn context_pack_item_count_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    context_pack_id: Uuid,
) -> Result<i32, RepositoryError> {
    let item_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM memory_core.context_pack_items \
         WHERE tenant_id=$1 AND context_pack_id=$2",
    )
    .bind(tenant_id)
    .bind(context_pack_id)
    .fetch_one(&mut **tx)
    .await?;
    i32::try_from(item_count)
        .map_err(|_| RepositoryError::Invariant("context pack item count overflowed".into()))
}

async fn load_current_context_sections_tx(
    tx: &mut Transaction<'_, Postgres>,
    authority: &DemoAuthority,
    purpose: &str,
    context_pack_id: Uuid,
) -> Result<Vec<ContextSection>, RepositoryError> {
    let item_count = context_pack_item_count_tx(tx, &authority.tenant_id, context_pack_id).await?;
    let item_rows = sqlx::query(
        "SELECT cpi.position,cpi.memory_record_id,cpi.memory_version_id,cpi.section_kind,\
                cpi.inclusion_reason,cpi.content_snapshot,cpi.source_refs,\
                v.content AS canonical_content,v.effective_from,\
                COALESCE((\
                    SELECT array_agg(DISTINCT se.source_ref ORDER BY se.source_ref) \
                    FROM memory_core.memory_version_sources mvs \
                    JOIN memory_core.source_events se \
                      ON se.tenant_id=mvs.tenant_id AND se.id=mvs.source_event_id \
                    WHERE mvs.tenant_id=v.tenant_id AND mvs.memory_version_id=v.id \
                      AND se.source_ref IS NOT NULL\
                ),ARRAY[]::text[]) AS canonical_source_refs \
         FROM memory_core.context_pack_items cpi \
         JOIN memory_core.memory_versions v \
           ON v.tenant_id=cpi.tenant_id AND v.id=cpi.memory_version_id \
         JOIN memory_core.memory_records r \
           ON r.tenant_id=v.tenant_id AND r.id=v.memory_record_id \
         JOIN memory_core.memory_heads h \
           ON h.tenant_id=cpi.tenant_id AND h.memory_record_id=cpi.memory_record_id \
         WHERE cpi.tenant_id=$1 AND cpi.context_pack_id=$2 \
           AND h.state='active' AND h.current_version_id=cpi.memory_version_id \
           AND (\
                (r.scope_kind='tenant' AND $3) \
             OR (r.scope_kind='user' AND r.scope_id=$4) \
             OR (r.scope_kind='project' AND r.scope_id=ANY($5)) \
             OR (r.scope_kind='agent' AND r.scope_id=ANY($6))\
           ) \
           AND $7=ANY(v.allowed_purposes) \
           AND (v.effective_from IS NULL OR v.effective_from<=transaction_timestamp()) \
           AND (v.effective_to IS NULL OR v.effective_to>transaction_timestamp()) \
           AND (v.retain_until IS NULL OR v.retain_until>transaction_timestamp()) \
           AND NOT EXISTS (SELECT 1 FROM memory_core.tombstones t \
               WHERE t.tenant_id=cpi.tenant_id \
                 AND t.memory_record_id=cpi.memory_record_id) \
         ORDER BY cpi.position",
    )
    .bind(&authority.tenant_id)
    .bind(context_pack_id)
    .bind(authority.tenant_scope)
    .bind(&authority.user_id)
    .bind(authority.project_ids.iter().cloned().collect::<Vec<_>>())
    .bind(authority.agent_ids.iter().cloned().collect::<Vec<_>>())
    .bind(purpose)
    .fetch_all(&mut **tx)
    .await?;
    if i32::try_from(item_rows.len()).unwrap_or(i32::MAX) != item_count {
        return Err(RepositoryError::Conflict(
            "context pack references memory that is no longer current or authorized".into(),
        ));
    }

    let mut sections = Vec::new();
    for item_row in item_rows {
        let content_snapshot = item_row
            .try_get::<Option<String>, _>("content_snapshot")?
            .ok_or_else(|| {
                RepositoryError::Invariant("context pack item had no content snapshot".into())
            })?;
        let canonical_content = item_row
            .try_get::<Option<String>, _>("canonical_content")?
            .ok_or_else(|| {
                RepositoryError::Conflict("context pack memory content was erased".into())
            })?;
        let stored_source_refs = parse_source_refs(&item_row, "source_refs")?;
        let canonical_source_refs: Vec<String> = item_row.try_get("canonical_source_refs")?;
        if content_snapshot != canonical_content
            || canonical_source_refs.is_empty()
            || stored_source_refs != canonical_source_refs
        {
            return Err(RepositoryError::Conflict(
                "context pack content or provenance changed before consumption".into(),
            ));
        }
        push_context_item(
            &mut sections,
            item_row.try_get("section_kind")?,
            ContextPackItem {
                memory_record_id: item_row.try_get("memory_record_id")?,
                memory_version_id: item_row.try_get("memory_version_id")?,
                text: canonical_content,
                source_refs: canonical_source_refs,
                effective_from: item_row.try_get("effective_from")?,
                inclusion_reason: item_row.try_get("inclusion_reason")?,
                trust: "untrusted_memory_data",
            },
        );
    }
    Ok(sections)
}

fn parse_source_refs(row: &PgRow, column: &str) -> Result<Vec<String>, RepositoryError> {
    let source_refs_value: serde_json::Value = row.try_get(column)?;
    serde_json::from_value(source_refs_value).map_err(|error| {
        RepositoryError::Invariant(format!(
            "context pack source_refs were not a string array: {error}"
        ))
    })
}

async fn load_extraction_snapshot_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
    lock: bool,
) -> Result<(String, String, Option<Vec<ExtractCandidateProposal>>), RepositoryError> {
    let statement = if lock {
        "SELECT request_hash,extractor_version,status,proposals
         FROM memory_core.extraction_operations
         WHERE tenant_id=$1 AND idempotency_key=$2
         FOR UPDATE"
    } else {
        "SELECT request_hash,extractor_version,status,proposals
         FROM memory_core.extraction_operations
         WHERE tenant_id=$1 AND idempotency_key=$2"
    };
    let row = sqlx::query(statement)
        .bind(tenant_id)
        .bind(idempotency_key)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RepositoryError::NotFound)?;
    let status: String = row.try_get("status")?;
    let raw: Option<serde_json::Value> = row.try_get("proposals")?;
    let proposals: Option<Vec<ExtractCandidateProposal>> = match (status.as_str(), raw) {
        ("pending", None) => None,
        ("completed", Some(raw)) => Some(serde_json::from_value(raw).map_err(|error| {
            RepositoryError::Invariant(format!(
                "stored extraction proposals violated their schema: {error}"
            ))
        })?),
        _ => {
            return Err(RepositoryError::Invariant(
                "extraction operation status and proposal snapshot diverged".into(),
            ));
        }
    };
    if let Some(proposals) = &proposals {
        validate_extraction_proposals(proposals)?;
    }
    Ok((
        row.try_get("request_hash")?,
        row.try_get("extractor_version")?,
        proposals,
    ))
}

fn push_context_item(sections: &mut Vec<ContextSection>, kind: String, item: ContextPackItem) {
    if let Some(section) = sections.iter_mut().find(|section| section.kind == kind) {
        section.items.push(item);
    } else {
        sections.push(ContextSection {
            kind,
            items: vec![item],
        });
    }
}

async fn load_omission_summary_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    context_pack_id: Uuid,
) -> Result<BTreeMap<String, i32>, RepositoryError> {
    let omission_rows = sqlx::query(
        "SELECT reason_code,count(*)::int AS omitted_count \
         FROM memory_core.usage_receipts \
         WHERE tenant_id=$1 AND context_pack_id=$2 AND result='omitted' \
         GROUP BY reason_code ORDER BY reason_code",
    )
    .bind(tenant_id)
    .bind(context_pack_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut omission_summary = BTreeMap::new();
    for omission_row in omission_rows {
        omission_summary.insert(
            omission_row.try_get("reason_code")?,
            omission_row.try_get("omitted_count")?,
        );
    }
    Ok(omission_summary)
}

pub fn demo_namespace_key(authority: &DemoAuthority) -> String {
    namespace_key(&authority.tenant_id, "user", Some(&authority.user_id))
}

pub fn authorized_namespace_key(
    authority: &DemoAuthority,
    scope_kind: &str,
    scope_id: Option<&str>,
) -> Option<String> {
    authority
        .authorizes_scope(scope_kind, scope_id)
        .then(|| namespace_key(&authority.tenant_id, scope_kind, scope_id))
}

fn namespace_key(tenant_id: &str, scope_kind: &str, scope_id: Option<&str>) -> String {
    let raw = format!("{tenant_id}:{scope_kind}:{}", scope_id.unwrap_or("_"));
    format!("ns-{}", hex::encode(Sha256::digest(raw.as_bytes())))
}

fn stable_id(value: &str) -> Uuid {
    Uuid::new_v5(&DEMO_ID_NAMESPACE, value.as_bytes())
}

fn content_hash(content: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(content.as_bytes())))
}

fn validate_retrieval_run(
    authority: &DemoAuthority,
    command: &RetrievalRunCommand,
) -> Result<(), RepositoryError> {
    let query_digest = command.query_hash.strip_prefix("sha256:");
    if command.purpose != "answer_user"
        || query_digest.is_none_or(|digest| {
            digest.len() != 64
                || digest
                    .bytes()
                    .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
        })
        || !(1..=50).contains(&command.requested_limit)
        || !(command.requested_limit..=200).contains(&command.candidate_limit)
        || command.timezone.trim().is_empty()
        || command.timezone.len() > 64
        || command.projection_generation.trim().is_empty()
        || command.projection_generation.len() > 128
        || command.ranking_version.trim().is_empty()
        || command.ranking_version.len() > 128
        || command
            .degradation_reason
            .as_ref()
            .is_some_and(|reason| reason.is_empty() || reason.len() > 512)
        || command.candidates.len() > 1_600
        || command.temporal_window.as_ref().is_some_and(|window| {
            window.start >= window.end
                || window.timezone != command.timezone
                || window.precision.is_empty()
                || window.source_expression.is_empty()
        })
    {
        return Err(RepositoryError::Validation(
            "retrieval run fields violate the explain receipt contract".into(),
        ));
    }

    let allowed_channels = ["provider", "bm25", "lexical", "temporal"];
    let allowed_exclusions = [
        "RRF_LIMIT",
        "PRE_RERANK_POLICY_DENIED",
        "RERANKER_OMITTED",
        "POST_RERANK_AUTHORIZATION_DENIED",
        "BELOW_RELEVANCE_THRESHOLD",
        "FINAL_LIMIT",
    ];
    let mut identities = HashSet::new();
    for candidate in &command.candidates {
        let scope_is_valid = match (candidate.scope_kind.as_str(), candidate.scope_id.as_deref()) {
            ("tenant", None) => true,
            ("user" | "project" | "agent", Some(id)) => !id.trim().is_empty() && id.len() <= 256,
            _ => false,
        };
        let exclusion_is_valid = if candidate.included {
            candidate.exclusion_reason.is_none()
        } else {
            candidate
                .exclusion_reason
                .as_ref()
                .is_some_and(|reason| allowed_exclusions.contains(&reason.as_str()))
        };
        if !scope_is_valid
            || !authority.authorizes_scope(&candidate.scope_kind, candidate.scope_id.as_deref())
            || !allowed_channels.contains(&candidate.channel.as_str())
            || !identities.insert((candidate.channel.as_str(), candidate.memory_version_id))
            || !(1..=200).contains(&candidate.channel_rank)
            || !candidate.raw_score.is_finite()
            || !(0.0..=1.0).contains(&candidate.raw_score)
            || !candidate.channel_weight.is_finite()
            || !(0.0..=10.0).contains(&candidate.channel_weight)
            || candidate.channel_weight == 0.0
            || !candidate.rrf_contribution.is_finite()
            || candidate.rrf_contribution <= 0.0
            || candidate
                .fused_score
                .is_some_and(|score| !score.is_finite() || !(0.0..=1.0).contains(&score))
            || candidate
                .rerank_score
                .is_some_and(|score| !score.is_finite() || !(0.0..=1.0).contains(&score))
            || (candidate.rerank_score.is_some() && !candidate.preauthorized)
            || (candidate.postauthorized && !candidate.preauthorized)
            || candidate.included != candidate.final_rank.is_some()
            || candidate
                .final_rank
                .is_some_and(|rank| !(1..=command.requested_limit).contains(&rank))
            || (candidate.included && !candidate.postauthorized)
            || !exclusion_is_valid
        {
            return Err(RepositoryError::Validation(
                "retrieval candidate fields violate the explain receipt contract".into(),
            ));
        }
    }
    Ok(())
}

fn validate_ingest_candidate(command: &IngestCandidateCommand) -> Result<(), RepositoryError> {
    let bounded_text = [
        (
            "idempotency_key",
            command.idempotency_key.as_str(),
            128usize,
        ),
        ("event_id", command.event_id.as_str(), 256usize),
        ("source_type", command.source_type.as_str(), 64usize),
        ("source_ref", command.source_ref.as_str(), 512usize),
        (
            "source_content",
            command.source_content.as_str(),
            32_768usize,
        ),
        (
            "memory_content",
            command.memory_content.as_str(),
            8_192usize,
        ),
        ("semantic_key", command.semantic_key.as_str(), 512usize),
        ("extractor_name", command.extractor_name.as_str(), 128usize),
        (
            "extractor_version",
            command.extractor_version.as_str(),
            128usize,
        ),
    ];
    for (name, value, max_len) in bounded_text {
        if value.is_empty()
            || value.len() > max_len
            || value.trim() != value
            || value.chars().any(char::is_control)
        {
            return Err(RepositoryError::Conflict(format!(
                "{name} must be trimmed, non-empty, bounded, and contain no control characters"
            )));
        }
    }
    if !(16..=128).contains(&command.idempotency_key.len()) {
        return Err(RepositoryError::Conflict(
            "candidate Idempotency-Key must contain 16 to 128 characters".into(),
        ));
    }
    if command.source_type != "conversation_message"
        || !SUPPORTED_EXTRACTED_MEMORY_TYPES.contains(&command.memory_type.as_str())
        || !matches!(command.scope_kind.as_str(), "user" | "project")
        || command
            .scope_id
            .as_deref()
            .is_none_or(|id| id.trim().is_empty() || id.len() > 256)
        || (command.memory_type == "project_fact") != (command.scope_kind == "project")
        || !matches!(command.inference_kind.as_str(), "explicit" | "derived")
        || !command.confidence.is_finite()
        || !(0.0..=1.0).contains(&command.confidence)
        || !command.importance.is_finite()
        || !(0.0..=1.0).contains(&command.importance)
    {
        return Err(RepositoryError::Conflict(
            "candidate type, inference metadata, or score is invalid".into(),
        ));
    }
    if command.inference_kind == "derived"
        && (command.confidence
            < if command.memory_type == "project_fact" {
                0.90
            } else {
                0.80
            }
            || !(3..=128).contains(&command.semantic_key.len())
            || !command.semantic_key.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            }))
    {
        return Err(RepositoryError::Conflict(
            "derived candidates require confidence >= 0.80 and a normalized semantic_key".into(),
        ));
    }
    Ok(())
}

fn validate_extraction_snapshot_command(
    command: &ExtractionSnapshotCommand,
) -> Result<(), RepositoryError> {
    if !(16..=128).contains(&command.idempotency_key.len())
        || command.idempotency_key.trim() != command.idempotency_key
        || command.idempotency_key.chars().any(char::is_control)
        || command.request_hash.len() != 71
        || !command.request_hash.starts_with("sha256:")
        || !command.request_hash[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || command.extractor_version.trim().is_empty()
        || command.extractor_version.trim() != command.extractor_version
        || command.extractor_version.len() > 256
        || command.extractor_version.chars().any(char::is_control)
    {
        return Err(RepositoryError::Conflict(
            "extraction idempotency metadata violated the snapshot contract".into(),
        ));
    }
    Ok(())
}

fn validate_extraction_proposals(
    proposals: &[ExtractCandidateProposal],
) -> Result<(), RepositoryError> {
    if proposals.len() > 8 {
        return Err(RepositoryError::Conflict(
            "an extraction snapshot may contain at most 8 proposals".into(),
        ));
    }
    let mut identities = HashSet::new();
    for proposal in proposals {
        if !SUPPORTED_EXTRACTED_MEMORY_TYPES.contains(&proposal.memory_type.as_str())
            || !(3..=128).contains(&proposal.semantic_key.len())
            || !proposal.semantic_key.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
            || proposal.content.trim().is_empty()
            || proposal.content.trim() != proposal.content
            || proposal.content.len() > 2_048
            || proposal.content.chars().any(char::is_control)
            || !proposal.confidence.is_finite()
            || !(0.80..=1.0).contains(&proposal.confidence)
            || !proposal.importance.is_finite()
            || !(0.0..=1.0).contains(&proposal.importance)
            || !identities.insert((
                proposal.memory_type.as_str(),
                proposal.semantic_key.as_str(),
            ))
        {
            return Err(RepositoryError::Conflict(
                "extraction proposal violated the canonical snapshot contract".into(),
            ));
        }
    }
    Ok(())
}

fn preference_semantic_key(content: &str) -> String {
    let normalized = content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let delimiter = normalized
        .find('：')
        .or_else(|| normalized.find(':'))
        .filter(|index| *index > 0);
    if let Some(index) = delimiter {
        let key = normalized[..index].trim();
        let value =
            normalized[index + normalized[index..].chars().next().unwrap().len_utf8()..].trim();
        if !value.is_empty() && key.chars().count() <= 128 {
            return key.to_owned();
        }
    }
    normalized
}

async fn load_ingest_operation_id_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    source_event_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT operation_id FROM memory_core.memory_ingest_results \
         WHERE tenant_id=$1 AND source_event_id=$2",
    )
    .bind(tenant_id)
    .bind(source_event_id)
    .fetch_optional(&mut **tx)
    .await
}

#[allow(clippy::too_many_arguments)]
async fn insert_ingest_result_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    source_event_id: Uuid,
    candidate_id: Uuid,
    memory_record_id: Uuid,
    memory_version_id: Uuid,
    operation_id: Uuid,
    result_kind: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO memory_core.memory_ingest_results (\
             tenant_id,source_event_id,candidate_id,memory_record_id,memory_version_id,\
             operation_id,result_kind\
         ) VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(tenant_id)
    .bind(source_event_id)
    .bind(candidate_id)
    .bind(memory_record_id)
    .bind(memory_version_id)
    .bind(operation_id)
    .bind(result_kind)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn set_tenant(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(tenant_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn validate_generation_name(generation: &str) -> Result<(), RepositoryError> {
    if !(1..=128).contains(&generation.len())
        || generation.trim() != generation
        || !generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RepositoryError::Conflict(
            "generation must contain 1 to 128 ASCII letters, digits, dot, dash or underscore"
                .into(),
        ));
    }
    Ok(())
}

async fn lock_projection_generation_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
) -> Result<(), RepositoryError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("projection-generation:{tenant_id}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn ensure_default_projection_generation_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
) -> Result<(), RepositoryError> {
    let counts = sqlx::query(
        "SELECT \
             count(*) FILTER (WHERE h.state='active')::integer AS expected_records,\
             count(*) FILTER (\
                 WHERE h.state='active' AND EXISTS (\
                     SELECT 1 FROM memory_core.projection_operations po \
                     WHERE po.tenant_id=h.tenant_id \
                       AND po.memory_record_id=h.memory_record_id \
                       AND po.memory_version_id=h.current_version_id \
                       AND po.generation=$2 AND po.status='ready'\
                 )\
             )::integer AS ready_records \
         FROM memory_core.memory_heads h WHERE h.tenant_id=$1",
    )
    .bind(tenant_id)
    .bind(DEMO_GENERATION)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO memory_core.projection_generations (\
             tenant_id,generation,status,expected_records,ready_records,activated_at,completed_at\
         ) SELECT $1,$2,'active',$3,$4,now(),now() \
         WHERE NOT EXISTS (\
             SELECT 1 FROM memory_core.projection_generations \
             WHERE tenant_id=$1 AND status IN ('active','canary')\
         ) ON CONFLICT (tenant_id,generation) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(DEMO_GENERATION)
    .bind(counts.try_get::<i32, _>("expected_records")?)
    .bind(counts.try_get::<i32, _>("ready_records")?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn active_projection_generation_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
) -> Result<String, RepositoryError> {
    sqlx::query_scalar::<_, String>(
        "SELECT generation FROM memory_core.projection_generations \
         WHERE tenant_id=$1 AND status='active'",
    )
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| RepositoryError::Invariant("tenant has no active projection generation".into()))
}

async fn current_projection_generation_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
) -> Result<String, RepositoryError> {
    sqlx::query_scalar::<_, String>(
        "SELECT generation FROM memory_core.projection_generations \
         WHERE tenant_id=$1 AND status IN ('canary','active') \
         ORDER BY CASE status WHEN 'canary' THEN 0 ELSE 1 END LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| {
        RepositoryError::Invariant("tenant has no readable projection generation".into())
    })
}

async fn projection_maintenance_in_progress_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
) -> Result<bool, RepositoryError> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM memory_core.projection_generations \
         WHERE tenant_id=$1 AND status IN ('building','ready','canary'))",
    )
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?)
}

async fn projection_write_generation_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
) -> Result<String, RepositoryError> {
    lock_projection_generation_tx(tx, tenant_id).await?;
    ensure_default_projection_generation_tx(tx, tenant_id).await?;
    if projection_maintenance_in_progress_tx(tx, tenant_id).await? {
        return Err(RepositoryError::Conflict(
            "projection rebuild maintenance is in progress; retry the write after finalize, rollback or abort"
                .into(),
        ));
    }
    active_projection_generation_tx(tx, tenant_id).await
}

async fn invalidate_projection_maintenance_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    reason: &str,
) -> Result<String, RepositoryError> {
    lock_projection_generation_tx(tx, tenant_id).await?;
    ensure_default_projection_generation_tx(tx, tenant_id).await?;
    let canary = sqlx::query_scalar::<_, String>(
        "SELECT generation FROM memory_core.projection_generations \
         WHERE tenant_id=$1 AND status='canary'",
    )
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(canary) = canary {
        let restored = sqlx::query_scalar::<_, String>(
            "UPDATE memory_core.projection_generations \
             SET status='active',activated_at=now(),retired_at=NULL \
             WHERE tenant_id=$1 AND status='standby' RETURNING generation",
        )
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| {
            RepositoryError::Invariant(
                "governance mutation found canary without standby generation".into(),
            )
        })?;
        sqlx::query(
            "UPDATE memory_core.projection_generations \
             SET status='failed',completed_at=now(),last_error_code=$3 \
             WHERE tenant_id=$1 AND generation=$2",
        )
        .bind(tenant_id)
        .bind(&canary)
        .bind(reason)
        .execute(&mut **tx)
        .await?;
        cancel_projection_generation_work_tx(tx, tenant_id, &canary, reason).await?;
        return Ok(restored);
    }
    let invalidated_generations = sqlx::query_scalar::<_, String>(
        "UPDATE memory_core.projection_generations \
         SET status='failed',completed_at=now(),last_error_code=$2 \
         WHERE tenant_id=$1 AND status IN ('building','ready') \
         RETURNING generation",
    )
    .bind(tenant_id)
    .bind(reason)
    .fetch_all(&mut **tx)
    .await?;
    for generation in invalidated_generations {
        cancel_projection_generation_work_tx(tx, tenant_id, &generation, reason).await?;
    }
    active_projection_generation_tx(tx, tenant_id).await
}

async fn cancel_projection_generation_work_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    generation: &str,
    error_code: &str,
) -> Result<(), RepositoryError> {
    sqlx::query(
        "UPDATE memory_core.outbox_events o \
         SET status='dead_letter',lease_owner=NULL,lease_until=NULL \
         FROM memory_core.projection_operations po \
         WHERE o.tenant_id=$1 \
           AND po.tenant_id=o.tenant_id \
           AND po.generation=$2 \
           AND po.id::text=o.payload->>'projection_operation_id' \
           AND o.status IN ('pending','leased')",
    )
    .bind(tenant_id)
    .bind(generation)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE memory_core.projection_operations \
         SET status='dead_letter',last_error_code=$3,lease_owner=NULL,lease_until=NULL,\
             completed_at=now(),updated_at=now() \
         WHERE tenant_id=$1 AND generation=$2 \
           AND status IN ('pending','leased','unknown','failed')",
    )
    .bind(tenant_id)
    .bind(generation)
    .bind(error_code)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn refresh_projection_generation_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    generation: &str,
) -> Result<(), RepositoryError> {
    let counts = sqlx::query(
        "SELECT \
             count(*) FILTER (WHERE status='ready')::integer AS ready_records,\
             count(*) FILTER (WHERE status IN ('dead_letter','stale'))::integer \
                 AS failed_records \
         FROM memory_core.projection_operations \
         WHERE tenant_id=$1 AND generation=$2 AND operation_type='upsert'",
    )
    .bind(tenant_id)
    .bind(generation)
    .fetch_one(&mut **tx)
    .await?;
    let ready_records: i32 = counts.try_get("ready_records")?;
    let failed_records: i32 = counts.try_get("failed_records")?;
    sqlx::query(
        "UPDATE memory_core.projection_generations \
         SET ready_records=$3,failed_records=$4,\
             status=CASE \
                 WHEN $4>0 THEN 'failed' \
                 WHEN $3=expected_records THEN 'ready' \
                 ELSE 'building' \
             END,\
             completed_at=CASE WHEN $4>0 OR $3=expected_records THEN now() ELSE NULL END,\
             last_error_code=CASE WHEN $4>0 THEN 'REBUILD_OPERATION_FAILED' ELSE NULL END \
         WHERE tenant_id=$1 AND generation=$2 AND status='building'",
    )
    .bind(tenant_id)
    .bind(generation)
    .bind(ready_records)
    .bind(failed_records)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn validate_projection_generation_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    generation: &str,
) -> Result<(), RepositoryError> {
    let canonical_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) \
         FROM memory_core.memory_heads h \
         JOIN memory_core.memory_versions v \
           ON v.tenant_id=h.tenant_id AND v.id=h.current_version_id \
         WHERE h.tenant_id=$1 AND h.state='active' AND v.content IS NOT NULL \
           AND (v.effective_from IS NULL OR v.effective_from<=transaction_timestamp()) \
           AND (v.effective_to IS NULL OR v.effective_to>transaction_timestamp()) \
           AND (v.retain_until IS NULL OR v.retain_until>transaction_timestamp()) \
           AND NOT EXISTS (SELECT 1 FROM memory_core.tombstones t \
               WHERE t.tenant_id=h.tenant_id AND t.memory_record_id=h.memory_record_id)",
    )
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;
    let valid_projection_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) \
         FROM memory_search.retrieval_documents rd \
         JOIN memory_core.memory_heads h \
           ON h.tenant_id=rd.tenant_id AND h.memory_record_id=rd.memory_record_id \
         JOIN memory_core.memory_versions v \
           ON v.tenant_id=rd.tenant_id AND v.id=rd.memory_version_id \
         WHERE rd.tenant_id=$1 AND rd.generation=$2 \
           AND h.state='active' AND h.current_version_id=rd.memory_version_id \
           AND h.revision=rd.head_revision AND v.content_hash=rd.content_hash \
           AND v.content IS NOT NULL \
           AND (v.effective_from IS NULL OR v.effective_from<=transaction_timestamp()) \
           AND (v.effective_to IS NULL OR v.effective_to>transaction_timestamp()) \
           AND (v.retain_until IS NULL OR v.retain_until>transaction_timestamp()) \
           AND NOT EXISTS (SELECT 1 FROM memory_core.tombstones t \
               WHERE t.tenant_id=rd.tenant_id AND t.memory_record_id=rd.memory_record_id)",
    )
    .bind(tenant_id)
    .bind(generation)
    .fetch_one(&mut **tx)
    .await?;
    let total_projection_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM memory_search.retrieval_documents \
         WHERE tenant_id=$1 AND generation=$2",
    )
    .bind(tenant_id)
    .bind(generation)
    .fetch_one(&mut **tx)
    .await?;
    if canonical_count != valid_projection_count || valid_projection_count != total_projection_count
    {
        return Err(RepositoryError::Conflict(format!(
            "projection generation validation failed: canonical={canonical_count}, valid_projection={valid_projection_count}, total_projection={total_projection_count}"
        )));
    }
    Ok(())
}

async fn load_projection_generation_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    generation: &str,
) -> Result<ProjectionGenerationView, RepositoryError> {
    let row = sqlx::query(
        "SELECT generation,status,source_generation,index_provider,embedding_provider,\
                embedding_model,embedding_revision,embedding_dimensions,\
                embedding_normalization,embedding_instruction_version,semantic_embedding,\
                model_manifest_hash,expected_records,ready_records,failed_records,created_at,\
                completed_at,activated_at,retired_at,last_error_code \
         FROM memory_core.projection_generations \
         WHERE tenant_id=$1 AND generation=$2",
    )
    .bind(tenant_id)
    .bind(generation)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RepositoryError::NotFound)?;
    Ok(ProjectionGenerationView {
        generation: row.try_get("generation")?,
        status: row.try_get("status")?,
        source_generation: row.try_get("source_generation")?,
        model_manifest: ProjectionModelManifest {
            index_provider: row.try_get("index_provider")?,
            embedding_provider: row.try_get("embedding_provider")?,
            embedding_model: row.try_get("embedding_model")?,
            embedding_revision: row.try_get("embedding_revision")?,
            embedding_dimensions: row.try_get("embedding_dimensions")?,
            embedding_normalization: row.try_get("embedding_normalization")?,
            embedding_instruction_version: row.try_get("embedding_instruction_version")?,
            semantic_embedding: row.try_get("semantic_embedding")?,
            manifest_hash: row.try_get("model_manifest_hash")?,
        },
        expected_records: row.try_get("expected_records")?,
        ready_records: row.try_get("ready_records")?,
        failed_records: row.try_get("failed_records")?,
        created_at: row.try_get("created_at")?,
        completed_at: row.try_get("completed_at")?,
        activated_at: row.try_get("activated_at")?,
        retired_at: row.try_get("retired_at")?,
        last_error_code: row.try_get("last_error_code")?,
    })
}

fn validate_generation_transition_idempotency_key(
    idempotency_key: &str,
) -> Result<(), RepositoryError> {
    if !(16..=128).contains(&idempotency_key.len()) {
        return Err(RepositoryError::Conflict(
            "generation transition Idempotency-Key must contain 16 to 128 characters".into(),
        ));
    }
    Ok(())
}

async fn replay_projection_generation_transition_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
    action: &str,
    target_generation: &str,
    request_hash: &str,
) -> Result<Option<ProjectionGenerationView>, RepositoryError> {
    let row = sqlx::query(
        "SELECT action,target_generation,request_hash,result_snapshot \
         FROM memory_core.projection_generation_transitions \
         WHERE tenant_id=$1 AND idempotency_key=$2",
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.try_get::<String, _>("action")? != action
        || row.try_get::<String, _>("target_generation")? != target_generation
        || row.try_get::<String, _>("request_hash")? != request_hash
    {
        return Err(RepositoryError::Conflict(
            "Idempotency-Key was already used for a different generation transition".into(),
        ));
    }
    let snapshot: serde_json::Value = row.try_get("result_snapshot")?;
    Ok(Some(serde_json::from_value(snapshot).map_err(|error| {
        RepositoryError::Invariant(format!(
            "stored projection generation transition snapshot is invalid: {error}"
        ))
    })?))
}

async fn record_projection_generation_transition_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
    action: &str,
    target_generation: &str,
    request_hash: &str,
    result: &ProjectionGenerationView,
) -> Result<(), RepositoryError> {
    sqlx::query(
        "INSERT INTO memory_core.projection_generation_transitions (\
             tenant_id,idempotency_key,action,target_generation,request_hash,result_generation,\
             result_snapshot\
         ) VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .bind(action)
    .bind(target_generation)
    .bind(request_hash)
    .bind(&result.generation)
    .bind(serde_json::to_value(result).map_err(|error| {
        RepositoryError::Invariant(format!(
            "projection generation transition could not be serialized: {error}"
        ))
    })?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn operation_exists_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    operation_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM memory_core.projection_operations \
         WHERE tenant_id=$1 AND id=$2)",
    )
    .bind(tenant_id)
    .bind(operation_id)
    .fetch_one(&mut **tx)
    .await
}

async fn load_erasure_request_tx(
    tx: &mut Transaction<'_, Postgres>,
    authority: &DemoAuthority,
    erasure_request_id: Uuid,
) -> Result<ErasureRequestView, RepositoryError> {
    let row = sqlx::query(
        "SELECT id,target_kind,target_id,status,record_count,handler_status,\
                requested_by_hash,created_at,completed_at \
         FROM memory_core.erasure_requests WHERE tenant_id=$1 AND id=$2",
    )
    .bind(&authority.tenant_id)
    .bind(erasure_request_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RepositoryError::NotFound)?;
    if row.try_get::<String, _>("requested_by_hash")? != principal_hash(authority) {
        return Err(RepositoryError::NotFound);
    }
    let record_rows = sqlx::query(
        "SELECT memory_record_id,status,projection_operation_id,last_error_code,completed_at \
         FROM memory_core.erasure_request_records \
         WHERE tenant_id=$1 AND erasure_request_id=$2 ORDER BY memory_record_id",
    )
    .bind(&authority.tenant_id)
    .bind(erasure_request_id)
    .fetch_all(&mut **tx)
    .await?;
    let records = record_rows
        .into_iter()
        .map(|record| {
            Ok(ErasureRecordView {
                memory_record_id: record.try_get("memory_record_id")?,
                status: record.try_get("status")?,
                projection_operation_id: record.try_get("projection_operation_id")?,
                last_error_code: record.try_get("last_error_code")?,
                completed_at: record.try_get("completed_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(ErasureRequestView {
        erasure_request_id: row.try_get("id")?,
        target_kind: row.try_get("target_kind")?,
        target_ref: row.try_get("target_id")?,
        status: row.try_get("status")?,
        record_count: row.try_get("record_count")?,
        handler_status: row.try_get("handler_status")?,
        created_at: row.try_get("created_at")?,
        completed_at: row.try_get("completed_at")?,
        records,
    })
}

async fn load_operation_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    operation_id: Uuid,
) -> Result<OperationView, RepositoryError> {
    let row = sqlx::query(
        "SELECT po.id,po.memory_record_id,po.memory_version_id,po.operation_type,\
                po.status,po.provider_object_id,po.attempts,po.last_error_code,\
                h.state AS head_state,h.revision AS head_revision,v.version_no,\
                COALESCE(\
                    (SELECT rr.candidate_id FROM memory_core.memory_review_results rr \
                     WHERE rr.tenant_id=po.tenant_id AND rr.operation_id=po.id),\
                    (SELECT mir.candidate_id FROM memory_core.memory_ingest_results mir \
                     WHERE mir.tenant_id=po.tenant_id AND mir.operation_id=po.id \
                       AND mir.result_kind='needs_review' LIMIT 1),\
                    d.candidate_id\
                ) AS candidate_id,\
                COALESCE((\
                    SELECT md.reason_code \
                    FROM memory_core.memory_review_results rr \
                    JOIN memory_core.memory_decisions md \
                      ON md.tenant_id=rr.tenant_id AND md.candidate_id=rr.candidate_id \
                     AND md.outcome=rr.outcome \
                    WHERE rr.tenant_id=po.tenant_id AND rr.operation_id=po.id\
                ),(\
                    SELECT md.reason_code \
                    FROM memory_core.memory_ingest_results mir \
                    JOIN memory_core.memory_decisions md \
                      ON md.tenant_id=mir.tenant_id AND md.candidate_id=mir.candidate_id \
                     AND md.outcome='needs_review' \
                    WHERE mir.tenant_id=po.tenant_id AND mir.operation_id=po.id \
                      AND mir.result_kind='needs_review' LIMIT 1\
                )) AS review_reason_code,\
                EXISTS(\
                    SELECT 1 FROM memory_core.memory_review_results rr \
                    WHERE rr.tenant_id=po.tenant_id AND rr.operation_id=po.id\
                ) AS is_review_operation,\
                EXISTS(\
                    SELECT 1 FROM memory_core.erasure_request_records err \
                    WHERE err.tenant_id=po.tenant_id \
                      AND err.projection_operation_id=po.id\
                ) AS is_erasure_operation \
         FROM memory_core.projection_operations po \
         JOIN memory_core.memory_heads h \
           ON h.tenant_id=po.tenant_id AND h.memory_record_id=po.memory_record_id \
         LEFT JOIN memory_core.memory_versions v \
           ON v.tenant_id=po.tenant_id AND v.id=po.memory_version_id \
         LEFT JOIN memory_core.memory_decisions d \
           ON d.tenant_id=v.tenant_id AND d.id=v.decision_id \
         WHERE po.tenant_id=$1 AND po.id=$2",
    )
    .bind(tenant_id)
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RepositoryError::NotFound)?;
    let status_raw: String = row.try_get("status")?;
    let projection_status = ProjectionOperationStatus::try_from(status_raw.as_str())
        .map_err(RepositoryError::Invariant)?;
    let head_state: String = row.try_get("head_state")?;
    let operation_type: String = row.try_get("operation_type")?;
    let version_no: Option<i32> = row.try_get("version_no")?;
    let is_review_operation: bool = row.try_get("is_review_operation")?;
    let is_erasure_operation: bool = row.try_get("is_erasure_operation")?;
    let canonical_status = match head_state.as_str() {
        "erased"
            if is_erasure_operation && projection_status == ProjectionOperationStatus::Deleted =>
        {
            CanonicalStatus::ErasureComplete
        }
        "erased" if is_erasure_operation => CanonicalStatus::ErasurePending,
        "contested" => CanonicalStatus::NeedsReview,
        "active" if is_review_operation => CanonicalStatus::MemoryAccepted,
        "active" if operation_type == "upsert" && version_no.is_some_and(|value| value > 1) => {
            CanonicalStatus::MemoryCorrected
        }
        "active" => CanonicalStatus::MemoryAccepted,
        "expired" => CanonicalStatus::Expired,
        "forgotten" | "erased" => CanonicalStatus::Forgotten,
        _ => CanonicalStatus::Failed,
    };
    Ok(OperationView {
        operation_id: row.try_get("id")?,
        candidate_id: row.try_get("candidate_id")?,
        memory_record_id: row.try_get("memory_record_id")?,
        memory_version_id: row.try_get("memory_version_id")?,
        head_revision: row.try_get("head_revision")?,
        canonical_status,
        projection_status,
        reason_code: row
            .try_get::<Option<String>, _>("last_error_code")?
            .or(row.try_get::<Option<String>, _>("review_reason_code")?),
        provider_object_id: row.try_get("provider_object_id")?,
        attempts: row.try_get("attempts")?,
    })
}

async fn fence_operation(
    tx: &mut Transaction<'_, Postgres>,
    authority: &DemoAuthority,
    work: &ProjectionWork,
    status: &str,
    provider_object_id: Option<&str>,
    error_code: Option<&str>,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE memory_core.projection_operations \
         SET status=$5,provider_object_id=COALESCE($6,provider_object_id),\
             last_error_code=$7,lease_owner=NULL,lease_until=NULL,\
             completed_at=now(),updated_at=now() \
         WHERE tenant_id=$1 AND id=$2 AND lease_owner=$3 AND lease_version=$4",
    )
    .bind(&authority.tenant_id)
    .bind(work.operation_id)
    .bind(&work.lease_owner)
    .bind(work.lease_version)
    .bind(status)
    .bind(provider_object_id)
    .bind(error_code)
    .execute(&mut **tx)
    .await?;
    if result.rows_affected() != 1 {
        return Err(RepositoryError::LeaseLost);
    }
    Ok(())
}

async fn replace_retrieval_terms_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    memory_record_id: Uuid,
    memory_version_id: Uuid,
    generation: &str,
    content: &str,
) -> Result<(), RepositoryError> {
    sqlx::query(
        "DELETE FROM memory_search.retrieval_terms \
         WHERE tenant_id=$1 AND memory_version_id=$2 AND generation=$3",
    )
    .bind(tenant_id)
    .bind(memory_version_id)
    .bind(generation)
    .execute(&mut **tx)
    .await?;

    let tokens = CHINESE_TOKENIZER.tokenize(content);
    if tokens.is_empty() {
        return Ok(());
    }
    let document_length = i32::try_from(tokens.len())
        .map_err(|_| RepositoryError::Invariant("BM25 document length overflowed".into()))?;
    let mut frequencies = HashMap::<String, i32>::new();
    for term in tokens {
        let frequency = frequencies.entry(term).or_default();
        *frequency = frequency
            .checked_add(1)
            .ok_or_else(|| RepositoryError::Invariant("BM25 term frequency overflowed".into()))?;
    }
    let mut frequencies = frequencies.into_iter().collect::<Vec<_>>();
    frequencies.sort_by(|left, right| left.0.cmp(&right.0));
    let terms = frequencies
        .iter()
        .map(|(term, _)| term.clone())
        .collect::<Vec<_>>();
    let term_frequencies = frequencies
        .iter()
        .map(|(_, frequency)| *frequency)
        .collect::<Vec<_>>();
    sqlx::query(
        "INSERT INTO memory_search.retrieval_terms (\
             tenant_id,memory_record_id,memory_version_id,generation,term,\
             term_frequency,document_length,tokenizer_version\
         ) \
         SELECT $1,$2,$3,$4,term,term_frequency,$7,$8 \
         FROM unnest($5::text[],$6::int[]) AS tokens(term,term_frequency)",
    )
    .bind(tenant_id)
    .bind(memory_record_id)
    .bind(memory_version_id)
    .bind(generation)
    .bind(&terms)
    .bind(&term_frequencies)
    .bind(document_length)
    .bind(CHINESE_TOKENIZER_VERSION)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn update_erasure_projection_status_tx(
    tx: &mut Transaction<'_, Postgres>,
    authority: &DemoAuthority,
    operation_id: Uuid,
    record_status: &str,
    error_code: Option<&str>,
) -> Result<(), RepositoryError> {
    let erasure_request_id = sqlx::query_scalar::<_, Uuid>(
        "UPDATE memory_core.erasure_request_records \
         SET status=$3,last_error_code=$4,\
             completed_at=CASE WHEN $3='complete' THEN now() ELSE NULL END \
         WHERE tenant_id=$1 AND projection_operation_id=$2 \
         RETURNING erasure_request_id",
    )
    .bind(&authority.tenant_id)
    .bind(operation_id)
    .bind(record_status)
    .bind(error_code)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(erasure_request_id) = erasure_request_id else {
        return Ok(());
    };
    let counts = sqlx::query(
        "SELECT count(*)::int AS total,\
                count(*) FILTER (WHERE status='complete')::int AS completed,\
                count(*) FILTER (WHERE status='failed')::int AS failed \
         FROM memory_core.erasure_request_records \
         WHERE tenant_id=$1 AND erasure_request_id=$2",
    )
    .bind(&authority.tenant_id)
    .bind(erasure_request_id)
    .fetch_one(&mut **tx)
    .await?;
    let total: i32 = counts.try_get("total")?;
    let completed: i32 = counts.try_get("completed")?;
    let failed: i32 = counts.try_get("failed")?;
    let (request_status, projection_status) = if failed > 0 {
        ("failed", "failed")
    } else if completed == total {
        ("complete", "complete")
    } else {
        ("processing", "pending")
    };
    sqlx::query(
        "UPDATE memory_core.erasure_requests \
         SET status=$3,handler_status=$4,\
             completed_at=CASE WHEN $3='complete' THEN now() ELSE NULL END \
         WHERE tenant_id=$1 AND id=$2",
    )
    .bind(&authority.tenant_id)
    .bind(erasure_request_id)
    .bind(request_status)
    .bind(json!({
        "canonical": "complete",
        "projection": projection_status,
        "completed_records": completed,
        "failed_records": failed
    }))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn complete_outbox(
    tx: &mut Transaction<'_, Postgres>,
    authority: &DemoAuthority,
    work: &ProjectionWork,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE memory_core.outbox_events \
         SET status='processed',processed_at=now(),lease_owner=NULL,lease_until=NULL \
         WHERE tenant_id=$1 AND id=$2 AND lease_owner=$3",
    )
    .bind(&authority.tenant_id)
    .bind(work.outbox_id)
    .bind(&work.lease_owner)
    .execute(&mut **tx)
    .await?;
    if result.rows_affected() != 1 {
        return Err(RepositoryError::LeaseLost);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn test_projection_model_manifest() -> ProjectionModelManifest {
        ProjectionModelManifest::new(
            "mem0".into(),
            "test-semantic".into(),
            "test-embedding-model".into(),
            "test-revision".into(),
            1024,
            "l2".into(),
            "memory-retrieval-v1".into(),
            true,
        )
        .unwrap()
    }

    async fn compile_test_pack(
        repository: &MemoryRepository,
        authority: &DemoAuthority,
        label: &str,
    ) -> (OperationView, ContextPackManifestView) {
        let memory_content = format!("先讲原理再讲代码 {label}");
        let operation = repository
            .ingest_explicit(
                authority,
                IngestCommand {
                    idempotency_key: format!("integration-ingest-{label}"),
                    event_id: format!("integration-event-{label}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:integration:{label}"),
                    source_content: format!("请记住：{memory_content}"),
                    memory_content: memory_content.clone(),
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        let memory_version_id = operation.memory_version_id.unwrap();
        let command = ContextPackCommand {
            task_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            purpose: "answer_user".into(),
            query: "回答技术问题采用什么顺序？".into(),
            limit: 5,
            memory_types: vec!["preference".into()],
            ranking_version: "transparent-lexical-v1".into(),
            projection_generation: DEMO_GENERATION.into(),
            token_budget: 512,
            tokenizer_id: DEMO_CONTEXT_TOKENIZER_ID.into(),
            memory_required: true,
            pinned_memory_version_ids: Vec::new(),
            temporal_window_hash: None,
        };
        let manifest = repository
            .compile_context_pack(
                authority,
                &command,
                &[ProjectionHit {
                    provider_object_id: format!("provider-{label}"),
                    memory_record_id: operation.memory_record_id,
                    memory_version_id,
                    head_revision: u64::try_from(operation.head_revision).unwrap(),
                    generation: DEMO_GENERATION.into(),
                    content_hash: content_hash(&memory_content),
                    provider_score: 1.0,
                }],
            )
            .await
            .unwrap();
        (operation, manifest)
    }

    async fn create_test_conflict(
        repository: &MemoryRepository,
        authority: &DemoAuthority,
        label: &str,
    ) -> (
        OperationView,
        ContextPackManifestView,
        OperationView,
        IngestCommand,
    ) {
        let original_content = format!("审核键{label}：原值");
        let original = repository
            .ingest_explicit(
                authority,
                IngestCommand {
                    idempotency_key: format!("review-original-{label}"),
                    event_id: format!("review-original-event-{label}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:review:{label}:original"),
                    source_content: format!("请记住：{original_content}"),
                    memory_content: original_content.clone(),
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        let original_version_id = original.memory_version_id.unwrap();
        let pack_command = ContextPackCommand {
            task_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            purpose: "answer_user".into(),
            query: format!("审核键{label}是什么？"),
            limit: 5,
            memory_types: vec!["preference".into()],
            ranking_version: "transparent-lexical-v1".into(),
            projection_generation: DEMO_GENERATION.into(),
            token_budget: 512,
            tokenizer_id: DEMO_CONTEXT_TOKENIZER_ID.into(),
            memory_required: true,
            pinned_memory_version_ids: Vec::new(),
            temporal_window_hash: None,
        };
        let manifest = repository
            .compile_context_pack(
                authority,
                &pack_command,
                &[ProjectionHit {
                    provider_object_id: format!("provider-review-{label}"),
                    memory_record_id: original.memory_record_id,
                    memory_version_id: original_version_id,
                    head_revision: u64::try_from(original.head_revision).unwrap(),
                    generation: DEMO_GENERATION.into(),
                    content_hash: content_hash(&original_content),
                    provider_score: 1.0,
                }],
            )
            .await
            .unwrap();
        let conflicting_content = format!("审核键{label}：冲突新值");
        let conflict_command = IngestCommand {
            idempotency_key: format!("review-conflict-{label}"),
            event_id: format!("review-conflict-event-{label}"),
            source_type: "conversation_message".into(),
            source_ref: format!("conversation:review:{label}:conflict"),
            source_content: format!("请记住：{conflicting_content}"),
            memory_content: conflicting_content,
            occurred_at: Utc::now(),
        };
        let conflict = repository
            .ingest_explicit(authority, conflict_command.clone())
            .await
            .unwrap();
        (original, manifest, conflict, conflict_command)
    }

    async fn create_rescoped_test_memory(
        repository: &MemoryRepository,
        authority: &DemoAuthority,
        label: &str,
        scope_kind: &str,
        scope_id: Option<&str>,
        memory_type: &str,
    ) -> (OperationView, String) {
        let memory_content = format!("typed scope {label}: verified value");
        let operation = repository
            .ingest_explicit(
                authority,
                IngestCommand {
                    idempotency_key: format!("typed-scope-ingest-{label}"),
                    event_id: format!("typed-scope-event-{label}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:typed-scope:{label}"),
                    source_content: format!("请记住：{memory_content}"),
                    memory_content: memory_content.clone(),
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        let mut tx = repository.pool.begin().await.unwrap();
        set_tenant(&mut tx, &authority.tenant_id).await.unwrap();
        sqlx::query(
            "UPDATE memory_core.memory_records \
             SET scope_kind=$3,scope_id=$4,memory_type=$5 \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(operation.memory_record_id)
        .bind(scope_kind)
        .bind(scope_id)
        .bind(memory_type)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE memory_core.memory_candidates \
             SET scope_kind=$3,scope_id=$4,memory_type=$5 \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(operation.candidate_id.unwrap())
        .bind(scope_kind)
        .bind(scope_id)
        .bind(memory_type)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        (operation, memory_content)
    }

    #[test]
    fn stable_ids_are_repeatable_and_purpose_specific() {
        assert_eq!(stable_id("source:key-1"), stable_id("source:key-1"));
        assert_ne!(stable_id("source:key-1"), stable_id("record:key-1"));
    }

    #[test]
    fn namespace_is_opaque_and_scope_bound() {
        let user_namespace = namespace_key("tenant-secret", "user", Some("alice"));
        let other_user_namespace = namespace_key("tenant-secret", "user", Some("bob"));
        assert!(user_namespace.starts_with("ns-"));
        assert!(!user_namespace.contains("tenant-secret"));
        assert!(!user_namespace.contains("alice"));
        assert_ne!(user_namespace, other_user_namespace);
    }

    #[test]
    fn content_hash_is_stable_and_prefixed() {
        let hash = content_hash("请记住：先讲原理");
        assert_eq!(hash, content_hash("请记住：先讲原理"));
        assert!(hash.starts_with("sha256:"));
    }

    #[test]
    fn preference_semantic_key_normalizes_exact_and_key_value_preferences() {
        assert_eq!(
            preference_semantic_key("  Use   Concise Answers  "),
            "use concise answers"
        );
        assert_eq!(
            preference_semantic_key("回答风格：先讲原理"),
            preference_semantic_key("回答风格：先给结论")
        );
        assert_ne!(
            preference_semantic_key("回答风格：先讲原理"),
            preference_semantic_key("代码语言：Rust")
        );
    }

    #[test]
    fn derived_candidate_requires_normalized_identity_and_high_confidence() {
        let command = IngestCandidateCommand {
            idempotency_key: "extract-derived-candidate-test-key".into(),
            event_id: "extract-event-test".into(),
            source_type: "conversation_message".into(),
            source_ref: "conversation:test#memory-style".into(),
            source_content: "我希望回答先讲原理".into(),
            memory_content: "用户偏好回答先讲原理".into(),
            occurred_at: Utc::now(),
            memory_type: "preference".into(),
            semantic_key: "user.answer.style".into(),
            confidence: 0.91,
            importance: 0.8,
            inference_kind: "derived".into(),
            extractor_name: "test-extractor".into(),
            extractor_version: "v1".into(),
            scope_kind: "user".into(),
            scope_id: Some("alice".into()),
        };
        validate_ingest_candidate(&command).unwrap();
        let mut invalid = command.clone();
        invalid.confidence = 0.79;
        assert!(validate_ingest_candidate(&invalid).is_err());
        invalid = command.clone();
        invalid.semantic_key = "用户.偏好".into();
        assert!(validate_ingest_candidate(&invalid).is_err());

        let mut project = command;
        project.memory_type = "project_fact".into();
        project.semantic_key = "project.architecture.authority_store".into();
        project.scope_kind = "project".into();
        project.scope_id = Some("project-a".into());
        project.confidence = 0.90;
        validate_ingest_candidate(&project).unwrap();
        project.confidence = 0.89;
        assert!(validate_ingest_candidate(&project).is_err());
    }

    #[test]
    fn extraction_snapshot_accepts_zero_candidates_and_rejects_duplicate_identity() {
        validate_extraction_proposals(&[]).unwrap();
        let proposal = ExtractCandidateProposal {
            memory_type: "preference".into(),
            semantic_key: "user.answer.style".into(),
            content: "回答先给结论".into(),
            confidence: 0.92,
            importance: 0.8,
        };
        validate_extraction_proposals(std::slice::from_ref(&proposal)).unwrap();
        assert!(validate_extraction_proposals(&[proposal.clone(), proposal]).is_err());
    }

    #[test]
    fn projection_model_manifest_hash_detects_drift() {
        let mut manifest = test_projection_model_manifest();
        manifest.embedding_revision = "different".into();
        assert!(manifest.validate_integrity().is_err());
    }

    #[test]
    fn context_budget_is_deterministic_and_conservative() {
        let first = AuthorizedMemory {
            memory_record_id: Uuid::new_v4(),
            memory_version_id: Uuid::new_v4(),
            content: "1234567890".into(),
            memory_type: "preference".into(),
            score: 1.0,
            source_refs: vec!["source:first".into()],
            effective_from: None,
        };
        let second = AuthorizedMemory {
            memory_record_id: Uuid::new_v4(),
            memory_version_id: Uuid::new_v4(),
            content: "12345".into(),
            memory_type: "preference".into(),
            score: 0.9,
            source_refs: vec!["source:second".into()],
            effective_from: None,
        };

        let evaluated = evaluate_context_budget(vec![first, second], 40);
        assert_eq!(evaluated[0].estimated_tokens, 42);
        assert!(!evaluated[0].injected);
        assert_eq!(evaluated[1].estimated_tokens, 37);
        assert!(evaluated[1].injected);
        assert_eq!(evaluated[0].rank, 1);
        assert_eq!(evaluated[1].rank, 2);
    }

    #[test]
    fn context_pack_hash_binds_attempt_and_budget() {
        let base = ContextPackCommand {
            task_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            purpose: "answer_user".into(),
            query: "解释 Outbox".into(),
            limit: 5,
            memory_types: vec!["preference".into()],
            ranking_version: "transparent-lexical-v1".into(),
            projection_generation: DEMO_GENERATION.into(),
            token_budget: 512,
            tokenizer_id: DEMO_CONTEXT_TOKENIZER_ID.into(),
            memory_required: true,
            pinned_memory_version_ids: Vec::new(),
            temporal_window_hash: None,
        };
        let mut changed = base.clone();
        changed.token_budget = 256;
        assert_eq!(
            context_pack_request_hash(&base),
            context_pack_request_hash(&base)
        );
        assert_ne!(
            context_pack_request_hash(&base),
            context_pack_request_hash(&changed)
        );

        let mut multi_type = base.clone();
        multi_type.memory_types = vec!["project_fact".into(), "preference".into()];
        let mut reordered = multi_type.clone();
        reordered.memory_types.reverse();
        assert_eq!(
            context_pack_request_hash(&multi_type),
            context_pack_request_hash(&reordered)
        );
        assert_ne!(
            context_pack_request_hash(&base),
            context_pack_request_hash(&multi_type)
        );
        let mut changed_ranking = base.clone();
        changed_ranking.ranking_version = "kimi-k3-semantic-v1".into();
        assert_ne!(
            context_pack_request_hash(&base),
            context_pack_request_hash(&changed_ranking)
        );
        let mut changed_generation = base.clone();
        changed_generation.projection_generation = "generation-2".into();
        assert_ne!(
            context_pack_request_hash(&base),
            context_pack_request_hash(&changed_generation)
        );
        let mut changed_temporal_window = base.clone();
        changed_temporal_window.temporal_window_hash =
            Some("2026-07-28T16:00:00Z\0 2026-07-29T16:00:00Z\0 Asia/Shanghai\0 day".into());
        assert_ne!(
            context_pack_request_hash(&base),
            context_pack_request_hash(&changed_temporal_window)
        );
    }

    #[test]
    fn projection_generation_names_are_strict_and_bounded() {
        assert!(validate_generation_name("generation-2026.07_28").is_ok());
        assert!(validate_generation_name("").is_err());
        assert!(validate_generation_name(" generation").is_err());
        assert!(validate_generation_name("generation/unsafe").is_err());
        assert!(validate_generation_name(&"g".repeat(129)).is_err());
    }

    #[test]
    fn context_memory_types_map_to_safe_sections() {
        assert_eq!(
            context_item_metadata("preference").unwrap(),
            ("user_preference", "AUTHORIZED_USER_PREFERENCE")
        );
        assert_eq!(
            context_item_metadata("project_fact").unwrap(),
            ("current_fact", "AUTHORIZED_CURRENT_FACT")
        );
        assert_eq!(
            context_item_metadata("episodic_summary").unwrap(),
            ("relevant_experience", "AUTHORIZED_RELEVANT_EXPERIENCE")
        );
        assert!(context_item_metadata("procedural_candidate").is_err());
    }

    #[test]
    fn lexical_retrieval_score_is_language_agnostic_and_deterministic() {
        let exact = lexical_relevance_score("先讲原理", "回答技术问题时先讲原理，再给代码");
        let partial = lexical_relevance_score("先讲原理", "先说明核心原理");
        let irrelevant = lexical_relevance_score("先讲原理", "数据库使用 PostgreSQL");
        assert!(exact > partial);
        assert!(partial > irrelevant);
        assert_eq!(
            exact,
            lexical_relevance_score("先讲原理", "回答技术问题时先讲原理，再给代码")
        );
        assert_eq!(lexical_relevance_score("", "content"), 0.0);
    }

    #[test]
    fn consume_hash_binds_pack_and_attempt() {
        let pack_id = Uuid::new_v4();
        let base = ConsumeContextPackCommand {
            idempotency_key: "consume-idempotency-key-1".into(),
            task_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            purpose: "answer_user".into(),
        };
        let mut changed = base.clone();
        changed.attempt_id = Uuid::new_v4();
        assert_eq!(
            context_pack_consume_request_hash(pack_id, &base),
            context_pack_consume_request_hash(pack_id, &base)
        );
        assert_ne!(
            context_pack_consume_request_hash(pack_id, &base),
            context_pack_consume_request_hash(pack_id, &changed)
        );
        assert_ne!(
            context_pack_consume_request_hash(pack_id, &base),
            context_pack_consume_request_hash(Uuid::new_v4(), &base)
        );
    }

    #[test]
    fn correction_hash_binds_record_revision_and_content() {
        let base = CorrectionCommand {
            idempotency_key: "correction-hash-test-key".into(),
            memory_record_id: Uuid::new_v4(),
            expected_revision: 0,
            new_content: "回答风格：先给结论".into(),
            reason: "用户明确修正".into(),
            source_ref: "conversation:correction:test".into(),
            effective_from: None,
        };
        let mut changed = base.clone();
        changed.expected_revision = 1;
        assert_eq!(
            correction_request_hash(&base),
            correction_request_hash(&base)
        );
        assert_ne!(
            correction_request_hash(&base),
            correction_request_hash(&changed)
        );
    }

    #[test]
    fn review_hash_binds_candidate_revision_and_outcome() {
        let base = ReviewDecisionCommand {
            idempotency_key: "review-hash-test-key".into(),
            candidate_id: Uuid::new_v4(),
            expected_candidate_revision: 0,
            outcome: ReviewOutcome::Accept,
            reason: "用户确认".into(),
        };
        let mut changed = base.clone();
        changed.outcome = ReviewOutcome::Reject;
        assert_eq!(review_request_hash(&base), review_request_hash(&base));
        assert_ne!(review_request_hash(&base), review_request_hash(&changed));
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn retrieval_explain_receipt_is_tenant_isolated_and_content_minimized() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("explain-test-{test_id}"), "alice");
        let memory_record_id = Uuid::new_v4();
        let memory_version_id = Uuid::new_v4();
        let command = RetrievalRunCommand {
            task_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            purpose: "answer_user".into(),
            query_hash: format!("sha256:{}", "a".repeat(64)),
            requested_limit: 5,
            candidate_limit: 20,
            reference_time: Utc::now(),
            timezone: "Asia/Shanghai".into(),
            temporal_window: None,
            projection_generation: DEMO_GENERATION.into(),
            ranking_version: "transparent-lexical-v1".into(),
            degraded: false,
            degradation_reason: None,
            candidates: vec![crate::model::RetrievalCandidateReceiptCommand {
                scope_kind: "user".into(),
                scope_id: Some("alice".into()),
                channel: "bm25".into(),
                memory_record_id,
                memory_version_id,
                channel_rank: 1,
                raw_score: 0.9,
                channel_weight: 1.15,
                rrf_contribution: 1.15 / 61.0,
                fused_score: Some(1.0),
                preauthorized: true,
                rerank_score: Some(0.95),
                postauthorized: true,
                final_rank: Some(1),
                included: true,
                exclusion_reason: None,
            }],
        };

        let created = repository
            .record_retrieval_run(&authority, &command)
            .await
            .unwrap();
        assert_eq!(created.query_hash, command.query_hash);
        assert_eq!(created.candidates.len(), 1);
        assert!(created.candidates[0].included);
        let loaded = repository
            .retrieval_run(&authority, created.retrieval_run_id)
            .await
            .unwrap();
        assert_eq!(loaded.candidates[0].memory_version_id, memory_version_id);

        let other_tenant = DemoAuthority::user(format!("other-{test_id}"), "alice");
        assert!(matches!(
            repository
                .retrieval_run(&other_tenant, created.retrieval_run_id)
                .await,
            Err(RepositoryError::NotFound)
        ));
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn ingest_redacts_pii_before_ledger_and_version_storage() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("security-test-{test_id}"), "alice");
        let idempotency_key = format!("security-ingest-{test_id}");
        let raw_source = format!("请记住：紧急联系电话是 13800138000，标识 {test_id}");
        let raw_memory = format!("紧急联系电话是 13800138000，标识 {test_id}");
        let operation = repository
            .ingest_explicit(
                &authority,
                IngestCommand {
                    idempotency_key: idempotency_key.clone(),
                    event_id: format!("security-event-{test_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:security:{test_id}"),
                    source_content: raw_source.clone(),
                    memory_content: raw_memory,
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap();

        let mut tx = repository.pool.begin().await.unwrap();
        set_tenant(&mut tx, &authority.tenant_id).await.unwrap();
        let source = sqlx::query(
            "SELECT content_hash,redacted_excerpt,redaction_policy_version,security_flags \
             FROM memory_core.source_events \
             WHERE tenant_id=$1 AND idempotency_key=$2",
        )
        .bind(&authority.tenant_id)
        .bind(&idempotency_key)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        assert_eq!(
            source.try_get::<String, _>("content_hash").unwrap(),
            content_hash(&raw_source)
        );
        assert_eq!(
            source
                .try_get::<String, _>("redaction_policy_version")
                .unwrap(),
            SECURITY_POLICY_VERSION
        );
        assert!(
            source
                .try_get::<Vec<String>, _>("security_flags")
                .unwrap()
                .contains(&"cn_phone".to_owned())
        );
        assert!(
            !source
                .try_get::<String, _>("redacted_excerpt")
                .unwrap()
                .contains("13800138000")
        );

        let version_content = sqlx::query_scalar::<_, String>(
            "SELECT content FROM memory_core.memory_versions \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(operation.memory_version_id.unwrap())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        assert!(!version_content.contains("13800138000"));
        assert!(version_content.contains("[REDACTED_cn_phone]"));
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn chinese_bm25_prefers_exact_identifier_and_is_generation_bound() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("bm25-test-{test_id}"), "alice");
        let documents = [
            ("identifier", "发布代号：玄武七号负责生产灰度开关"),
            ("generic", "发布策略：生产上线采用普通灰度发布流程"),
        ];
        let mut records = HashMap::new();
        for (label, content) in documents {
            let operation = repository
                .ingest_explicit(
                    &authority,
                    IngestCommand {
                        idempotency_key: format!("bm25-{label}-{test_id}"),
                        event_id: format!("bm25-event-{label}-{test_id}"),
                        source_type: "conversation_message".into(),
                        source_ref: format!("conversation:bm25:{label}:{test_id}"),
                        source_content: format!("请记住：{content}"),
                        memory_content: content.into(),
                        occurred_at: Utc::now(),
                    },
                )
                .await
                .unwrap();
            records.insert(label, operation.memory_record_id);
            let work = repository
                .claim_projection(&authority, &format!("bm25-worker-{label}"))
                .await
                .unwrap()
                .unwrap();
            repository
                .complete_ready(
                    &authority,
                    &work,
                    &format!("provider-bm25-{label}-{test_id}"),
                )
                .await
                .unwrap();
        }

        let hits = repository
            .search_bm25_projection(
                &authority,
                "user",
                Some("alice"),
                "玄武七号的灰度开关",
                DEMO_GENERATION,
                10,
            )
            .await
            .unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].memory_record_id, records["identifier"]);
        assert!(hits[0].provider_object_id.starts_with("local-bm25:"));

        let stale_generation = repository
            .search_bm25_projection(
                &authority,
                "user",
                Some("alice"),
                "玄武七号",
                "retired-generation",
                10,
            )
            .await
            .unwrap();
        assert!(stale_generation.is_empty());

        let mut tx = repository.pool.begin().await.unwrap();
        set_tenant(&mut tx, &authority.tenant_id).await.unwrap();
        let tokenizer_versions = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT tokenizer_version \
             FROM memory_search.retrieval_terms WHERE tenant_id=$1",
        )
        .bind(&authority.tenant_id)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(tokenizer_versions, vec![CHINESE_TOKENIZER_VERSION]);
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn temporal_candidates_use_source_time_and_still_require_canonical_authorization() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("temporal-test-{test_id}"), "alice");
        let reference_time = Utc.with_ymd_and_hms(2026, 7, 30, 4, 0, 0).unwrap();
        let episodes = [
            (
                "yesterday",
                "昨天修复了队列重复消费",
                Utc.with_ymd_and_hms(2026, 7, 29, 2, 0, 0).unwrap(),
            ),
            (
                "older",
                "十天前完成了数据库迁移",
                Utc.with_ymd_and_hms(2026, 7, 20, 2, 0, 0).unwrap(),
            ),
        ];
        let mut operations = HashMap::new();
        for (label, content, occurred_at) in episodes {
            let operation = repository
                .ingest_extracted(
                    &authority,
                    IngestCandidateCommand {
                        idempotency_key: format!("temporal-{label}-{test_id}"),
                        event_id: format!("temporal-event-{label}-{test_id}"),
                        source_type: "conversation_message".into(),
                        source_ref: format!("conversation:temporal:{label}:{test_id}"),
                        source_content: content.into(),
                        memory_content: content.into(),
                        occurred_at,
                        memory_type: "episodic_summary".into(),
                        semantic_key: format!("episode.temporal.{label}.{test_id}"),
                        confidence: 0.95,
                        importance: 0.8,
                        inference_kind: "derived".into(),
                        extractor_name: "temporal-test".into(),
                        extractor_version: "v1".into(),
                        scope_kind: "user".into(),
                        scope_id: Some("alice".into()),
                    },
                )
                .await
                .unwrap();
            let work = repository
                .claim_projection(&authority, &format!("temporal-worker-{label}"))
                .await
                .unwrap()
                .unwrap();
            repository
                .complete_ready(
                    &authority,
                    &work,
                    &format!("provider-temporal-{label}-{test_id}"),
                )
                .await
                .unwrap();
            operations.insert(label, operation);
        }

        let window = memory_application::parse_temporal_query(
            "昨天处理了什么？",
            reference_time,
            "Asia/Shanghai",
        )
        .unwrap()
        .unwrap();
        let hits = repository
            .search_temporal_projection(
                &authority,
                "user",
                Some("alice"),
                &window,
                DEMO_GENERATION,
                10,
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].memory_record_id,
            operations["yesterday"].memory_record_id
        );

        repository
            .forget(
                &authority,
                ForgetCommand {
                    idempotency_key: format!("temporal-forget-{test_id}"),
                    memory_record_id: operations["yesterday"].memory_record_id,
                    expected_revision: operations["yesterday"].head_revision,
                    reason: "temporal authorization regression".into(),
                },
            )
            .await
            .unwrap();
        let stale_projection_hits = repository
            .search_temporal_projection(
                &authority,
                "user",
                Some("alice"),
                &window,
                DEMO_GENERATION,
                10,
            )
            .await
            .unwrap();
        assert_eq!(stale_projection_hits.len(), 1);
        assert!(
            repository
                .authorized_hits(&authority, &stale_projection_hits)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn context_pack_retains_authorized_preference_below_topical_threshold() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("preference-ranking-test-{test_id}"), "alice");
        let content = format!("回答技术问题时先用生活类比 {test_id}");
        let operation = repository
            .ingest_explicit(
                &authority,
                IngestCommand {
                    idempotency_key: format!("preference-ranking-ingest-{test_id}"),
                    event_id: format!("preference-ranking-event-{test_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:preference-ranking:{test_id}"),
                    source_content: format!("请记住：{content}"),
                    memory_content: content.clone(),
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        let command = ContextPackCommand {
            task_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            purpose: "answer_user".into(),
            query: "PostgreSQL Outbox 如何工作？".into(),
            limit: 5,
            memory_types: vec!["preference".into()],
            ranking_version: "dedicated-test-v1".into(),
            projection_generation: DEMO_GENERATION.into(),
            token_budget: 512,
            tokenizer_id: DEMO_CONTEXT_TOKENIZER_ID.into(),
            memory_required: true,
            pinned_memory_version_ids: Vec::new(),
            temporal_window_hash: None,
        };
        let manifest = repository
            .compile_context_pack(
                &authority,
                &command,
                &[ProjectionHit {
                    provider_object_id: format!("provider-preference-ranking-{test_id}"),
                    memory_record_id: operation.memory_record_id,
                    memory_version_id: operation.memory_version_id.unwrap(),
                    head_revision: u64::try_from(operation.head_revision).unwrap(),
                    generation: DEMO_GENERATION.into(),
                    content_hash: content_hash(&content),
                    provider_score: 0.02,
                }],
            )
            .await
            .unwrap();
        assert_eq!(manifest.item_count, 1);
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn pinned_current_version_is_authorized_without_waiting_for_projection() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("pinned-context-test-{test_id}"), "alice");
        let content = format!("回答时先给结论 {test_id}");
        let operation = repository
            .ingest_explicit(
                &authority,
                IngestCommand {
                    idempotency_key: format!("pinned-ingest-{test_id}"),
                    event_id: format!("pinned-event-{test_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:pinned:{test_id}"),
                    source_content: format!("请记住：{content}"),
                    memory_content: content.clone(),
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            operation.projection_status,
            ProjectionOperationStatus::Pending
        );
        let command = ContextPackCommand {
            task_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            purpose: "answer_user".into(),
            query: "请解释事务型 Outbox".into(),
            limit: 5,
            memory_types: vec!["preference".into()],
            ranking_version: "dedicated-test-v1".into(),
            projection_generation: DEMO_GENERATION.into(),
            token_budget: 512,
            tokenizer_id: DEMO_CONTEXT_TOKENIZER_ID.into(),
            memory_required: true,
            pinned_memory_version_ids: vec![operation.memory_version_id.unwrap()],
            temporal_window_hash: None,
        };
        let manifest = repository
            .compile_context_pack(&authority, &command, &[])
            .await
            .unwrap();
        assert_eq!(manifest.item_count, 1);
        let pack = repository
            .consume_context_pack(
                &authority,
                manifest.context_pack_id,
                &ConsumeContextPackCommand {
                    idempotency_key: format!("pinned-consume-{test_id}"),
                    task_id: command.task_id,
                    attempt_id: command.attempt_id,
                    purpose: command.purpose,
                },
            )
            .await
            .unwrap();
        assert_eq!(pack.sections[0].items[0].text, content);
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn extracted_project_fact_is_project_scoped_authorized_and_conflict_safe() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let mut authority =
            DemoAuthority::user(format!("project-extraction-test-{test_id}"), "alice");
        authority.project_ids.insert("project-a".into());
        let base = IngestCandidateCommand {
            idempotency_key: format!("project-extraction-first-{test_id}"),
            event_id: format!("project-extraction-event-first-{test_id}"),
            source_type: "conversation_message".into(),
            source_ref: format!("conversation:project-extraction:{test_id}:first"),
            source_content: "当前项目使用 PostgreSQL 作为权威数据库".into(),
            memory_content: "当前项目使用PostgreSQL作为权威数据库。".into(),
            occurred_at: Utc::now(),
            memory_type: "project_fact".into(),
            semantic_key: "project.architecture.authority_store".into(),
            confidence: 0.94,
            importance: 0.9,
            inference_kind: "derived".into(),
            extractor_name: "test-extractor".into(),
            extractor_version: "v3".into(),
            scope_kind: "project".into(),
            scope_id: Some("project-a".into()),
        };
        let accepted = repository
            .ingest_extracted(&authority, base.clone())
            .await
            .unwrap();
        assert_eq!(accepted.canonical_status, CanonicalStatus::MemoryAccepted);
        let record = repository
            .inspect_memory(&authority, accepted.memory_record_id)
            .await
            .unwrap();
        assert_eq!(record.scope.kind, "project");
        assert_eq!(record.scope.id.as_deref(), Some("project-a"));
        assert_eq!(record.versions[0].classification, "internal");

        let outsider = DemoAuthority::user(authority.tenant_id.clone(), "bob");
        assert!(matches!(
            repository
                .inspect_memory(&outsider, accepted.memory_record_id)
                .await,
            Err(RepositoryError::NotFound)
        ));
        let mut unauthorized = base.clone();
        unauthorized.idempotency_key = format!("project-extraction-denied-{test_id}");
        unauthorized.event_id = format!("project-extraction-event-denied-{test_id}");
        assert!(matches!(
            repository.ingest_extracted(&outsider, unauthorized).await,
            Err(RepositoryError::Unauthorized)
        ));

        let mut conflicting = base;
        conflicting.idempotency_key = format!("project-extraction-conflict-{test_id}");
        conflicting.event_id = format!("project-extraction-event-conflict-{test_id}");
        conflicting.source_ref = format!("conversation:project-extraction:{test_id}:conflict");
        conflicting.source_content = "当前项目改用 CockroachDB 作为权威数据库".into();
        conflicting.memory_content = "当前项目使用CockroachDB作为权威数据库。".into();
        let contested = repository
            .ingest_extracted(&authority, conflicting)
            .await
            .unwrap();
        assert_eq!(contested.canonical_status, CanonicalStatus::NeedsReview);
        assert_eq!(contested.memory_record_id, accepted.memory_record_id);
        assert_eq!(
            repository
                .inspect_memory(&authority, accepted.memory_record_id)
                .await
                .unwrap()
                .head_state,
            "contested"
        );
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn extraction_snapshot_freezes_first_result_and_persists_zero_candidates() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("extraction-test-{test_id}"), "alice");
        let command = ExtractionSnapshotCommand {
            idempotency_key: format!("extraction-snapshot-{test_id}"),
            request_hash: content_hash("same extraction request"),
            extractor_version: "test-extractor-v1".into(),
        };
        let pending = repository
            .begin_extraction_snapshot(&authority, &command)
            .await
            .unwrap();
        assert!(pending.proposals.is_none());

        let proposal = ExtractCandidateProposal {
            memory_type: "preference".into(),
            semantic_key: "user.answer.style".into(),
            content: "回答先给结论".into(),
            confidence: 0.93,
            importance: 0.8,
        };
        let completed = repository
            .complete_extraction_snapshot(&authority, &command, vec![proposal.clone()])
            .await
            .unwrap();
        assert_eq!(completed.proposals, Some(vec![proposal.clone()]));
        let replay = repository
            .begin_extraction_snapshot(&authority, &command)
            .await
            .unwrap();
        assert_eq!(replay.proposals, Some(vec![proposal]));

        let zero_command = ExtractionSnapshotCommand {
            idempotency_key: format!("extraction-snapshot-zero-{test_id}"),
            request_hash: content_hash("zero extraction request"),
            extractor_version: "test-extractor-v1".into(),
        };
        repository
            .begin_extraction_snapshot(&authority, &zero_command)
            .await
            .unwrap();
        let zero = repository
            .complete_extraction_snapshot(&authority, &zero_command, vec![])
            .await
            .unwrap();
        assert_eq!(zero.proposals, Some(vec![]));
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn projection_generation_rebuild_switch_rollback_and_finalize_are_safe() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("rebuild-test-{test_id}"), "alice");
        let worker = DemoAuthority::worker(authority.tenant_id.clone());

        let first = repository
            .ingest_explicit(
                &authority,
                IngestCommand {
                    idempotency_key: format!("rebuild-first-ingest-{test_id}"),
                    event_id: format!("rebuild-first-event-{test_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:rebuild:{test_id}:first"),
                    source_content: "请记住：回答风格先给结论".into(),
                    memory_content: "回答风格先给结论".into(),
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        let first_work = repository
            .claim_projection(&worker, &format!("rebuild-worker-{test_id}"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first_work.operation_id, first.operation_id);
        repository
            .complete_ready(&worker, &first_work, "provider-first")
            .await
            .unwrap();

        let rollback_generation = format!("rebuild-rollback-{test_id}");
        let rebuild = repository
            .start_projection_rebuild(
                &authority,
                ProjectionRebuildCommand {
                    idempotency_key: format!("rebuild-start-rollback-{test_id}"),
                    generation: rollback_generation.clone(),
                    model_manifest: test_projection_model_manifest(),
                },
            )
            .await
            .unwrap();
        assert_eq!(rebuild.status, "building");
        assert_eq!(rebuild.expected_records, 1);
        assert_eq!(
            repository
                .current_projection_generation(&authority)
                .await
                .unwrap(),
            DEMO_GENERATION
        );
        assert!(matches!(
            repository
                .ingest_explicit(
                    &authority,
                    IngestCommand {
                        idempotency_key: format!("rebuild-blocked-ingest-{test_id}"),
                        event_id: format!("rebuild-blocked-event-{test_id}"),
                        source_type: "conversation_message".into(),
                        source_ref: format!("conversation:rebuild:{test_id}:blocked"),
                        source_content: "请记住：重建期间不应提交".into(),
                        memory_content: "重建期间不应提交".into(),
                        occurred_at: Utc::now(),
                    },
                )
                .await,
            Err(RepositoryError::Conflict(_))
        ));
        let rebuild_work = repository
            .claim_projection(&worker, &format!("rebuild-worker-{test_id}"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rebuild_work.generation, rollback_generation);
        repository
            .reschedule(
                &worker,
                &rebuild_work,
                ProjectionOperationStatus::Failed,
                "TRANSIENT_PROVIDER_FAILURE",
            )
            .await
            .unwrap();
        let still_building = repository
            .projection_generation(&authority, &rollback_generation)
            .await
            .unwrap();
        assert_eq!(still_building.status, "building");
        assert_eq!(still_building.failed_records, 0);
        tokio::time::sleep(Duration::from_millis(600)).await;
        let rebuild_work = repository
            .claim_projection(&worker, &format!("rebuild-worker-{test_id}"))
            .await
            .unwrap()
            .unwrap();
        repository
            .complete_ready(&worker, &rebuild_work, "provider-rebuild-rollback")
            .await
            .unwrap();
        let ready = repository
            .projection_generation(&authority, &rollback_generation)
            .await
            .unwrap();
        assert_eq!(ready.status, "ready");
        assert_eq!(ready.ready_records, ready.expected_records);

        let activate_key = format!("rebuild-activate-rollback-{test_id}");
        let canary = repository
            .activate_projection_generation(
                &authority,
                &rollback_generation,
                DEMO_GENERATION,
                &activate_key,
            )
            .await
            .unwrap();
        assert_eq!(canary.status, "canary");
        let replayed_canary = repository
            .activate_projection_generation(
                &authority,
                &rollback_generation,
                DEMO_GENERATION,
                &activate_key,
            )
            .await
            .unwrap();
        assert_eq!(replayed_canary.status, "canary");
        assert_eq!(
            repository
                .current_projection_generation(&authority)
                .await
                .unwrap(),
            rollback_generation
        );
        let rollback_key = format!("rebuild-rollback-action-{test_id}");
        let restored = repository
            .rollback_projection_generation(&authority, &rollback_generation, &rollback_key)
            .await
            .unwrap();
        assert_eq!(restored.generation, DEMO_GENERATION);
        assert_eq!(restored.status, "active");
        let replayed_restored = repository
            .rollback_projection_generation(&authority, &rollback_generation, &rollback_key)
            .await
            .unwrap();
        assert_eq!(replayed_restored.generation, DEMO_GENERATION);

        let second = repository
            .ingest_explicit(
                &authority,
                IngestCommand {
                    idempotency_key: format!("rebuild-second-ingest-{test_id}"),
                    event_id: format!("rebuild-second-event-{test_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:rebuild:{test_id}:second"),
                    source_content: "请记住：代码示例优先 Rust".into(),
                    memory_content: "代码示例优先 Rust".into(),
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        let second_work = repository
            .claim_projection(&worker, &format!("rebuild-worker-{test_id}"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second_work.operation_id, second.operation_id);
        repository
            .complete_ready(&worker, &second_work, "provider-second")
            .await
            .unwrap();

        let final_generation = format!("rebuild-final-{test_id}");
        let final_build = repository
            .start_projection_rebuild(
                &authority,
                ProjectionRebuildCommand {
                    idempotency_key: format!("rebuild-start-final-{test_id}"),
                    generation: final_generation.clone(),
                    model_manifest: test_projection_model_manifest(),
                },
            )
            .await
            .unwrap();
        assert_eq!(final_build.expected_records, 2);
        for index in 0..2 {
            let work = repository
                .claim_projection(&worker, &format!("rebuild-worker-{test_id}"))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(work.generation, final_generation);
            repository
                .complete_ready(&worker, &work, &format!("provider-final-{index}"))
                .await
                .unwrap();
        }
        assert_eq!(
            repository
                .projection_generation(&authority, &final_generation)
                .await
                .unwrap()
                .status,
            "ready"
        );
        repository
            .activate_projection_generation(
                &authority,
                &final_generation,
                DEMO_GENERATION,
                &format!("rebuild-activate-final-{test_id}"),
            )
            .await
            .unwrap();
        let finalized = repository
            .finalize_projection_generation(
                &authority,
                &final_generation,
                &format!("rebuild-finalize-{test_id}"),
            )
            .await
            .unwrap();
        assert_eq!(finalized.status, "active");
        assert_eq!(
            repository
                .current_projection_generation(&authority)
                .await
                .unwrap(),
            final_generation
        );
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn context_pack_consume_is_atomic_bound_and_idempotent() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("consume-test-{test_id}"), "alice");

        let (replay_operation, manifest) =
            compile_test_pack(&repository, &authority, &format!("replay-{test_id}")).await;
        assert_eq!(manifest.item_count, 1);
        assert_eq!(manifest.delivery_status, "compiled");
        let consume = ConsumeContextPackCommand {
            idempotency_key: format!("consume-replay-{test_id}"),
            task_id: manifest.task_id,
            attempt_id: manifest.attempt_id,
            purpose: manifest.purpose.clone(),
        };
        let first = repository
            .consume_context_pack(&authority, manifest.context_pack_id, &consume)
            .await
            .unwrap();
        let repeated = repository
            .consume_context_pack(&authority, manifest.context_pack_id, &consume)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&repeated).unwrap()
        );
        assert_eq!(first.sections[0].items.len(), 1);

        let outcome = OutcomeReceiptCommand {
            idempotency_key: format!("outcome-replay-{test_id}"),
            task_id: manifest.task_id,
            attempt_id: manifest.attempt_id,
            purpose: manifest.purpose.clone(),
            outcome: "succeeded".into(),
            model_id: "k3".into(),
            model_request_id: Some(format!("provider-request-{test_id}")),
            latency_ms: 321,
            prompt_tokens: Some(10),
            completion_tokens: Some(20),
            total_tokens: Some(30),
            error_code: None,
        };
        let first_outcome = repository
            .record_outcome(&authority, manifest.context_pack_id, &outcome)
            .await
            .unwrap();
        let replayed_outcome = repository
            .record_outcome(&authority, manifest.context_pack_id, &outcome)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&first_outcome).unwrap(),
            serde_json::to_value(&replayed_outcome).unwrap()
        );
        assert_eq!(
            first_outcome.model_request_id_hash,
            Some(content_hash(&format!("provider-request-{test_id}")))
        );
        let mut changed_outcome = outcome.clone();
        changed_outcome.total_tokens = Some(31);
        assert!(matches!(
            repository
                .record_outcome(&authority, manifest.context_pack_id, &changed_outcome)
                .await,
            Err(RepositoryError::Conflict(_))
        ));
        let mut second_key = outcome.clone();
        second_key.idempotency_key = format!("outcome-second-key-{test_id}");
        assert!(matches!(
            repository
                .record_outcome(&authority, manifest.context_pack_id, &second_key)
                .await,
            Err(RepositoryError::Conflict(_))
        ));

        let (_, unconsumed_manifest) = compile_test_pack(
            &repository,
            &authority,
            &format!("unconsumed-outcome-{test_id}"),
        )
        .await;
        let unconsumed_outcome = OutcomeReceiptCommand {
            idempotency_key: format!("outcome-unconsumed-{test_id}"),
            task_id: unconsumed_manifest.task_id,
            attempt_id: unconsumed_manifest.attempt_id,
            purpose: unconsumed_manifest.purpose.clone(),
            outcome: "cancelled".into(),
            model_id: "k3".into(),
            model_request_id: None,
            latency_ms: 0,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            error_code: Some("CANCELLED_BEFORE_MODEL".into()),
        };
        assert!(matches!(
            repository
                .record_outcome(
                    &authority,
                    unconsumed_manifest.context_pack_id,
                    &unconsumed_outcome
                )
                .await,
            Err(RepositoryError::Conflict(_))
        ));

        let wrong_attempt = ConsumeContextPackCommand {
            idempotency_key: format!("consume-wrong-attempt-{test_id}"),
            task_id: manifest.task_id,
            attempt_id: Uuid::new_v4(),
            purpose: manifest.purpose.clone(),
        };
        assert!(matches!(
            repository
                .consume_context_pack(&authority, manifest.context_pack_id, &wrong_attempt)
                .await,
            Err(RepositoryError::Conflict(_))
        ));
        repository
            .forget(
                &authority,
                ForgetCommand {
                    idempotency_key: format!("forget-consumed-{test_id}"),
                    memory_record_id: replay_operation.memory_record_id,
                    expected_revision: replay_operation.head_revision,
                    reason: "integration consumed pack invalidation".into(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            repository
                .consume_context_pack(&authority, manifest.context_pack_id, &consume)
                .await,
            Err(RepositoryError::Conflict(_))
        ));

        let (forgotten_operation, forgotten_manifest) =
            compile_test_pack(&repository, &authority, &format!("forgotten-{test_id}")).await;
        repository
            .forget(
                &authority,
                ForgetCommand {
                    idempotency_key: format!("forget-before-consume-{test_id}"),
                    memory_record_id: forgotten_operation.memory_record_id,
                    expected_revision: forgotten_operation.head_revision,
                    reason: "integration test".into(),
                },
            )
            .await
            .unwrap();
        let forgotten_consume = ConsumeContextPackCommand {
            idempotency_key: format!("consume-forgotten-{test_id}"),
            task_id: forgotten_manifest.task_id,
            attempt_id: forgotten_manifest.attempt_id,
            purpose: forgotten_manifest.purpose,
        };
        assert!(matches!(
            repository
                .consume_context_pack(
                    &authority,
                    forgotten_manifest.context_pack_id,
                    &forgotten_consume
                )
                .await,
            Err(RepositoryError::Conflict(_))
        ));

        let (_, expired_manifest) =
            compile_test_pack(&repository, &authority, &format!("expired-{test_id}")).await;
        let mut expire_tx = repository.pool.begin().await.unwrap();
        set_tenant(&mut expire_tx, &authority.tenant_id)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE memory_core.context_packs \
             SET created_at=now()-interval '10 minutes',\
                 expires_at=now()-interval '5 minutes' \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(expired_manifest.context_pack_id)
        .execute(&mut *expire_tx)
        .await
        .unwrap();
        expire_tx.commit().await.unwrap();
        let expired_consume = ConsumeContextPackCommand {
            idempotency_key: format!("consume-expired-{test_id}"),
            task_id: expired_manifest.task_id,
            attempt_id: expired_manifest.attempt_id,
            purpose: expired_manifest.purpose,
        };
        assert!(matches!(
            repository
                .consume_context_pack(
                    &authority,
                    expired_manifest.context_pack_id,
                    &expired_consume
                )
                .await,
            Err(RepositoryError::Conflict(_))
        ));

        let (racing_operation, racing_manifest) =
            compile_test_pack(&repository, &authority, &format!("race-{test_id}")).await;
        let racing_consume = ConsumeContextPackCommand {
            idempotency_key: format!("consume-race-{test_id}"),
            task_id: racing_manifest.task_id,
            attempt_id: racing_manifest.attempt_id,
            purpose: racing_manifest.purpose,
        };
        let consume_repository = repository.clone();
        let forget_repository = repository.clone();
        let consume_authority = authority.clone();
        let forget_authority = authority.clone();
        let race_result = tokio::time::timeout(Duration::from_secs(5), async move {
            tokio::join!(
                consume_repository.consume_context_pack(
                    &consume_authority,
                    racing_manifest.context_pack_id,
                    &racing_consume
                ),
                forget_repository.forget(
                    &forget_authority,
                    ForgetCommand {
                        idempotency_key: format!("forget-race-{test_id}"),
                        memory_record_id: racing_operation.memory_record_id,
                        expected_revision: racing_operation.head_revision,
                        reason: "integration race test".into(),
                    }
                )
            )
        })
        .await
        .expect("consume/Forget race must not deadlock");
        assert!(race_result.1.is_ok());
        assert!(
            race_result.0.is_ok() || matches!(race_result.0, Err(RepositoryError::Conflict(_)))
        );
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn correction_appends_version_invalidates_pack_and_has_single_revision_winner() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("correction-test-{test_id}"), "alice");
        let (original, manifest) =
            compile_test_pack(&repository, &authority, &format!("correction-{test_id}")).await;
        let original_version_id = original.memory_version_id.unwrap();
        let correction = CorrectionCommand {
            idempotency_key: format!("correction-first-{test_id}"),
            memory_record_id: original.memory_record_id,
            expected_revision: original.head_revision,
            new_content: format!("回答风格：先给结论 {test_id}"),
            reason: "用户明确修正回答顺序".into(),
            source_ref: format!("conversation:correction:{test_id}:first"),
            effective_from: None,
        };

        let corrected = repository
            .correct(&authority, correction.clone())
            .await
            .unwrap();
        let replayed = repository
            .correct(&authority, correction.clone())
            .await
            .unwrap();
        assert_eq!(corrected.operation_id, replayed.operation_id);
        assert_eq!(corrected.memory_record_id, original.memory_record_id);
        assert_ne!(corrected.memory_version_id, Some(original_version_id));
        assert_eq!(corrected.head_revision, original.head_revision + 1);
        assert_eq!(corrected.canonical_status, CanonicalStatus::MemoryCorrected);

        let mut changed_replay = correction.clone();
        changed_replay.reason = "同一个幂等键但请求内容变化".into();
        assert!(matches!(
            repository.correct(&authority, changed_replay).await,
            Err(RepositoryError::Conflict(_))
        ));

        let stale_pack_consume = ConsumeContextPackCommand {
            idempotency_key: format!("correction-stale-pack-{test_id}"),
            task_id: manifest.task_id,
            attempt_id: manifest.attempt_id,
            purpose: manifest.purpose,
        };
        assert!(matches!(
            repository
                .consume_context_pack(&authority, manifest.context_pack_id, &stale_pack_consume)
                .await,
            Err(RepositoryError::Conflict(_))
        ));

        let left_repository = repository.clone();
        let right_repository = repository.clone();
        let left_authority = authority.clone();
        let right_authority = authority.clone();
        let expected_revision = corrected.head_revision;
        let (left, right) = tokio::join!(
            left_repository.correct(
                &left_authority,
                CorrectionCommand {
                    idempotency_key: format!("correction-race-left-{test_id}"),
                    memory_record_id: original.memory_record_id,
                    expected_revision,
                    new_content: format!("回答风格：先给结论再给证据 {test_id}"),
                    reason: "并发修正左侧".into(),
                    source_ref: format!("conversation:correction:{test_id}:left"),
                    effective_from: None,
                }
            ),
            right_repository.correct(
                &right_authority,
                CorrectionCommand {
                    idempotency_key: format!("correction-race-right-{test_id}"),
                    memory_record_id: original.memory_record_id,
                    expected_revision,
                    new_content: format!("回答风格：先给证据再给结论 {test_id}"),
                    reason: "并发修正右侧".into(),
                    source_ref: format!("conversation:correction:{test_id}:right"),
                    effective_from: None,
                }
            )
        );
        assert!(
            matches!(
                (&left, &right),
                (Ok(_), Err(RepositoryError::Conflict(_)))
                    | (Err(RepositoryError::Conflict(_)), Ok(_))
            ),
            "exactly one correction must win the expected Head revision; left={left:?}, right={right:?}"
        );

        let mut verify_tx = repository.pool.begin().await.unwrap();
        set_tenant(&mut verify_tx, &authority.tenant_id)
            .await
            .unwrap();
        let versions = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_core.memory_versions \
             WHERE tenant_id=$1 AND memory_record_id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(original.memory_record_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let corrected_events = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_core.memory_events \
             WHERE tenant_id=$1 AND memory_record_id=$2 AND event_type='memory_corrected'",
        )
        .bind(&authority.tenant_id)
        .bind(original.memory_record_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let invalidation_reason = sqlx::query_scalar::<_, Option<String>>(
            "SELECT invalidation_reason FROM memory_core.context_packs \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(manifest.context_pack_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        verify_tx.commit().await.unwrap();
        assert_eq!(versions, 3);
        assert_eq!(corrected_events, 2);
        assert_eq!(invalidation_reason.as_deref(), Some("MEMORY_CORRECTED"));
        let history = repository
            .inspect_memory(&authority, original.memory_record_id)
            .await
            .unwrap();
        assert_eq!(history.memory_record_id, original.memory_record_id);
        assert_eq!(history.scope.kind, "user");
        assert_eq!(history.scope.id.as_deref(), Some("alice"));
        assert_eq!(history.head_state, "active");
        assert_eq!(history.revision, 2);
        assert_eq!(
            history
                .versions
                .iter()
                .map(|version| version.version_no)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(
            history
                .versions
                .iter()
                .all(|version| !version.source_refs.is_empty())
        );
        assert_eq!(
            history
                .events
                .iter()
                .filter(|event| event.event_type == "memory_corrected")
                .count(),
            2
        );
        let other_user = DemoAuthority::user(authority.tenant_id.clone(), "bob");
        assert!(matches!(
            repository
                .inspect_memory(&other_user, original.memory_record_id)
                .await,
            Err(RepositoryError::NotFound)
        ));
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn semantic_conflict_is_fail_closed_until_review_accept_or_reject() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("review-test-{test_id}"), "alice");

        let accept_label = format!("accept-{test_id}");
        let (accept_original, accept_manifest, accept_conflict, accept_conflict_command) =
            create_test_conflict(&repository, &authority, &accept_label).await;
        let accept_candidate_id = accept_conflict.candidate_id.unwrap();
        assert_eq!(
            accept_conflict.canonical_status,
            CanonicalStatus::NeedsReview
        );
        assert_eq!(
            accept_conflict.memory_record_id,
            accept_original.memory_record_id
        );
        assert_eq!(
            accept_conflict.memory_version_id,
            accept_original.memory_version_id
        );
        assert_eq!(
            accept_conflict.head_revision,
            accept_original.head_revision + 1
        );
        assert_eq!(
            accept_conflict.reason_code.as_deref(),
            Some("SEMANTIC_CONFLICT_REQUIRES_REVIEW")
        );
        let replayed_conflict = repository
            .ingest_explicit(&authority, accept_conflict_command)
            .await
            .unwrap();
        assert_eq!(replayed_conflict.operation_id, accept_conflict.operation_id);
        let stale_pack_consume = ConsumeContextPackCommand {
            idempotency_key: format!("review-contested-pack-{test_id}"),
            task_id: accept_manifest.task_id,
            attempt_id: accept_manifest.attempt_id,
            purpose: accept_manifest.purpose.clone(),
        };
        assert!(matches!(
            repository
                .consume_context_pack(
                    &authority,
                    accept_manifest.context_pack_id,
                    &stale_pack_consume
                )
                .await,
            Err(RepositoryError::Conflict(_))
        ));
        let old_hit = ProjectionHit {
            provider_object_id: "provider-stale-contested".into(),
            memory_record_id: accept_original.memory_record_id,
            memory_version_id: accept_original.memory_version_id.unwrap(),
            head_revision: u64::try_from(accept_original.head_revision).unwrap(),
            generation: DEMO_GENERATION.into(),
            content_hash: content_hash(&format!("审核键{accept_label}：原值")),
            provider_score: 1.0,
        };
        assert!(
            repository
                .authorized_hits(&authority, &[old_hit])
                .await
                .unwrap()
                .is_empty()
        );

        let accept_decision = ReviewDecisionCommand {
            idempotency_key: format!("review-accept-decision-{test_id}"),
            candidate_id: accept_candidate_id,
            expected_candidate_revision: 0,
            outcome: ReviewOutcome::Accept,
            reason: "用户确认冲突新值".into(),
        };
        let accepted = repository
            .decide_candidate(&authority, accept_decision.clone())
            .await
            .unwrap();
        let accepted_replay = repository
            .decide_candidate(&authority, accept_decision.clone())
            .await
            .unwrap();
        assert_eq!(accepted.operation_id, accepted_replay.operation_id);
        assert_eq!(accepted.candidate_id, Some(accept_candidate_id));
        assert_eq!(accepted.canonical_status, CanonicalStatus::MemoryAccepted);
        assert_ne!(
            accepted.memory_version_id,
            accept_original.memory_version_id
        );
        assert_eq!(accepted.head_revision, accept_conflict.head_revision + 1);
        let original_work = repository
            .claim_projection(&authority, "review-order-worker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original_work.operation_id, accept_original.operation_id);
        repository
            .complete_stale(&authority, &original_work, "STALE_CANONICAL_HEAD")
            .await
            .unwrap();
        let contest_work = repository
            .claim_projection(&authority, "review-order-worker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(contest_work.operation_id, accept_conflict.operation_id);
        repository
            .complete_stale(&authority, &contest_work, "DELETE_NOT_CURRENT")
            .await
            .unwrap();
        let review_work = repository
            .claim_projection(&authority, "review-order-worker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(review_work.operation_id, accepted.operation_id);
        repository
            .complete_ready(&authority, &review_work, "provider-review-order-test")
            .await
            .unwrap();
        let mut changed_accept_replay = accept_decision;
        changed_accept_replay.reason = "同一个幂等键但不同审核请求".into();
        assert!(matches!(
            repository
                .decide_candidate(&authority, changed_accept_replay)
                .await,
            Err(RepositoryError::Conflict(_))
        ));

        let reject_label = format!("reject-{test_id}");
        let (reject_original, _, reject_conflict, _) =
            create_test_conflict(&repository, &authority, &reject_label).await;
        let reject_candidate_id = reject_conflict.candidate_id.unwrap();
        let rejected = repository
            .decide_candidate(
                &authority,
                ReviewDecisionCommand {
                    idempotency_key: format!("review-reject-decision-{test_id}"),
                    candidate_id: reject_candidate_id,
                    expected_candidate_revision: 0,
                    outcome: ReviewOutcome::Reject,
                    reason: "保留原值并拒绝冲突候选".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(rejected.canonical_status, CanonicalStatus::MemoryAccepted);
        assert_eq!(rejected.candidate_id, Some(reject_candidate_id));
        assert_eq!(
            rejected.reason_code.as_deref(),
            Some("REVIEW_REJECTED_CONFLICTING_VALUE")
        );
        assert_eq!(
            rejected.memory_version_id,
            reject_original.memory_version_id
        );
        assert_eq!(rejected.head_revision, reject_conflict.head_revision + 1);

        let race_label = format!("race-{test_id}");
        let (_, _, race_conflict, _) =
            create_test_conflict(&repository, &authority, &race_label).await;
        let race_candidate_id = race_conflict.candidate_id.unwrap();
        let left_repository = repository.clone();
        let right_repository = repository.clone();
        let left_authority = authority.clone();
        let right_authority = authority.clone();
        let (left, right) = tokio::join!(
            left_repository.decide_candidate(
                &left_authority,
                ReviewDecisionCommand {
                    idempotency_key: format!("review-race-left-{test_id}"),
                    candidate_id: race_candidate_id,
                    expected_candidate_revision: 0,
                    outcome: ReviewOutcome::Accept,
                    reason: "并发审核接受".into(),
                }
            ),
            right_repository.decide_candidate(
                &right_authority,
                ReviewDecisionCommand {
                    idempotency_key: format!("review-race-right-{test_id}"),
                    candidate_id: race_candidate_id,
                    expected_candidate_revision: 0,
                    outcome: ReviewOutcome::Reject,
                    reason: "并发审核拒绝".into(),
                }
            )
        );
        assert!(
            matches!(
                (&left, &right),
                (Ok(_), Err(RepositoryError::Conflict(_)))
                    | (Err(RepositoryError::Conflict(_)), Ok(_))
            ),
            "only one reviewer may decide candidate revision 0; left={left:?}, right={right:?}"
        );

        let mut verify_tx = repository.pool.begin().await.unwrap();
        set_tenant(&mut verify_tx, &authority.tenant_id)
            .await
            .unwrap();
        let accept_state = sqlx::query_as::<_, (String, i64)>(
            "SELECT state,revision FROM memory_core.memory_candidates \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(accept_candidate_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let reject_state = sqlx::query_as::<_, (String, i64)>(
            "SELECT state,revision FROM memory_core.memory_candidates \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(reject_candidate_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let accepted_version_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_core.memory_versions \
             WHERE tenant_id=$1 AND memory_record_id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(accept_original.memory_record_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let rejected_version_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_core.memory_versions \
             WHERE tenant_id=$1 AND memory_record_id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(reject_original.memory_record_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let accept_invalidation = sqlx::query_scalar::<_, Option<String>>(
            "SELECT invalidation_reason FROM memory_core.context_packs \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(accept_manifest.context_pack_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        verify_tx.commit().await.unwrap();
        assert_eq!(accept_state, ("accepted".into(), 1));
        assert_eq!(reject_state, ("rejected".into(), 1));
        assert_eq!(accepted_version_count, 2);
        assert_eq!(rejected_version_count, 1);
        assert_eq!(accept_invalidation.as_deref(), Some("MEMORY_CONTESTED"));
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn typed_scopes_intersect_authority_and_project_erasure_is_tracked() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let mut alice = DemoAuthority::user(format!("typed-scope-test-{test_id}"), "alice");
        alice.tenant_scope = true;
        alice.project_ids.insert("project-a".into());
        alice.agent_ids.insert("agent-a".into());

        let (tenant_memory, tenant_content) = create_rescoped_test_memory(
            &repository,
            &alice,
            &format!("tenant-{test_id}"),
            "tenant",
            None,
            "project_fact",
        )
        .await;
        let (project_memory, project_content) = create_rescoped_test_memory(
            &repository,
            &alice,
            &format!("project-{test_id}"),
            "project",
            Some("project-a"),
            "project_fact",
        )
        .await;
        let (agent_memory, agent_content) = create_rescoped_test_memory(
            &repository,
            &alice,
            &format!("agent-{test_id}"),
            "agent",
            Some("agent-a"),
            "episodic_summary",
        )
        .await;
        let expected_namespaces = [
            authorized_namespace_key(&alice, "tenant", None).unwrap(),
            authorized_namespace_key(&alice, "project", Some("project-a")).unwrap(),
            authorized_namespace_key(&alice, "agent", Some("agent-a")).unwrap(),
        ];
        let expected_operations = HashMap::from([
            (tenant_memory.operation_id, expected_namespaces[0].clone()),
            (project_memory.operation_id, expected_namespaces[1].clone()),
            (agent_memory.operation_id, expected_namespaces[2].clone()),
        ]);
        for _ in 0..3 {
            let work = repository
                .claim_projection(&alice, "typed-scope-worker")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                work.namespace_key,
                *expected_operations.get(&work.operation_id).unwrap()
            );
            repository
                .complete_ready(
                    &alice,
                    &work,
                    &format!("provider-{}", work.memory_record_id),
                )
                .await
                .unwrap();
        }
        assert_eq!(
            expected_namespaces.iter().collect::<HashSet<_>>().len(),
            expected_namespaces.len()
        );
        assert_eq!(
            repository
                .export_memories(&alice)
                .await
                .unwrap()
                .record_count,
            3
        );
        assert_eq!(
            repository
                .inspect_memory(&alice, tenant_memory.memory_record_id)
                .await
                .unwrap()
                .scope
                .kind,
            "tenant"
        );
        assert_eq!(
            repository
                .inspect_memory(&alice, project_memory.memory_record_id)
                .await
                .unwrap()
                .scope
                .id
                .as_deref(),
            Some("project-a")
        );
        assert_eq!(
            repository
                .inspect_memory(&alice, agent_memory.memory_record_id)
                .await
                .unwrap()
                .scope
                .id
                .as_deref(),
            Some("agent-a")
        );

        let outsider = DemoAuthority::user(alice.tenant_id.clone(), "bob");
        assert_eq!(
            repository
                .export_memories(&outsider)
                .await
                .unwrap()
                .record_count,
            0
        );
        for record_id in [
            tenant_memory.memory_record_id,
            project_memory.memory_record_id,
            agent_memory.memory_record_id,
        ] {
            assert!(matches!(
                repository.inspect_memory(&outsider, record_id).await,
                Err(RepositoryError::NotFound)
            ));
        }

        let mut project_member = DemoAuthority::user(alice.tenant_id.clone(), "bob");
        project_member.project_ids.insert("project-a".into());
        assert_eq!(
            repository
                .export_memories(&project_member)
                .await
                .unwrap()
                .record_count,
            1
        );
        assert!(
            repository
                .inspect_memory(&project_member, project_memory.memory_record_id)
                .await
                .is_ok()
        );
        assert!(
            repository
                .operation(&project_member, project_memory.operation_id)
                .await
                .is_ok()
        );
        let project_hit = ProjectionHit {
            provider_object_id: format!("provider-{}", project_memory.memory_record_id),
            memory_record_id: project_memory.memory_record_id,
            memory_version_id: project_memory.memory_version_id.unwrap(),
            head_revision: u64::try_from(project_memory.head_revision).unwrap(),
            generation: DEMO_GENERATION.into(),
            content_hash: content_hash(&project_content),
            provider_score: 1.0,
        };
        let authorized = repository
            .authorized_hits(&project_member, std::slice::from_ref(&project_hit))
            .await
            .unwrap();
        assert_eq!(authorized.len(), 1);
        assert_eq!(authorized[0].content, project_content);
        assert!(
            repository
                .authorized_hits(&outsider, std::slice::from_ref(&project_hit))
                .await
                .unwrap()
                .is_empty()
        );
        let local_project_hits = repository
            .search_local_projection(
                &project_member,
                "project",
                Some("project-a"),
                "project-a verified value",
                DEMO_GENERATION,
                5,
            )
            .await
            .unwrap();
        assert_eq!(
            local_project_hits.first().map(|hit| hit.memory_record_id),
            Some(project_memory.memory_record_id)
        );
        assert!(matches!(
            repository
                .search_local_projection(
                    &outsider,
                    "project",
                    Some("project-a"),
                    "project-a verified value",
                    DEMO_GENERATION,
                    5,
                )
                .await,
            Err(RepositoryError::NotFound)
        ));

        let pack_command = ContextPackCommand {
            task_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            purpose: "answer_user".into(),
            query: "project-a verified value".into(),
            limit: 5,
            memory_types: vec!["project_fact".into()],
            ranking_version: "transparent-lexical-v1".into(),
            projection_generation: DEMO_GENERATION.into(),
            token_budget: 512,
            tokenizer_id: DEMO_CONTEXT_TOKENIZER_ID.into(),
            memory_required: true,
            pinned_memory_version_ids: Vec::new(),
            temporal_window_hash: None,
        };
        let manifest = repository
            .compile_context_pack(
                &project_member,
                &pack_command,
                std::slice::from_ref(&project_hit),
            )
            .await
            .unwrap();
        let consume = ConsumeContextPackCommand {
            idempotency_key: format!("typed-scope-consume-{test_id}"),
            task_id: pack_command.task_id,
            attempt_id: pack_command.attempt_id,
            purpose: pack_command.purpose,
        };
        assert!(matches!(
            repository
                .consume_context_pack(&outsider, manifest.context_pack_id, &consume)
                .await,
            Err(RepositoryError::Conflict(_))
        ));

        let project_erasure = repository
            .create_erasure_request(
                &alice,
                ErasureCommand {
                    idempotency_key: format!("project-erasure-{test_id}"),
                    target_kind: ErasureTargetKind::Project,
                    target_id: "project-a".into(),
                    reason: "authorized project deletion".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(project_erasure.status, "processing");
        assert_eq!(project_erasure.record_count, 1);
        assert!(project_erasure.target_ref.starts_with("sha256:"));
        assert_ne!(project_erasure.target_ref, "project-a");
        assert!(matches!(
            repository
                .inspect_memory(&project_member, project_memory.memory_record_id)
                .await,
            Err(RepositoryError::NotFound)
        ));
        let erasure_work = repository
            .claim_projection(&alice, "typed-scope-worker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            erasure_work.operation_id,
            project_erasure.records[0].projection_operation_id.unwrap()
        );
        repository
            .complete_deleted(&alice, &erasure_work)
            .await
            .unwrap();
        assert_eq!(
            repository
                .erasure_request(&alice, project_erasure.erasure_request_id)
                .await
                .unwrap()
                .status,
            "complete"
        );
        assert!(matches!(
            repository
                .erasure_request(&project_member, project_erasure.erasure_request_id)
                .await,
            Err(RepositoryError::NotFound)
        ));
        assert_eq!(
            repository
                .export_memories(&alice)
                .await
                .unwrap()
                .record_count,
            2
        );
        assert_eq!(
            tenant_content,
            format!("typed scope tenant-{test_id}: verified value")
        );
        assert_eq!(
            agent_content,
            format!("typed scope agent-{test_id}: verified value")
        );
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn export_is_authorized_and_expiry_is_fail_closed_and_projection_safe() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("export-expiry-test-{test_id}"), "alice");
        let retention_label = format!("retention-{test_id}");
        let effective_label = format!("effective-{test_id}");
        let (retention, retention_manifest) =
            compile_test_pack(&repository, &authority, &retention_label).await;
        let retention_upsert = repository
            .claim_projection(&authority, "export-expiry-worker")
            .await
            .unwrap()
            .unwrap();
        repository
            .complete_ready(
                &authority,
                &retention_upsert,
                "provider-before-retention-expiry",
            )
            .await
            .unwrap();
        let (effective, _) = compile_test_pack(&repository, &authority, &effective_label).await;
        let effective_upsert = repository
            .claim_projection(&authority, "export-expiry-worker")
            .await
            .unwrap()
            .unwrap();
        repository
            .complete_ready(
                &authority,
                &effective_upsert,
                "provider-before-effective-expiry",
            )
            .await
            .unwrap();

        let before = repository.export_memories(&authority).await.unwrap();
        assert_eq!(before.format_version, "lazytt-memory-export-v1");
        assert_eq!(before.record_count, 2);
        assert_eq!(before.records.len(), 2);
        assert!(
            before
                .records
                .iter()
                .all(|record| record.head_state == "active")
        );
        let other_user = DemoAuthority::user(authority.tenant_id.clone(), "bob");
        assert_eq!(
            repository
                .export_memories(&other_user)
                .await
                .unwrap()
                .record_count,
            0
        );
        assert!(matches!(
            repository
                .operation(&other_user, retention.operation_id)
                .await,
            Err(RepositoryError::NotFound)
        ));

        let mut schedule_tx = repository.pool.begin().await.unwrap();
        set_tenant(&mut schedule_tx, &authority.tenant_id)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE memory_core.memory_versions \
             SET retain_until=transaction_timestamp()-interval '1 day' \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(retention.memory_version_id.unwrap())
        .execute(&mut *schedule_tx)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE memory_core.memory_versions \
             SET effective_from=transaction_timestamp()-interval '2 days',\
                 effective_to=transaction_timestamp()-interval '1 day' \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(effective.memory_version_id.unwrap())
        .execute(&mut *schedule_tx)
        .await
        .unwrap();
        schedule_tx.commit().await.unwrap();

        let expired = repository.expire_due(&authority, 10).await.unwrap();
        assert_eq!(expired.len(), 2);
        assert!(
            expired
                .iter()
                .all(|operation| operation.canonical_status == CanonicalStatus::Expired)
        );
        assert!(
            expired
                .iter()
                .all(|operation| operation.projection_status == ProjectionOperationStatus::Pending)
        );
        assert!(
            repository
                .expire_due(&authority, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            repository.expire_due(&authority, 0).await,
            Err(RepositoryError::Conflict(_))
        ));

        let retention_view = repository
            .inspect_memory(&authority, retention.memory_record_id)
            .await
            .unwrap();
        let effective_view = repository
            .inspect_memory(&authority, effective.memory_record_id)
            .await
            .unwrap();
        assert_eq!(retention_view.head_state, "expired");
        assert_eq!(effective_view.head_state, "expired");
        assert!(retention_view.identity_retired_at.is_some());
        assert!(effective_view.identity_retired_at.is_some());
        assert_eq!(
            retention_view.events.last().unwrap().reason_code,
            "RETENTION_EXPIRED"
        );
        assert_eq!(
            effective_view.events.last().unwrap().reason_code,
            "EFFECTIVE_PERIOD_ENDED"
        );
        assert!(
            retention_view
                .events
                .last()
                .unwrap()
                .payload
                .get("projection_operation_id")
                .is_some()
        );
        let consume = ConsumeContextPackCommand {
            idempotency_key: format!("consume-after-expiry-{test_id}"),
            task_id: retention_manifest.task_id,
            attempt_id: retention_manifest.attempt_id,
            purpose: retention_manifest.purpose,
        };
        assert!(matches!(
            repository
                .consume_context_pack(&authority, retention_manifest.context_pack_id, &consume)
                .await,
            Err(RepositoryError::Conflict(_))
        ));
        let retention_content = format!("先讲原理再讲代码 {retention_label}");
        assert!(
            repository
                .authorized_hits(
                    &authority,
                    &[ProjectionHit {
                        provider_object_id: "provider-before-retention-expiry".into(),
                        memory_record_id: retention.memory_record_id,
                        memory_version_id: retention.memory_version_id.unwrap(),
                        head_revision: u64::try_from(retention.head_revision).unwrap(),
                        generation: DEMO_GENERATION.into(),
                        content_hash: content_hash(&retention_content),
                        provider_score: 1.0,
                    }]
                )
                .await
                .unwrap()
                .is_empty()
        );

        let operation_ids = expired
            .iter()
            .map(|operation| operation.operation_id)
            .collect::<HashSet<_>>();
        for _ in 0..2 {
            let work = repository
                .claim_projection(&authority, "export-expiry-worker")
                .await
                .unwrap()
                .unwrap();
            assert!(operation_ids.contains(&work.operation_id));
            assert_eq!(work.current_head_state, "expired");
            repository
                .complete_deleted(&authority, &work)
                .await
                .unwrap();
        }
        for operation in expired {
            let completed = repository
                .operation(&authority, operation.operation_id)
                .await
                .unwrap();
            assert_eq!(completed.canonical_status, CanonicalStatus::Expired);
            assert_eq!(
                completed.projection_status,
                ProjectionOperationStatus::Deleted
            );
        }
        let after = repository.export_memories(&authority).await.unwrap();
        assert_eq!(after.record_count, 2);
        assert!(
            after
                .records
                .iter()
                .all(|record| record.head_state == "expired")
        );
        let mut verify_tx = repository.pool.begin().await.unwrap();
        set_tenant(&mut verify_tx, &authority.tenant_id)
            .await
            .unwrap();
        let local_projection_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_search.retrieval_documents \
             WHERE tenant_id=$1 AND memory_record_id=ANY($2)",
        )
        .bind(&authority.tenant_id)
        .bind([retention.memory_record_id, effective.memory_record_id])
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        verify_tx.commit().await.unwrap();
        assert_eq!(local_projection_count, 0);
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn erasure_scrubs_canonical_content_and_completes_after_projection_receipt() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("erasure-test-{test_id}"), "alice");
        let (original, manifest) =
            compile_test_pack(&repository, &authority, &format!("erasure-{test_id}")).await;
        let original_version_id = original.memory_version_id.unwrap();
        let original_work = repository
            .claim_projection(&authority, "erasure-test-worker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original_work.operation_id, original.operation_id);
        repository
            .complete_ready(&authority, &original_work, "provider-before-erasure")
            .await
            .unwrap();

        let command = ErasureCommand {
            idempotency_key: format!("erasure-record-request-{test_id}"),
            target_kind: ErasureTargetKind::MemoryRecord,
            target_id: original.memory_record_id.to_string(),
            reason: "用户要求物理擦除测试数据".into(),
        };
        let requested = repository
            .create_erasure_request(&authority, command.clone())
            .await
            .unwrap();
        let replayed = repository
            .create_erasure_request(&authority, command.clone())
            .await
            .unwrap();
        assert_eq!(requested.erasure_request_id, replayed.erasure_request_id);
        assert_eq!(requested.status, "processing");
        assert_eq!(requested.record_count, 1);
        assert_eq!(requested.records.len(), 1);
        assert_eq!(requested.records[0].status, "pending");
        assert_eq!(requested.target_ref, original.memory_record_id.to_string());
        let mut changed_replay = command;
        changed_replay.reason = "相同幂等键但不同擦除原因".into();
        assert!(matches!(
            repository
                .create_erasure_request(&authority, changed_replay)
                .await,
            Err(RepositoryError::Conflict(_))
        ));
        assert!(matches!(
            repository
                .inspect_memory(&authority, original.memory_record_id)
                .await,
            Err(RepositoryError::NotFound)
        ));
        assert!(
            repository
                .authorized_hits(
                    &authority,
                    &[ProjectionHit {
                        provider_object_id: "provider-before-erasure".into(),
                        memory_record_id: original.memory_record_id,
                        memory_version_id: original_version_id,
                        head_revision: u64::try_from(original.head_revision).unwrap(),
                        generation: DEMO_GENERATION.into(),
                        content_hash: content_hash(&format!("先讲原理再讲代码 erasure-{test_id}")),
                        provider_score: 1.0,
                    }]
                )
                .await
                .unwrap()
                .is_empty()
        );
        let consume = ConsumeContextPackCommand {
            idempotency_key: format!("erasure-pack-consume-{test_id}"),
            task_id: manifest.task_id,
            attempt_id: manifest.attempt_id,
            purpose: manifest.purpose,
        };
        assert!(matches!(
            repository
                .consume_context_pack(&authority, manifest.context_pack_id, &consume)
                .await,
            Err(RepositoryError::Conflict(_))
        ));

        let mut verify_tx = repository.pool.begin().await.unwrap();
        set_tenant(&mut verify_tx, &authority.tenant_id)
            .await
            .unwrap();
        let scrubbed = sqlx::query_as::<_, (String, Option<Uuid>, String, String)>(
            "SELECT h.state,h.current_version_id,r.semantic_key,r.scope_id \
             FROM memory_core.memory_heads h \
             JOIN memory_core.memory_records r \
               ON r.tenant_id=h.tenant_id AND r.id=h.memory_record_id \
             WHERE h.tenant_id=$1 AND h.memory_record_id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(original.memory_record_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let version_content_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_core.memory_versions \
             WHERE tenant_id=$1 AND memory_record_id=$2 \
               AND (content IS NOT NULL OR content_erased_at IS NULL \
                    OR content_hash NOT LIKE 'erased:%')",
        )
        .bind(&authority.tenant_id)
        .bind(original.memory_record_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let candidate_content_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_core.memory_candidates \
             WHERE tenant_id=$1 AND target_memory_record_id=$2 \
               AND (content IS NOT NULL OR content_erased_at IS NULL)",
        )
        .bind(&authority.tenant_id)
        .bind(original.memory_record_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let source_plaintext_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_core.source_events se \
             JOIN memory_core.memory_version_sources mvs \
               ON mvs.tenant_id=se.tenant_id AND mvs.source_event_id=se.id \
             JOIN memory_core.memory_versions v \
               ON v.tenant_id=mvs.tenant_id AND v.id=mvs.memory_version_id \
             WHERE v.tenant_id=$1 AND v.memory_record_id=$2 \
               AND (se.redacted_excerpt IS NOT NULL \
                    OR se.source_ref NOT LIKE 'erased:%')",
        )
        .bind(&authority.tenant_id)
        .bind(original.memory_record_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let pack_snapshot_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_core.context_pack_items \
             WHERE tenant_id=$1 AND memory_record_id=$2 \
               AND (content_snapshot IS NOT NULL OR source_refs<>'[]'::jsonb)",
        )
        .bind(&authority.tenant_id)
        .bind(original.memory_record_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let local_projection_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_search.retrieval_documents \
             WHERE tenant_id=$1 AND memory_record_id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(original.memory_record_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        verify_tx.commit().await.unwrap();
        assert_eq!(scrubbed.0, "erased");
        assert_eq!(scrubbed.1, None);
        assert!(scrubbed.2.starts_with("erased:"));
        assert_ne!(scrubbed.3, "alice");
        assert_eq!(version_content_count, 0);
        assert_eq!(candidate_content_count, 0);
        assert_eq!(source_plaintext_count, 0);
        assert_eq!(pack_snapshot_count, 0);
        assert_eq!(local_projection_count, 0);

        let erasure_work = repository
            .claim_projection(&authority, "erasure-test-worker")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            erasure_work.operation_id,
            requested.records[0].projection_operation_id.unwrap()
        );
        assert_eq!(erasure_work.current_head_state, "erased");
        repository
            .complete_deleted(&authority, &erasure_work)
            .await
            .unwrap();
        let completed = repository
            .erasure_request(&authority, requested.erasure_request_id)
            .await
            .unwrap();
        assert_eq!(completed.status, "complete");
        assert!(completed.completed_at.is_some());
        assert_eq!(completed.records[0].status, "complete");
        let operation = repository
            .operation(
                &authority,
                completed.records[0].projection_operation_id.unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(operation.canonical_status, CanonicalStatus::ErasureComplete);

        let other_user = DemoAuthority::user(authority.tenant_id.clone(), "bob");
        assert!(matches!(
            repository
                .erasure_request(&other_user, requested.erasure_request_id)
                .await,
            Err(RepositoryError::NotFound)
        ));

        for index in 0..2 {
            repository
                .ingest_explicit(
                    &authority,
                    IngestCommand {
                        idempotency_key: format!("erasure-user-ingest-{index}-{test_id}"),
                        event_id: format!("erasure-user-event-{index}-{test_id}"),
                        source_type: "conversation_message".into(),
                        source_ref: format!("conversation:erasure-user:{index}:{test_id}"),
                        source_content: format!("请记住：用户擦除键{index}-{test_id}：值"),
                        memory_content: format!("用户擦除键{index}-{test_id}：值"),
                        occurred_at: Utc::now(),
                    },
                )
                .await
                .unwrap();
        }
        let user_erasure = repository
            .create_erasure_request(
                &authority,
                ErasureCommand {
                    idempotency_key: format!("erasure-user-request-{test_id}"),
                    target_kind: ErasureTargetKind::User,
                    target_id: authority.user_id.clone(),
                    reason: "用户要求擦除全部记忆".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(user_erasure.record_count, 2);
        assert_ne!(user_erasure.target_ref, authority.user_id);
        assert!(user_erasure.target_ref.starts_with("sha256:"));
    }

    #[tokio::test]
    #[ignore = "requires a migrated local PostgreSQL configured by TEST_DATABASE_URL"]
    async fn exact_semantic_dedupe_reuses_record_and_attaches_sources_under_concurrency() {
        let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into()
        });
        let repository = MemoryRepository::connect(&database_url).await.unwrap();
        repository.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let authority = DemoAuthority::user(format!("dedupe-test-{test_id}"), "alice");
        let content = format!("回答风格：先讲原理 {test_id}");
        let first = repository
            .ingest_explicit(
                &authority,
                IngestCommand {
                    idempotency_key: format!("dedupe-first-{test_id}"),
                    event_id: format!("dedupe-event-first-{test_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:dedupe:{test_id}:first"),
                    source_content: format!("请记住：{content}"),
                    memory_content: content.clone(),
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        let second = repository
            .ingest_explicit(
                &authority,
                IngestCommand {
                    idempotency_key: format!("dedupe-second-{test_id}"),
                    event_id: format!("dedupe-event-second-{test_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:dedupe:{test_id}:second"),
                    source_content: format!("请记住：{content}"),
                    memory_content: content.clone(),
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        assert_eq!(first.operation_id, second.operation_id);
        assert_eq!(first.memory_record_id, second.memory_record_id);
        assert_eq!(first.memory_version_id, second.memory_version_id);

        let concurrent_content = format!("代码格式：使用 rustfmt {test_id}");
        let left_repository = repository.clone();
        let right_repository = repository.clone();
        let left_authority = authority.clone();
        let right_authority = authority.clone();
        let left_content = concurrent_content.clone();
        let right_content = concurrent_content;
        let (left, right) = tokio::join!(
            left_repository.ingest_explicit(
                &left_authority,
                IngestCommand {
                    idempotency_key: format!("dedupe-concurrent-left-{test_id}"),
                    event_id: format!("dedupe-concurrent-event-left-{test_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:dedupe:{test_id}:left"),
                    source_content: format!("请记住：{left_content}"),
                    memory_content: left_content,
                    occurred_at: Utc::now(),
                }
            ),
            right_repository.ingest_explicit(
                &right_authority,
                IngestCommand {
                    idempotency_key: format!("dedupe-concurrent-right-{test_id}"),
                    event_id: format!("dedupe-concurrent-event-right-{test_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:dedupe:{test_id}:right"),
                    source_content: format!("请记住：{right_content}"),
                    memory_content: right_content,
                    occurred_at: Utc::now(),
                }
            )
        );
        let left = left.unwrap();
        let right = right.unwrap();
        assert_eq!(left.operation_id, right.operation_id);
        assert_eq!(left.memory_record_id, right.memory_record_id);
        assert_eq!(left.memory_version_id, right.memory_version_id);

        let mut verify_tx = repository.pool.begin().await.unwrap();
        set_tenant(&mut verify_tx, &authority.tenant_id)
            .await
            .unwrap();
        let source_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_core.memory_version_sources \
             WHERE tenant_id=$1 AND memory_version_id=$2",
        )
        .bind(&authority.tenant_id)
        .bind(first.memory_version_id.unwrap())
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        let live_identity_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_core.memory_records \
             WHERE tenant_id=$1 AND identity_retired_at IS NULL",
        )
        .bind(&authority.tenant_id)
        .fetch_one(&mut *verify_tx)
        .await
        .unwrap();
        verify_tx.commit().await.unwrap();
        assert_eq!(source_count, 2);
        assert_eq!(live_identity_count, 2);
    }
}
