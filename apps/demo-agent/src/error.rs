use reqwest::StatusCode;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DemoError {
    #[error(transparent)]
    Memory(#[from] memory_client::MemoryClientError),
    #[error("invalid demo configuration: {0}")]
    Config(String),
    #[error("{service} HTTP client setup failed: {source}")]
    Client {
        service: &'static str,
        #[source]
        source: reqwest::Error,
    },
    #[error("{service} transport failed: {source}")]
    Transport {
        service: &'static str,
        #[source]
        source: reqwest::Error,
    },
    #[error(
        "{service} rejected the request with HTTP {status}; code={code}; request_id={request_id}"
    )]
    Rejected {
        service: &'static str,
        status: StatusCode,
        code: String,
        request_id: String,
    },
    #[error("{service} returned an invalid response: {detail}")]
    InvalidResponse {
        service: &'static str,
        detail: String,
    },
    #[error("projection operation {operation_id} did not reach {expected_status:?} before timeout")]
    ProjectionTimeout {
        operation_id: Uuid,
        expected_status: &'static str,
    },
    #[error(
        "projection operation {operation_id} failed while waiting for {expected_status:?}: {reason}"
    )]
    ProjectionFailed {
        operation_id: Uuid,
        expected_status: &'static str,
        reason: String,
    },
    #[error("the target memory {memory_record_id} was not included in the Context Pack")]
    TargetMemoryMissing { memory_record_id: Uuid },
    #[error("the forgotten memory {memory_record_id} was still included in a new Context Pack")]
    ForgottenMemoryVisible { memory_record_id: Uuid },
}
