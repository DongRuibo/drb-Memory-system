use async_trait::async_trait;
use chrono::{DateTime, Utc};
use memory_domain::{EventTime, StructuredContent};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractCandidateProposal {
    pub memory_type: String,
    pub semantic_key: String,
    pub content: String,
    pub confidence: f64,
    pub importance: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_time: Option<EventTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<StructuredContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractMemoryRequest {
    pub source_ref: String,
    pub content: String,
    pub source_event_time: DateTime<Utc>,
    pub timezone: String,
    pub max_candidates: usize,
    /// The service, not the model, decides which memory types may be proposed
    /// for this request. This keeps scope and policy authority outside the LLM.
    pub allowed_memory_types: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ExtractorError {
    #[error("extractor configuration is invalid: {0}")]
    Config(String),
    #[error("extractor request failed: {0}")]
    Transport(String),
    #[error("extractor response violated the contract: {0}")]
    InvalidResponse(String),
}

#[async_trait]
pub trait MemoryExtractor: Send + Sync {
    fn version(&self) -> &str;

    async fn extract(
        &self,
        request: ExtractMemoryRequest,
    ) -> Result<Vec<ExtractCandidateProposal>, ExtractorError>;
}
