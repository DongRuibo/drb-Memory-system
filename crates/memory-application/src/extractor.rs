use std::collections::BTreeMap;

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

/// Bounded, content-free audit data for one extractor invocation.
///
/// Counts are persisted with the proposal snapshot so idempotent replays can
/// explain the same extraction decision without retaining the raw model output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionDiagnostics {
    pub model_candidate_count: Option<usize>,
    pub valid_model_candidate_count: usize,
    #[serde(default)]
    pub filtered_reason_counts: BTreeMap<String, usize>,
    pub coverage_added_count: usize,
    pub coverage_enriched_count: usize,
}

impl ExtractionDiagnostics {
    pub fn legacy(valid_candidate_count: usize) -> Self {
        Self {
            model_candidate_count: None,
            valid_model_candidate_count: valid_candidate_count,
            filtered_reason_counts: BTreeMap::new(),
            coverage_added_count: 0,
            coverage_enriched_count: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractMemoryOutput {
    pub proposals: Vec<ExtractCandidateProposal>,
    pub diagnostics: ExtractionDiagnostics,
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
    ) -> Result<ExtractMemoryOutput, ExtractorError>;
}
