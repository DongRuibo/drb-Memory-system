use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use demo_agent::{
    CompletionCommand, DemoError, KimiClient, ModelAnswer, assemble_empty_prompt, assemble_prompt,
};
use memory_client::{
    ContextPackCommand, ContextPackResponse, CorrectionCommand, ForgetCommand,
    MemoryExtractionCommand, MemoryHttpClient, MemoryPort, OutcomeReceiptCommand,
    ReviewDecisionCommand, ScopeSelector, TaskContext,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::time::Instant;
use tracing::{error, warn};
use uuid::Uuid;

use crate::store::{ChatStore, CompletedTurn, TurnView};

const CONTEXT_TOKENIZER_ID: &str = "demo-conservative-char-v1";
const CONTEXT_TOKEN_BUDGET: i32 = 1_200;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MemoryAction {
    Correct {
        memory_record_id: Uuid,
        new_content: String,
    },
    Forget {
        memory_record_id: Uuid,
    },
}

#[derive(Clone)]
pub struct ChatRuntime {
    pub store: ChatStore,
    pub memory: Arc<MemoryHttpClient>,
    pub model: Arc<KimiClient>,
    pub user_id: String,
    pub project_id: String,
}

pub struct TurnWork {
    pub turn: TurnView,
    pub content: String,
    pub memory_action: Option<MemoryAction>,
}

impl ChatRuntime {
    pub fn spawn_turn(self: Arc<Self>, work: TurnWork) {
        tokio::spawn(async move {
            let turn_id = work.turn.id;
            if let Err(error) = self.run_turn(work).await {
                error!(%turn_id, %error, "interactive chat turn failed");
                if self.store.cancel_requested(turn_id).await.unwrap_or(false) {
                    let _ = self
                        .store
                        .finish_cancelled(turn_id, "Generation stopped by the user")
                        .await;
                } else {
                    let _ = self
                        .store
                        .fail_turn(turn_id, "TURN_FAILED", &error.to_string())
                        .await;
                }
            }
        });
    }

    async fn run_turn(&self, work: TurnWork) -> Result<(), RuntimeError> {
        let turn = work.turn;
        self.store.start_turn(turn.id).await?;
        if self.store.cancel_requested(turn.id).await? {
            self.store
                .finish_cancelled(turn.id, "Generation stopped before memory processing")
                .await?;
            return Ok(());
        }

        self.store
            .append_event(turn.id, "memory.processing", json!({}))
            .await?;
        let mut pinned_versions = Vec::new();
        let mut memory_effects = Vec::<Value>::new();
        match work.memory_action {
            Some(MemoryAction::Correct {
                memory_record_id,
                new_content,
            }) => {
                let record = self.memory.inspect_memory(memory_record_id).await?;
                let operation = self
                    .memory
                    .correct(&CorrectionCommand {
                        memory_record_id,
                        idempotency_key: format!("chat-correct-{}", turn.user_message_id),
                        expected_revision: record.revision,
                        new_content,
                        reason: "User corrected memory in interactive chat".into(),
                        source_ref: format!(
                            "conversation:{}:{}",
                            turn.session_id, turn.user_message_id
                        ),
                        effective_from: Some(Utc::now()),
                        new_structured_content: None,
                    })
                    .await?;
                if operation.canonical_status != "memory_corrected" {
                    return Err(RuntimeError::Contract(format!(
                        "correction returned canonical_status={}",
                        operation.canonical_status
                    )));
                }
                if let Some(version_id) = operation.memory_version_id {
                    pinned_versions.push(version_id);
                }
                memory_effects.push(json!({
                    "kind": "corrected",
                    "memory_record_id": memory_record_id,
                    "memory_version_id": operation.memory_version_id,
                    "revision": operation.head_revision,
                    "operation_id": operation.operation_id,
                    "projection_status": operation.projection_status,
                }));
            }
            Some(MemoryAction::Forget { memory_record_id }) => {
                let record = self.memory.inspect_memory(memory_record_id).await?;
                let operation = self
                    .memory
                    .forget(&ForgetCommand {
                        memory_record_id,
                        idempotency_key: format!("chat-forget-{}", turn.user_message_id),
                        expected_revision: record.revision,
                        reason: "User requested Forget in interactive chat".into(),
                        request_erasure: false,
                    })
                    .await?;
                if operation.canonical_status != "forgotten" {
                    return Err(RuntimeError::Contract(format!(
                        "Forget returned canonical_status={}",
                        operation.canonical_status
                    )));
                }
                memory_effects.push(json!({
                    "kind": "forgotten",
                    "memory_record_id": memory_record_id,
                    "revision": operation.head_revision,
                    "operation_id": operation.operation_id,
                    "projection_status": operation.projection_status,
                }));
            }
            None => {
                let existing_record_ids = self
                    .memory
                    .export_memories()
                    .await
                    .ok()
                    .map(|export| {
                        export
                            .records
                            .into_iter()
                            .map(|record| record.memory_record_id)
                            .collect::<HashSet<_>>()
                    })
                    .unwrap_or_default();
                match self
                    .memory
                    .extract_memories(&MemoryExtractionCommand {
                        idempotency_key: format!("chat-extract-{}", turn.user_message_id),
                        event_id: format!("chat-message-{}", turn.user_message_id),
                        source_type: "conversation_message".into(),
                        source_ref: format!(
                            "conversation:{}:{}",
                            turn.session_id, turn.user_message_id
                        ),
                        content: work.content.clone(),
                        occurred_at: Utc::now(),
                        timezone: "Asia/Shanghai".into(),
                        max_candidates: 8,
                        scope_selector: Some(ScopeSelector::user(&self.user_id)),
                        project_scope_selector: Some(ScopeSelector::project(&self.project_id)),
                    })
                    .await
                {
                    Ok(extraction) => {
                        let candidate_count = extraction.candidate_count;
                        let diagnostics = extraction.diagnostics;
                        let mut remembered_count = 0usize;
                        let mut source_merged_count = 0usize;
                        let mut corrected_count = 0usize;
                        for candidate in extraction.candidates {
                            let operation = candidate.operation;
                            if matches!(
                                operation.canonical_status.as_str(),
                                "memory_accepted" | "memory_corrected"
                            ) {
                                let source_was_merged =
                                    existing_record_ids.contains(&operation.memory_record_id);
                                if source_was_merged {
                                    source_merged_count += 1;
                                } else {
                                    remembered_count += 1;
                                }
                                if let Some(version_id) = operation.memory_version_id {
                                    pinned_versions.push(version_id);
                                }
                                memory_effects.push(json!({
                                    "kind": if source_was_merged {
                                        "source_merged"
                                    } else {
                                        "remembered"
                                    },
                                    "memory_record_id": operation.memory_record_id,
                                    "memory_version_id": operation.memory_version_id,
                                    "memory_type": candidate.proposal.memory_type,
                                    "content": candidate.proposal.content,
                                    "revision": operation.head_revision,
                                    "operation_id": operation.operation_id,
                                    "projection_status": operation.projection_status,
                                }));
                            } else if operation.canonical_status == "needs_review" {
                                let [candidate_id] = operation.candidate_ids.as_slice() else {
                                    return Err(RuntimeError::Contract(format!(
                                        "automatic conflict resolution requires exactly one candidate; received {}",
                                        operation.candidate_ids.len()
                                    )));
                                };
                                let resolved = self
                                    .memory
                                    .decide_candidate(&ReviewDecisionCommand {
                                        candidate_id: *candidate_id,
                                        idempotency_key: format!(
                                            "chat-auto-resolve-{}-{candidate_id}",
                                            turn.user_message_id
                                        ),
                                        expected_candidate_revision: 0,
                                        outcome: "accept".into(),
                                        reason: "Authorized durable memory was automatically superseded by a newer explicit statement".into(),
                                    })
                                    .await?;
                                if resolved.canonical_status != "memory_accepted" {
                                    return Err(RuntimeError::Contract(format!(
                                        "automatic conflict resolution returned canonical_status={}",
                                        resolved.canonical_status
                                    )));
                                }
                                corrected_count += 1;
                                if let Some(version_id) = resolved.memory_version_id {
                                    pinned_versions.push(version_id);
                                }
                                memory_effects.push(json!({
                                    "kind": "corrected",
                                    "memory_record_id": resolved.memory_record_id,
                                    "memory_version_id": resolved.memory_version_id,
                                    "memory_type": candidate.proposal.memory_type,
                                    "content": candidate.proposal.content,
                                    "revision": resolved.head_revision,
                                    "operation_id": resolved.operation_id,
                                    "projection_status": resolved.projection_status,
                                    "resolution": "automatic_supersede",
                                }));
                            }
                        }
                        let applied_count =
                            remembered_count + source_merged_count + corrected_count;
                        memory_effects.push(json!({
                            "kind": "extraction_summary",
                            "model_candidate_count": diagnostics.model_candidate_count,
                            "valid_model_candidate_count": diagnostics.valid_model_candidate_count,
                            "candidate_count": candidate_count,
                            "filtered_reason_counts": diagnostics.filtered_reason_counts,
                            "coverage_added_count": diagnostics.coverage_added_count,
                            "coverage_enriched_count": diagnostics.coverage_enriched_count,
                            "remembered_count": remembered_count,
                            "source_merged_count": source_merged_count,
                            "corrected_count": corrected_count,
                            "ignored_count": candidate_count.saturating_sub(applied_count),
                        }));
                    }
                    Err(error) => {
                        warn!(turn_id = %turn.id, %error, "memory extraction failed; answering without new memory");
                        self.store
                            .append_event(
                                turn.id,
                                "memory.warning",
                                json!({
                                    "code": "EXTRACTION_UNAVAILABLE",
                                    "detail": "这次回答仍会继续，但没有声称保存新记忆。",
                                }),
                            )
                            .await?;
                    }
                }
            }
        }
        self.store
            .append_event(
                turn.id,
                "memory.effects",
                json!({"effects": memory_effects}),
            )
            .await?;

        if self.store.cancel_requested(turn.id).await? {
            self.store
                .finish_cancelled(turn.id, "Generation stopped after memory processing")
                .await?;
            return Ok(());
        }

        let task_context = TaskContext {
            task_id: turn.task_id,
            attempt_id: turn.attempt_id,
            purpose: "answer_user".into(),
        };
        let reference_time = Utc::now();
        let timezone = "Asia/Shanghai";
        let (context_pack, prepared) = match self
            .memory
            .acquire_context_pack(
                &ContextPackCommand {
                    task_context: task_context.clone(),
                    query: work.content.clone(),
                    reference_time: Some(reference_time),
                    timezone: Some(timezone.into()),
                    scope_selectors: vec![
                        ScopeSelector::user(&self.user_id),
                        ScopeSelector::project(&self.project_id),
                    ],
                    memory_types: vec![
                        "preference".into(),
                        "profile_fact".into(),
                        "semantic_fact".into(),
                        "episodic_summary".into(),
                    ],
                    limit: 8,
                    token_budget: CONTEXT_TOKEN_BUDGET,
                    tokenizer_id: CONTEXT_TOKENIZER_ID.into(),
                    memory_required: false,
                    pinned_memory_version_ids: pinned_versions,
                },
                &format!("chat-consume-{}", turn.attempt_id),
            )
            .await
        {
            Ok(pack) => {
                let prompt = assemble_prompt(&work.content, &pack)?;
                (Some(pack), prompt)
            }
            Err(error) => {
                warn!(turn_id = %turn.id, %error, "context compilation failed; using empty memory context");
                self.store
                    .append_event(
                        turn.id,
                        "memory.warning",
                        json!({
                            "code": "CONTEXT_UNAVAILABLE",
                            "detail": "记忆上下文暂时不可用，本次按普通对话回答。",
                        }),
                    )
                    .await?;
                (
                    None,
                    assemble_empty_prompt(&work.content, reference_time, timezone)?,
                )
            }
        };
        let used_memory_ids = prepared.memory_ids.clone();
        let used_memory_version_ids = prepared.memory_version_ids.clone();
        let memory_effects_value = Value::Array(memory_effects);
        self.store
            .begin_streaming(
                turn.id,
                context_pack.as_ref().map(|pack| pack.context_pack_id),
                &prepared.context_snapshot,
                &used_memory_ids,
                &used_memory_version_ids,
                &memory_effects_value,
            )
            .await?;

        let model_started = Instant::now();
        let delta_store = self.store.clone();
        let delta_turn_id = turn.id;
        let answer = self
            .model
            .stream_to(
                &CompletionCommand {
                    task_id: turn.task_id,
                    system_prompt: prepared.system_prompt,
                    user_payload: prepared.user_payload,
                },
                move |delta| {
                    let store = delta_store.clone();
                    async move {
                        store
                            .append_answer_delta(delta_turn_id, &delta)
                            .await
                            .map_err(|error| DemoError::Config(error.to_string()))
                    }
                },
            )
            .await;

        let answer = match answer {
            Ok(answer) => answer,
            Err(_error) if self.store.cancel_requested(turn.id).await.unwrap_or(false) => {
                if let Some(pack) = &context_pack {
                    let _ = self
                        .record_outcome(
                            pack,
                            &task_context,
                            model_started.elapsed().as_millis(),
                            None,
                            "cancelled",
                            Some("TURN_CANCELLED"),
                        )
                        .await;
                }
                self.store
                    .finish_cancelled(turn.id, "Generation stopped by the user")
                    .await?;
                return Ok(());
            }
            Err(error) => {
                if let Some(pack) = &context_pack {
                    let _ = self
                        .record_outcome(
                            pack,
                            &task_context,
                            model_started.elapsed().as_millis(),
                            None,
                            "failed",
                            Some("MODEL_FAILED"),
                        )
                        .await;
                }
                return Err(error.into());
            }
        };

        let outcome_receipt_id = if let Some(pack) = &context_pack {
            match self
                .record_outcome(
                    pack,
                    &task_context,
                    model_started.elapsed().as_millis(),
                    Some(&answer),
                    "succeeded",
                    None,
                )
                .await
            {
                Ok(id) => Some(id),
                Err(error) => {
                    warn!(turn_id = %turn.id, %error, "answer completed but Outcome Receipt failed");
                    self.store
                        .append_event(
                            turn.id,
                            "evidence.warning",
                            json!({"code": "OUTCOME_RECEIPT_FAILED"}),
                        )
                        .await?;
                    None
                }
            }
        } else {
            None
        };
        let request_hash = answer.request_id.as_deref().map(hash_sensitive_id);
        self.store
            .complete_turn(
                turn.id,
                CompletedTurn {
                    context_pack_id: context_pack.as_ref().map(|pack| pack.context_pack_id),
                    outcome_receipt_id,
                    used_memory_ids: &used_memory_ids,
                    used_memory_version_ids: &used_memory_version_ids,
                    memory_effects: &memory_effects_value,
                    model_id: &answer.model,
                    model_request_id_hash: request_hash.as_deref(),
                    prompt_tokens: checked_tokens(answer.usage.prompt_tokens)?,
                    completion_tokens: checked_tokens(answer.usage.completion_tokens)?,
                    total_tokens: checked_tokens(answer.usage.total_tokens)?,
                },
            )
            .await?;
        Ok(())
    }

    async fn record_outcome(
        &self,
        pack: &ContextPackResponse,
        task_context: &TaskContext,
        latency_ms: u128,
        answer: Option<&ModelAnswer>,
        outcome: &str,
        error_code: Option<&str>,
    ) -> Result<Uuid, RuntimeError> {
        let latency_ms = i64::try_from(latency_ms)
            .map_err(|_| RuntimeError::Contract("model latency exceeded i64".into()))?;
        let receipt = self
            .memory
            .record_outcome(&OutcomeReceiptCommand {
                context_pack_id: pack.context_pack_id,
                task_context: task_context.clone(),
                idempotency_key: format!("chat-outcome-{}", task_context.attempt_id),
                outcome: outcome.into(),
                model_id: answer.map_or_else(|| "k3".into(), |answer| answer.model.clone()),
                model_request_id: answer.and_then(|answer| answer.request_id.clone()),
                latency_ms,
                prompt_tokens: answer
                    .and_then(|answer| checked_tokens(answer.usage.prompt_tokens).ok().flatten()),
                completion_tokens: answer.and_then(|answer| {
                    checked_tokens(answer.usage.completion_tokens)
                        .ok()
                        .flatten()
                }),
                total_tokens: answer
                    .and_then(|answer| checked_tokens(answer.usage.total_tokens).ok().flatten()),
                error_code: error_code.map(str::to_owned),
            })
            .await?;
        Ok(receipt.outcome_receipt_id)
    }
}

fn checked_tokens(tokens: Option<u64>) -> Result<Option<i64>, RuntimeError> {
    tokens
        .map(|value| {
            i64::try_from(value)
                .map_err(|_| RuntimeError::Contract("model token count exceeded i64".into()))
        })
        .transpose()
}

fn hash_sensitive_id(value: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(value.as_bytes())))
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
    #[error(transparent)]
    Memory(#[from] memory_client::MemoryClientError),
    #[error(transparent)]
    Demo(#[from] DemoError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("runtime contract violation: {0}")]
    Contract(String),
}
