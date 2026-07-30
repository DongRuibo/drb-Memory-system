use std::time::Duration;

use chrono::Utc;
use memory_client::{
    ContextPackCommand, ContextPackResponse, ForgetCommand, MemoryExtractionCommand, MemoryPort,
    OperationResponse, OutcomeReceiptCommand, ScopeSelector, SourceEventCommand, TaskContext,
};
use serde_json::{Map, Value};
use tokio::time::{Instant, sleep};
use uuid::Uuid;

use crate::error::DemoError;
use crate::kimi_client::{ChatModel, CompletionCommand, ModelAnswer};
use crate::model::{AnswerTurn, DemoReport};
use crate::prompt::assemble_prompt;

const CONTEXT_TOKENIZER_ID: &str = "demo-conservative-char-v1";
const CONTEXT_TOKEN_BUDGET: i32 = 512;

pub struct DemoRunConfig {
    pub user_id: String,
    pub preference: String,
    pub question: String,
    pub automatic_memory: bool,
    pub projection_timeout: Duration,
}

pub struct DemoAgent<M, C> {
    memory: M,
    model: C,
    config: DemoRunConfig,
}

impl<M, C> DemoAgent<M, C>
where
    M: MemoryPort,
    C: ChatModel,
{
    pub fn new(memory: M, model: C, config: DemoRunConfig) -> Self {
        Self {
            memory,
            model,
            config,
        }
    }

    pub async fn run(&self) -> Result<DemoReport, DemoError> {
        self.memory.ready().await?;

        let run_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut metadata = Map::new();
        metadata.insert("demo".into(), Value::String("agent-v1".into()));
        metadata.insert("session_id".into(), Value::String(session_id.to_string()));
        let remember_accepted = if self.config.automatic_memory {
            let extraction = self
                .memory
                .extract_memories(&MemoryExtractionCommand {
                    idempotency_key: format!("agent-v1-extract-{run_id}"),
                    event_id: format!("agent-v1-event-{run_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:{session_id}:message-1"),
                    content: format!("这是我的稳定偏好：{}。以后请遵守。", self.config.preference),
                    occurred_at: Utc::now(),
                    timezone: "Asia/Shanghai".into(),
                    max_candidates: 1,
                    scope_selector: Some(ScopeSelector::user(&self.config.user_id)),
                    project_scope_selector: None,
                })
                .await?;
            if extraction.candidates.len() != 1 {
                return Err(DemoError::InvalidResponse {
                    service: "Memory Service",
                    detail: format!(
                        "automatic extraction returned {} candidates; expected exactly one",
                        extraction.candidates.len()
                    ),
                });
            }
            extraction.candidates.into_iter().next().unwrap().operation
        } else {
            self.memory
                .submit_event(&SourceEventCommand {
                    idempotency_key: format!("agent-v1-remember-{run_id}"),
                    event_id: format!("agent-v1-event-{run_id}"),
                    source_type: "conversation_message".into(),
                    source_ref: format!("conversation:{session_id}:message-1"),
                    content: format!("请记住：{}", self.config.preference),
                    occurred_at: Utc::now(),
                    scope_selector: Some(ScopeSelector::user(&self.config.user_id)),
                    metadata: Some(metadata),
                })
                .await?
        };
        if remember_accepted.canonical_status != "memory_accepted" {
            return Err(DemoError::InvalidResponse {
                service: "Memory Service",
                detail: format!(
                    "remember returned canonical_status={:?}",
                    remember_accepted.canonical_status
                ),
            });
        }
        let memory_record_id = remember_accepted.memory_record_id;
        let remember_ready = self
            .wait_for_projection(remember_accepted.operation_id, "ready")
            .await?;

        let with_task_id = Uuid::new_v4();
        let with_attempt_id = Uuid::new_v4();
        let with_pack = self
            .context_pack(with_task_id, with_attempt_id, &self.config.question, true)
            .await?;
        if !context_pack_contains(&with_pack, memory_record_id) {
            self.best_effort_forget(run_id, memory_record_id, remember_ready.head_revision)
                .await;
            return Err(DemoError::TargetMemoryMissing { memory_record_id });
        }
        let with_prompt = match assemble_prompt(&self.config.question, &with_pack) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.best_effort_forget(run_id, memory_record_id, remember_ready.head_revision)
                    .await;
                return Err(error);
            }
        };
        let with_model_started = Instant::now();
        let with_answer = match self
            .model
            .complete(&CompletionCommand {
                task_id: with_task_id,
                system_prompt: with_prompt.system_prompt,
                user_payload: with_prompt.user_payload,
            })
            .await
        {
            Ok(answer) => answer,
            Err(error) => {
                self.best_effort_forget(run_id, memory_record_id, remember_ready.head_revision)
                    .await;
                return Err(error);
            }
        };
        let with_outcome = match self
            .record_success_outcome(
                with_task_id,
                with_attempt_id,
                with_pack.context_pack_id,
                with_model_started.elapsed(),
                &with_answer,
            )
            .await
        {
            Ok(receipt_id) => receipt_id,
            Err(error) => {
                self.best_effort_forget(run_id, memory_record_id, remember_ready.head_revision)
                    .await;
                return Err(error);
            }
        };
        let with_memory = answer_turn(
            with_task_id,
            with_attempt_id,
            with_pack.context_pack_id,
            with_outcome,
            with_prompt.memory_ids,
            with_answer,
        );

        let forget_accepted = self
            .memory
            .forget(&ForgetCommand {
                memory_record_id,
                idempotency_key: format!("agent-v1-forget-{run_id}"),
                expected_revision: remember_ready.head_revision,
                reason: "Demo Agent v1 user requested forget".into(),
                request_erasure: false,
            })
            .await?;
        if forget_accepted.canonical_status != "forgotten" {
            return Err(DemoError::InvalidResponse {
                service: "Memory Service",
                detail: format!(
                    "forget returned canonical_status={:?}",
                    forget_accepted.canonical_status
                ),
            });
        }

        let without_task_id = Uuid::new_v4();
        let without_attempt_id = Uuid::new_v4();
        let without_pack = self
            .context_pack(
                without_task_id,
                without_attempt_id,
                &self.config.question,
                false,
            )
            .await?;
        if context_pack_contains(&without_pack, memory_record_id) {
            return Err(DemoError::ForgottenMemoryVisible { memory_record_id });
        }
        let without_prompt = assemble_prompt(&self.config.question, &without_pack)?;
        let without_model_started = Instant::now();
        let without_answer = self
            .model
            .complete(&CompletionCommand {
                task_id: without_task_id,
                system_prompt: without_prompt.system_prompt,
                user_payload: without_prompt.user_payload,
            })
            .await?;
        let without_outcome = self
            .record_success_outcome(
                without_task_id,
                without_attempt_id,
                without_pack.context_pack_id,
                without_model_started.elapsed(),
                &without_answer,
            )
            .await?;
        let without_memory = answer_turn(
            without_task_id,
            without_attempt_id,
            without_pack.context_pack_id,
            without_outcome,
            without_prompt.memory_ids,
            without_answer,
        );
        let delete_completed = self
            .wait_for_projection(forget_accepted.operation_id, "deleted")
            .await?;

        Ok(DemoReport {
            remember_accepted,
            remember_ready,
            with_memory,
            forget_accepted,
            without_memory,
            delete_completed,
        })
    }

    async fn context_pack(
        &self,
        task_id: Uuid,
        attempt_id: Uuid,
        query: &str,
        memory_required: bool,
    ) -> Result<ContextPackResponse, DemoError> {
        let command = ContextPackCommand {
            task_context: TaskContext {
                task_id,
                attempt_id,
                purpose: "answer_user".into(),
            },
            query: query.into(),
            reference_time: None,
            timezone: None,
            scope_selectors: vec![ScopeSelector::user(&self.config.user_id)],
            memory_types: vec!["preference".into()],
            limit: 5,
            token_budget: CONTEXT_TOKEN_BUDGET,
            tokenizer_id: CONTEXT_TOKENIZER_ID.into(),
            memory_required,
            pinned_memory_version_ids: Vec::new(),
        };
        Ok(self
            .memory
            .acquire_context_pack(&command, &format!("agent-v1-consume-{attempt_id}"))
            .await?)
    }

    async fn wait_for_projection(
        &self,
        operation_id: Uuid,
        expected_status: &'static str,
    ) -> Result<OperationResponse, DemoError> {
        let deadline = Instant::now() + self.config.projection_timeout;
        let mut delay = Duration::from_millis(100);
        loop {
            let operation = self.memory.operation(operation_id).await?;
            if operation.projection_status == expected_status {
                return Ok(operation);
            }
            if operation.projection_status == "failed" {
                return Err(DemoError::ProjectionFailed {
                    operation_id,
                    expected_status,
                    reason: operation
                        .reason_code
                        .unwrap_or_else(|| "reason unavailable".into()),
                });
            }
            if Instant::now() >= deadline {
                return Err(DemoError::ProjectionTimeout {
                    operation_id,
                    expected_status,
                });
            }
            sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(1));
        }
    }

    async fn record_success_outcome(
        &self,
        task_id: Uuid,
        attempt_id: Uuid,
        context_pack_id: Uuid,
        latency: Duration,
        answer: &ModelAnswer,
    ) -> Result<Uuid, DemoError> {
        let latency_ms =
            i64::try_from(latency.as_millis()).map_err(|_| DemoError::InvalidResponse {
                service: "Agent Runtime",
                detail: "model latency exceeded the Outcome Receipt range".into(),
            })?;
        let prompt_tokens = checked_tokens(answer.usage.prompt_tokens)?;
        let completion_tokens = checked_tokens(answer.usage.completion_tokens)?;
        let total_tokens = checked_tokens(answer.usage.total_tokens)?;
        let receipt = self
            .memory
            .record_outcome(&OutcomeReceiptCommand {
                context_pack_id,
                task_context: TaskContext {
                    task_id,
                    attempt_id,
                    purpose: "answer_user".into(),
                },
                idempotency_key: format!("agent-v1-outcome-{attempt_id}"),
                outcome: "succeeded".into(),
                model_id: answer.model.clone(),
                model_request_id: answer.request_id.clone(),
                latency_ms,
                prompt_tokens,
                completion_tokens,
                total_tokens,
                error_code: None,
            })
            .await?;
        if receipt.context_pack_id != context_pack_id
            || receipt.task_id != task_id
            || receipt.attempt_id != attempt_id
            || receipt.purpose != "answer_user"
            || receipt.outcome != "succeeded"
            || receipt.model_id != answer.model
            || receipt.latency_ms != latency_ms
            || receipt.prompt_tokens != prompt_tokens
            || receipt.completion_tokens != completion_tokens
            || receipt.total_tokens != total_tokens
            || receipt.error_code.is_some()
            || receipt.model_request_id_hash.is_some() != answer.request_id.is_some()
        {
            return Err(DemoError::InvalidResponse {
                service: "Memory Service",
                detail: format!(
                    "Outcome Receipt {} did not match the completed Attempt",
                    receipt.outcome_receipt_id
                ),
            });
        }
        Ok(receipt.outcome_receipt_id)
    }

    async fn best_effort_forget(
        &self,
        run_id: Uuid,
        memory_record_id: Uuid,
        expected_revision: i64,
    ) {
        let _ = self
            .memory
            .forget(&ForgetCommand {
                memory_record_id,
                idempotency_key: format!("agent-v1-forget-{run_id}"),
                expected_revision,
                reason: "Demo Agent v1 cleanup after failure".into(),
                request_erasure: false,
            })
            .await;
    }
}

