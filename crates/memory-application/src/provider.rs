use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionUpsert {
    pub operation_id: Uuid,
    /// Opaque, service-derived partition key. It must not be accepted from an end client.
    pub namespace_key: String,
    pub memory_record_id: Uuid,
    pub memory_version_id: Uuid,
    pub head_revision: u64,
    pub generation: String,
    pub content: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionDelete {
    pub operation_id: Uuid,
    pub namespace_key: String,
    pub provider_object_id: String,
    pub memory_record_id: Uuid,
    pub head_revision: u64,
    pub generation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSearch {
    pub namespace_key: String,
    pub query: String,
    pub generation: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionLookup {
    pub namespace_key: String,
    pub generation: String,
    pub operation_id: Option<Uuid>,
    pub memory_record_id: Option<Uuid>,
    pub memory_version_id: Option<Uuid>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionHit {
    pub provider_object_id: String,
    pub memory_record_id: Uuid,
    pub memory_version_id: Uuid,
    pub head_revision: u64,
    pub generation: String,
    pub content_hash: String,
    pub provider_score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionStatus {
    Ready,
    Deleted,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionReceipt {
    pub operation_id: Uuid,
    pub provider_object_id: Option<String>,
    pub status: ProjectionStatus,
}

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("provider request timed out; result is unknown")]
    UnknownResult,
    #[error("provider rejected the request with status {status}: {body}")]
    Rejected { status: u16, body: String },
    #[error("provider response violated the adapter contract: {0}")]
    InvalidResponse(String),
    #[error("provider transport failed: {0}")]
    Transport(String),
}

#[async_trait]
pub trait RetrievalIndex: Send + Sync {
    async fn upsert(&self, request: ProjectionUpsert) -> Result<ProjectionReceipt, IndexError>;
    async fn delete(&self, request: ProjectionDelete) -> Result<ProjectionReceipt, IndexError>;
    async fn search(&self, request: ProjectionSearch) -> Result<Vec<ProjectionHit>, IndexError>;
    async fn lookup(&self, request: ProjectionLookup) -> Result<Vec<ProjectionHit>, IndexError>;
    async fn health_check(&self) -> Result<(), IndexError>;
}
