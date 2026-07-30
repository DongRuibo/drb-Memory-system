use memory_client::OperationResponse;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct AnswerTurn {
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub context_pack_id: Uuid,
    pub outcome_receipt_id: Uuid,
    pub context_memory_ids: Vec<Uuid>,
    pub answer: String,
    pub model: String,
    pub request_id: Option<String>,
    pub usage: ModelUsage,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoReport {
    pub remember_accepted: OperationResponse,
    pub remember_ready: OperationResponse,
    pub with_memory: AnswerTurn,
    pub forget_accepted: OperationResponse,
    pub without_memory: AnswerTurn,
    pub delete_completed: OperationResponse,
}