fn answer_turn(
    task_id: Uuid,
    attempt_id: Uuid,
    context_pack_id: Uuid,
    outcome_receipt_id: Uuid,
    context_memory_ids: Vec<Uuid>,
    answer: ModelAnswer,
) -> AnswerTurn {
    AnswerTurn {
        task_id,
        attempt_id,
        context_pack_id,
        outcome_receipt_id,
        context_memory_ids,
        answer: answer.content,
        model: answer.model,
        request_id: answer.request_id,
        usage: answer.usage,
    }
}

fn checked_tokens(value: Option<u64>) -> Result<Option<i64>, DemoError> {
    value
        .map(|value| {
            i64::try_from(value).map_err(|_| DemoError::InvalidResponse {
                service: "Model",
                detail: "token usage exceeded the Outcome Receipt range".into(),
            })
        })
        .transpose()
}

fn context_pack_contains(pack: &ContextPackResponse, memory_record_id: Uuid) -> bool {
    pack.sections
        .iter()
        .flat_map(|section| &section.items)
        .any(|item| item.memory_record_id == memory_record_id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::Utc;

    use super::*;
    use crate::kimi_client::{CompletionCommand, ModelAnswer};
    use crate::model::ModelUsage;
    use memory_client::{
        ConsumeContextPackCommand, ContextPackItem, ContextPackManifestResponse, ContextSection,
        ExtractedCandidateProposal, ExtractedMemoryResult, ExtractionDiagnostics,
        MemoryClientError, MemoryExtractionResponse, OutcomeReceiptResponse,
    };

    struct FakeMemory {
        active: AtomicBool,
        memory_record_id: Uuid,
        memory_version_id: Uuid,
        remember_operation_id: Uuid,
        forget_operation_id: Uuid,
    }

    impl FakeMemory {
        fn new() -> Self {
            Self {
                active: AtomicBool::new(false),
                memory_record_id: Uuid::new_v4(),
                memory_version_id: Uuid::new_v4(),
                remember_operation_id: Uuid::new_v4(),
                forget_operation_id: Uuid::new_v4(),
            }
        }

        fn operation_response(
            &self,
            operation_id: Uuid,
            canonical_status: &str,
            projection_status: &str,
        ) -> OperationResponse {
            OperationResponse {
                operation_id,
                candidate_ids: vec![],
                memory_record_id: self.memory_record_id,
                memory_version_id: Some(self.memory_version_id),
                head_revision: i64::from(canonical_status == "forgotten"),
                canonical_status: canonical_status.into(),
                projection_status: projection_status.into(),
                reason_code: None,
                provider_object_id: Some("provider-object".into()),
                attempts: 1,
            }
        }

        fn context_time() -> chrono::DateTime<Utc> {
            chrono::DateTime::parse_from_rfc3339("2099-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        }
    }

    #[async_trait]
    impl MemoryPort for FakeMemory {
        async fn ready(&self) -> Result<(), MemoryClientError> {
            Ok(())
        }

        async fn submit_event(
            &self,
            _command: &SourceEventCommand,
        ) -> Result<OperationResponse, MemoryClientError> {
            self.active.store(true, Ordering::SeqCst);
            Ok(self.operation_response(self.remember_operation_id, "memory_accepted", "pending"))
        }

        async fn extract_memories(
            &self,
            _command: &MemoryExtractionCommand,
        ) -> Result<MemoryExtractionResponse, MemoryClientError> {
            self.active.store(true, Ordering::SeqCst);
            Ok(MemoryExtractionResponse {
                extractor_version: "test-extractor-v1".into(),
                candidate_count: 1,
                diagnostics: ExtractionDiagnostics {
                    model_candidate_count: Some(1),
                    valid_model_candidate_count: 1,
                    filtered_reason_counts: BTreeMap::new(),
                    coverage_added_count: 0,
                    coverage_enriched_count: 0,
                },
                candidates: vec![ExtractedMemoryResult {
                    proposal: ExtractedCandidateProposal {
                        memory_type: "preference".into(),
                        semantic_key: "user.answer.style".into(),
                        content: "用户偏好固定使用两个标题".into(),
                        confidence: 0.95,
                        importance: 0.8,
                        event_time: None,
                        structured_content: None,
                    },
                    operation: self.operation_response(
                        self.remember_operation_id,
                        "memory_accepted",
                        "pending",
                    ),
                }],
            })
        }

        async fn operation(
            &self,
            operation_id: Uuid,
        ) -> Result<OperationResponse, MemoryClientError> {
            if operation_id == self.remember_operation_id {
                Ok(self.operation_response(operation_id, "memory_accepted", "ready"))
            } else {
                Ok(self.operation_response(operation_id, "forgotten", "deleted"))
            }
        }

        async fn compile_context_pack(
            &self,
            command: &ContextPackCommand,
        ) -> Result<ContextPackManifestResponse, MemoryClientError> {
            let now = Self::context_time();
            Ok(ContextPackManifestResponse {
                context_pack_id: Uuid::new_v5(
                    &Uuid::NAMESPACE_OID,
                    command.task_context.attempt_id.as_bytes(),
                ),
                task_id: command.task_context.task_id,
                attempt_id: command.task_context.attempt_id,
                purpose: command.task_context.purpose.clone(),
                authority_snapshot_id: "authz-test".into(),
                policy_version: "demo-policy-v1".into(),
                compiler_version: "demo-context-compiler-v2".into(),
                ranking_version: "transparent-lexical-v1".into(),
                projection_generation: "demo-v0-generation-1".into(),
                reference_time: command.reference_time.unwrap_or(now),
                timezone: command
                    .timezone
                    .clone()
                    .unwrap_or_else(|| "Asia/Shanghai".into()),
                created_at: now,
                expires_at: now + chrono::Duration::minutes(5),
                tokenizer_id: CONTEXT_TOKENIZER_ID.into(),
                token_budget: CONTEXT_TOKEN_BUDGET,
                estimated_tokens: if self.active.load(Ordering::SeqCst) {
                    64
                } else {
                    0
                },
                item_count: i32::from(self.active.load(Ordering::SeqCst)),
                delivery_status: "compiled".into(),
                consumed_at: None,
                omission_summary: BTreeMap::new(),
            })
        }

        async fn consume_context_pack(
            &self,
            command: &ConsumeContextPackCommand,
        ) -> Result<ContextPackResponse, MemoryClientError> {
            let now = Self::context_time();
            let sections = if self.active.load(Ordering::SeqCst) {
                vec![ContextSection {
                    kind: "user_preference".into(),
                    items: vec![ContextPackItem {
                        memory_record_id: self.memory_record_id,
                        memory_version_id: self.memory_version_id,
                        text: "固定使用【原理】和【示例】两个标题".into(),
                        source_refs: vec!["conversation:test:message-1".into()],
                        effective_from: Some(now),
                        event_time: None,
                        inclusion_reason: "EXPLICIT_PREFERENCE_AND_TASK_RELEVANT".into(),
                        trust: "untrusted_memory_data".into(),
                    }],
                }]
            } else {
                vec![]
            };
            Ok(ContextPackResponse {
                context_pack_id: command.context_pack_id,
                task_id: command.task_context.task_id,
                attempt_id: command.task_context.attempt_id,
                purpose: command.task_context.purpose.clone(),
                authority_snapshot_id: "authz-test".into(),
                policy_version: "demo-policy-v1".into(),
                compiler_version: "demo-context-compiler-v2".into(),
                ranking_version: "transparent-lexical-v1".into(),
                projection_generation: "demo-v0-generation-1".into(),
                reference_time: now,
                timezone: "Asia/Shanghai".into(),
                created_at: now,
                expires_at: now + chrono::Duration::minutes(5),
                consumed_at: now,
                tokenizer_id: CONTEXT_TOKENIZER_ID.into(),
                token_budget: CONTEXT_TOKEN_BUDGET,
                estimated_tokens: if self.active.load(Ordering::SeqCst) {
                    64
                } else {
                    0
                },
                sections,
                omission_summary: BTreeMap::new(),
            })
        }

        async fn forget(
            &self,
            _command: &ForgetCommand,
        ) -> Result<OperationResponse, MemoryClientError> {
            self.active.store(false, Ordering::SeqCst);
            Ok(self.operation_response(self.forget_operation_id, "forgotten", "pending"))
        }

        async fn record_outcome(
            &self,
            command: &OutcomeReceiptCommand,
        ) -> Result<OutcomeReceiptResponse, MemoryClientError> {
            Ok(OutcomeReceiptResponse {
                outcome_receipt_id: Uuid::new_v5(
                    &Uuid::NAMESPACE_OID,
                    command.context_pack_id.as_bytes(),
                ),
                context_pack_id: command.context_pack_id,
                task_id: command.task_context.task_id,
                attempt_id: command.task_context.attempt_id,
                purpose: command.task_context.purpose.clone(),
                outcome: command.outcome.clone(),
                model_id: command.model_id.clone(),
                model_request_id_hash: command
                    .model_request_id
                    .as_ref()
                    .map(|_| format!("sha256:{}", "0".repeat(64))),
                latency_ms: command.latency_ms,
                prompt_tokens: command.prompt_tokens,
                completion_tokens: command.completion_tokens,
                total_tokens: command.total_tokens,
                error_code: command.error_code.clone(),
                recorded_at: Utc::now(),
            })
        }
    }

    struct FakeModel {
        calls: Arc<Mutex<Vec<CompletionCommand>>>,
    }

    #[async_trait]
    impl ChatModel for FakeModel {
        async fn complete(&self, command: &CompletionCommand) -> Result<ModelAnswer, DemoError> {
            self.calls.lock().unwrap().push(command.clone());
            let value: serde_json::Value = serde_json::from_str(&command.user_payload).unwrap();
            let has_memory = value["memory_context"]
                .as_array()
                .is_some_and(|items| !items.is_empty());
            Ok(ModelAnswer {
                content: if has_memory {
                    "answer-with-memory".into()
                } else {
                    "answer-without-memory".into()
                },
                model: "k3".into(),
                request_id: Some("test-request".into()),
                usage: ModelUsage {
                    prompt_tokens: Some(10),
                    completion_tokens: Some(20),
                    total_tokens: Some(30),
                },
            })
        }
    }

    #[tokio::test]
    async fn full_agent_flow_uses_memory_then_forgets_it() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let agent = DemoAgent::new(
            FakeMemory::new(),
            FakeModel {
                calls: Arc::clone(&calls),
            },
            DemoRunConfig {
                user_id: "alice".into(),
                preference: "固定使用两个标题".into(),
                question: "解释 Outbox".into(),
                automatic_memory: false,
                projection_timeout: Duration::from_secs(1),
            },
        );

        let report = agent.run().await.unwrap();
        assert_eq!(report.with_memory.context_memory_ids.len(), 1);
        assert!(report.without_memory.context_memory_ids.is_empty());
        assert_ne!(report.with_memory.task_id, report.without_memory.task_id);
        assert_ne!(
            report.with_memory.attempt_id,
            report.without_memory.attempt_id
        );
        assert_ne!(
            report.with_memory.context_pack_id,
            report.without_memory.context_pack_id
        );
        assert_ne!(
            report.with_memory.outcome_receipt_id,
            report.without_memory.outcome_receipt_id
        );
        assert_eq!(report.with_memory.answer, "answer-with-memory");
        assert_eq!(report.without_memory.answer, "answer-without-memory");

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let first: serde_json::Value = serde_json::from_str(&calls[0].user_payload).unwrap();
        let second: serde_json::Value = serde_json::from_str(&calls[1].user_payload).unwrap();
        assert_eq!(first["memory_context"].as_array().unwrap().len(), 1);
        assert!(second["memory_context"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn automatic_extraction_uses_the_same_authority_chain() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let agent = DemoAgent::new(
            FakeMemory::new(),
            FakeModel {
                calls: Arc::clone(&calls),
            },
            DemoRunConfig {
                user_id: "alice".into(),
                preference: "固定使用两个标题".into(),
                question: "解释 Outbox".into(),
                automatic_memory: true,
                projection_timeout: Duration::from_secs(1),
            },
        );

        let report = agent.run().await.unwrap();
        assert_eq!(report.remember_accepted.canonical_status, "memory_accepted");
        assert_eq!(report.with_memory.context_memory_ids.len(), 1);
        assert!(report.without_memory.context_memory_ids.is_empty());
        assert_eq!(calls.lock().unwrap().len(), 2);
    }
}
