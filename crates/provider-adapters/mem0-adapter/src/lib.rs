//! Adapter for the self-hosted Mem0 REST server.
//!
//! The canonical projection path always sends `infer=false`. Mem0's normal
//! inference path couples extraction and storage and therefore cannot be used
//! to accept canonical memory.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use memory_application::{
    IndexError, ProjectionDelete, ProjectionHit, ProjectionLookup, ProjectionReceipt,
    ProjectionSearch, ProjectionStatus, ProjectionUpsert, RetrievalIndex,
};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

const MAX_ERROR_BODY_BYTES: usize = 2_048;

#[derive(Debug, Clone)]
pub struct Mem0Config {
    pub base_url: String,
    pub api_key: Option<String>,
    pub timeout: Duration,
}

pub struct Mem0Adapter {
    base_url: String,
    client: reqwest::Client,
}

impl Mem0Adapter {
    pub fn new(config: Mem0Config) -> Result<Self, IndexError> {
        let base_url = config.base_url.trim_end_matches('/').to_owned();
        reqwest::Url::parse(&base_url)
            .map_err(|error| IndexError::Transport(format!("invalid base URL: {error}")))?;

        let mut headers = HeaderMap::new();
        if let Some(api_key) = config.api_key {
            let value = HeaderValue::from_str(&api_key)
                .map_err(|_| IndexError::Transport("invalid Mem0 API key header".into()))?;
            headers.insert("x-api-key", value);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(config.timeout)
            .build()
            .map_err(map_reqwest_error)?;

        Ok(Self { base_url, client })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }
}

#[derive(Debug, Serialize)]
struct Mem0Message<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct Mem0AddRequest<'a> {
    messages: [Mem0Message<'a>; 1],
    user_id: &'a str,
    metadata: HashMap<&'static str, Value>,
    infer: bool,
}

#[derive(Debug, Deserialize)]
struct Mem0AddResponse {
    #[serde(default)]
    results: Vec<Mem0AddResult>,
}

#[derive(Debug, Deserialize)]
struct Mem0AddResult {
    id: String,
}

#[derive(Debug, Serialize)]
struct Mem0SearchRequest<'a> {
    query: &'a str,
    filters: Value,
    top_k: usize,
}

#[derive(Debug, Deserialize)]
struct Mem0SearchResponse {
    #[serde(default)]
    results: Vec<Mem0SearchHit>,
}

#[derive(Debug, Deserialize)]
struct Mem0SearchHit {
    id: String,
    #[serde(default)]
    score: f64,
    #[serde(default)]
    metadata: Map<String, Value>,
}

#[async_trait]
impl RetrievalIndex for Mem0Adapter {
    async fn upsert(&self, request: ProjectionUpsert) -> Result<ProjectionReceipt, IndexError> {
        let mut metadata = HashMap::new();
        metadata.insert(
            "operation_id",
            Value::String(request.operation_id.to_string()),
        );
        metadata.insert(
            "memory_record_id",
            Value::String(request.memory_record_id.to_string()),
        );
        metadata.insert(
            "memory_version_id",
            Value::String(request.memory_version_id.to_string()),
        );
        metadata.insert("head_revision", Value::from(request.head_revision));
        metadata.insert("generation", Value::String(request.generation));
        metadata.insert("content_hash", Value::String(request.content_hash));

        let payload = Mem0AddRequest {
            messages: [Mem0Message {
                role: "user",
                content: &request.content,
            }],
            user_id: &request.namespace_key,
            metadata,
            // Authority before index: the content is already an accepted Memory Version.
            infer: false,
        };

        let response = self
            .client
            .post(self.endpoint("/memories"))
            .json(&payload)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let response = require_success(response).await?;
        let body: Mem0AddResponse = response
            .json()
            .await
            .map_err(|error| IndexError::InvalidResponse(error.to_string()))?;
        let provider_object_id = body
            .results
            .into_iter()
            .next()
            .map(|result| result.id)
            .ok_or_else(|| IndexError::InvalidResponse("missing results[0].id".into()))?;

        Ok(ProjectionReceipt {
            operation_id: request.operation_id,
            provider_object_id: Some(provider_object_id),
            status: ProjectionStatus::Ready,
        })
    }

    async fn delete(&self, request: ProjectionDelete) -> Result<ProjectionReceipt, IndexError> {
        let path = format!("/memories/{}", request.provider_object_id);
        let response = self
            .client
            .delete(self.endpoint(&path))
            .send()
            .await
            .map_err(map_reqwest_error)?;
        if response.status() != reqwest::StatusCode::NOT_FOUND {
            require_success(response).await?;
        }

        Ok(ProjectionReceipt {
            operation_id: request.operation_id,
            provider_object_id: Some(request.provider_object_id),
            status: ProjectionStatus::Deleted,
        })
    }

    async fn search(&self, request: ProjectionSearch) -> Result<Vec<ProjectionHit>, IndexError> {
        let payload = Mem0SearchRequest {
            query: &request.query,
            filters: serde_json::json!({
                "user_id": request.namespace_key,
                "generation": {"eq": request.generation}
            }),
            top_k: request.limit,
        };

        let response = self
            .client
            .post(self.endpoint("/search"))
            .json(&payload)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let response = require_success(response).await?;
        let body: Mem0SearchResponse = response
            .json()
            .await
            .map_err(|error| IndexError::InvalidResponse(error.to_string()))?;

        body.results.into_iter().map(parse_search_hit).collect()
    }

    async fn lookup(&self, request: ProjectionLookup) -> Result<Vec<ProjectionHit>, IndexError> {
        let mut filters = Map::new();
        filters.insert("user_id".into(), Value::String(request.namespace_key));
        filters.insert(
            "generation".into(),
            serde_json::json!({"eq": request.generation}),
        );
        if let Some(operation_id) = request.operation_id {
            filters.insert(
                "operation_id".into(),
                serde_json::json!({"eq": operation_id.to_string()}),
            );
        }
        if let Some(memory_record_id) = request.memory_record_id {
            filters.insert(
                "memory_record_id".into(),
                serde_json::json!({"eq": memory_record_id.to_string()}),
            );
        }
        if let Some(memory_version_id) = request.memory_version_id {
            filters.insert(
                "memory_version_id".into(),
                serde_json::json!({"eq": memory_version_id.to_string()}),
            );
        }

        let payload = Mem0SearchRequest {
            query: "projection reconciliation",
            filters: Value::Object(filters),
            top_k: request.limit,
        };
        let response = self
            .client
            .post(self.endpoint("/search"))
            .json(&payload)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let response = require_success(response).await?;
        let body: Mem0SearchResponse = response
            .json()
            .await
            .map_err(|error| IndexError::InvalidResponse(error.to_string()))?;

        body.results.into_iter().map(parse_search_hit).collect()
    }

    async fn health_check(&self) -> Result<(), IndexError> {
        let response = self
            .client
            .get(self.endpoint("/auth/setup-status"))
            .send()
            .await
            .map_err(map_reqwest_error)?;
        require_success(response).await?;
        Ok(())
    }
}

fn parse_search_hit(hit: Mem0SearchHit) -> Result<ProjectionHit, IndexError> {
    Ok(ProjectionHit {
        provider_object_id: hit.id,
        memory_record_id: metadata_uuid(&hit.metadata, "memory_record_id")?,
        memory_version_id: metadata_uuid(&hit.metadata, "memory_version_id")?,
        head_revision: metadata_u64(&hit.metadata, "head_revision")?,
        generation: metadata_string(&hit.metadata, "generation")?,
        content_hash: metadata_string(&hit.metadata, "content_hash")?,
        provider_score: hit.score,
    })
}

fn metadata_string(metadata: &Map<String, Value>, key: &str) -> Result<String, IndexError> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| IndexError::InvalidResponse(format!("missing metadata.{key}")))
}

fn metadata_uuid(metadata: &Map<String, Value>, key: &str) -> Result<Uuid, IndexError> {
    let raw = metadata_string(metadata, key)?;
    Uuid::parse_str(&raw)
        .map_err(|_| IndexError::InvalidResponse(format!("invalid metadata.{key}")))
}

fn metadata_u64(metadata: &Map<String, Value>, key: &str) -> Result<u64, IndexError> {
    metadata
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| IndexError::InvalidResponse(format!("missing metadata.{key}")))
}

fn map_reqwest_error(error: reqwest::Error) -> IndexError {
    if error.is_timeout() {
        IndexError::UnknownResult
    } else {
        IndexError::Transport(error.to_string())
    }
}

async fn require_success(response: reqwest::Response) -> Result<reqwest::Response, IndexError> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status().as_u16();
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unavailable")
        .chars()
        .take(128)
        .collect::<String>();
    let mut response = response;
    let mut bounded_body = Vec::new();
    while bounded_body.len() < MAX_ERROR_BODY_BYTES {
        let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| IndexError::Transport(error.to_string()))?
        else {
            break;
        };
        let remaining = MAX_ERROR_BODY_BYTES - bounded_body.len();
        bounded_body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    let code = serde_json::from_slice::<Value>(&bounded_body)
        .ok()
        .and_then(|value| {
            value
                .get("code")
                .or_else(|| value.pointer("/error/code"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "REMOTE_REJECTED".into());
    let body = format!("code={code}; request_id={request_id}");
    Err(IndexError::Rejected { status, body })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{self, Receiver};
    use std::thread;

    use super::*;

    fn mock_server(status: &str, response_body: &str) -> (String, Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let address = listener.local_addr().expect("mock address");
        let status = status.to_owned();
        let response_body = response_body.to_owned();
        let (sender, receiver) = mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let mut expected_len = None;

            loop {
                let read = stream.read(&mut buffer).expect("read request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);

                if expected_len.is_none()
                    && let Some(header_end) = find_bytes(&request, b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_len = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    expected_len = Some(header_end + 4 + content_len);
                }

                if expected_len.is_some_and(|length| request.len() >= length) {
                    break;
                }
            }

            sender
                .send(String::from_utf8(request).expect("HTTP request is UTF-8"))
                .expect("send captured request");
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            )
            .expect("write response");
        });

        (format!("http://{address}"), receiver)
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn adapter(base_url: String) -> Mem0Adapter {
        Mem0Adapter::new(Mem0Config {
            base_url,
            api_key: Some("test-api-key-not-secret".into()),
            timeout: Duration::from_secs(2),
        })
        .expect("adapter config")
    }

    #[tokio::test]
    async fn accepted_version_is_indexed_with_infer_false() {
        let provider_id = "provider-memory-1";
        let (base_url, captured) = mock_server(
            "200 OK",
            &format!(r#"{{"results":[{{"id":"{provider_id}"}}]}}"#),
        );
        let adapter = adapter(base_url);
        let operation_id = Uuid::new_v4();
        let record_id = Uuid::new_v4();
        let version_id = Uuid::new_v4();

        let receipt = adapter
            .upsert(ProjectionUpsert {
                operation_id,
                namespace_key: "opaque-tenant-scope-key".into(),
                memory_record_id: record_id,
                memory_version_id: version_id,
                head_revision: 7,
                generation: "generation-1".into(),
                content: "User prefers explanations before code.".into(),
                content_hash: "sha256:test".into(),
            })
            .await
            .expect("upsert succeeds");

        assert_eq!(receipt.operation_id, operation_id);
        assert_eq!(receipt.provider_object_id.as_deref(), Some(provider_id));
        assert_eq!(receipt.status, ProjectionStatus::Ready);

        let raw_request = captured.recv().expect("captured request");
        assert!(raw_request.starts_with("POST /memories HTTP/1.1"));
        assert!(raw_request.to_ascii_lowercase().contains("x-api-key:"));
        let body = raw_request.split("\r\n\r\n").nth(1).expect("request body");
        let json: Value = serde_json::from_str(body).expect("JSON body");
        assert_eq!(json["infer"], false);
        assert_eq!(json["user_id"], "opaque-tenant-scope-key");
        assert_eq!(json["metadata"]["memory_record_id"], record_id.to_string());
        assert_eq!(
            json["metadata"]["memory_version_id"],
            version_id.to_string()
        );
        assert_eq!(json["metadata"]["operation_id"], operation_id.to_string());
    }

    #[tokio::test]
    async fn search_returns_only_canonical_mapping_fields() {
        let record_id = Uuid::new_v4();
        let version_id = Uuid::new_v4();
        let response = serde_json::json!({
            "results": [{
                "id": "provider-memory-1",
                "memory": "untrusted provider copy",
                "score": 0.91,
                "metadata": {
                    "memory_record_id": record_id,
                    "memory_version_id": version_id,
                    "head_revision": 3,
                    "generation": "generation-1",
                    "content_hash": "sha256:test"
                }
            }]
        });
        let (base_url, captured) = mock_server("200 OK", &response.to_string());
        let adapter = adapter(base_url);

        let hits = adapter
            .search(ProjectionSearch {
                namespace_key: "opaque-tenant-scope-key".into(),
                query: "preferences".into(),
                generation: "generation-1".into(),
                limit: 5,
            })
            .await
            .expect("search succeeds");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory_record_id, record_id);
        assert_eq!(hits[0].memory_version_id, version_id);
        assert_eq!(hits[0].provider_score, 0.91);

        let raw_request = captured.recv().expect("captured request");
        assert!(raw_request.starts_with("POST /search HTTP/1.1"));
        let body = raw_request.split("\r\n\r\n").nth(1).expect("request body");
        let json: Value = serde_json::from_str(body).expect("JSON body");
        assert_eq!(json["filters"]["user_id"], "opaque-tenant-scope-key");
        assert_eq!(json["filters"]["generation"]["eq"], "generation-1");
        assert_eq!(json["top_k"], 5);
        assert!(json.get("user_id").is_none());
        assert!(json.get("limit").is_none());
    }

    #[tokio::test]
    async fn delete_uses_provider_object_id() {
        let (base_url, captured) = mock_server("204 No Content", "");
        let adapter = adapter(base_url);
        let operation_id = Uuid::new_v4();

        let receipt = adapter
            .delete(ProjectionDelete {
                operation_id,
                namespace_key: "opaque-tenant-scope-key".into(),
                provider_object_id: "provider-memory-7".into(),
                memory_record_id: Uuid::new_v4(),
                head_revision: 9,
                generation: "generation-1".into(),
            })
            .await
            .expect("delete succeeds");

        assert_eq!(receipt.status, ProjectionStatus::Deleted);
        let raw_request = captured.recv().expect("captured request");
        assert!(raw_request.starts_with("DELETE /memories/provider-memory-7 HTTP/1.1"));
    }

    #[tokio::test]
    async fn deleting_an_already_missing_projection_is_idempotent() {
        let (base_url, _captured) = mock_server("404 Not Found", r#"{"detail":"missing"}"#);
        let adapter = adapter(base_url);
        let receipt = adapter
            .delete(ProjectionDelete {
                operation_id: Uuid::new_v4(),
                namespace_key: "opaque-tenant-scope-key".into(),
                provider_object_id: "already-missing".into(),
                memory_record_id: Uuid::new_v4(),
                head_revision: 2,
                generation: "generation-1".into(),
            })
            .await
            .expect("delete of missing projection is converged");
        assert_eq!(receipt.status, ProjectionStatus::Deleted);
    }

    #[tokio::test]
    async fn rejected_provider_body_is_summarized_without_echoing_content() {
        let (base_url, _captured) = mock_server(
            "400 Bad Request",
            r#"{"code":"INVALID_REQUEST","detail":"sensitive memory content"}"#,
        );
        let error = adapter(base_url)
            .health_check()
            .await
            .expect_err("provider rejection must be returned");
        let IndexError::Rejected { status, body } = error else {
            panic!("expected rejected error");
        };
        assert_eq!(status, 400);
        assert!(body.contains("INVALID_REQUEST"));
        assert!(!body.contains("sensitive memory content"));
    }

    #[tokio::test]
    async fn mutation_timeout_is_reported_as_unknown_result() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stalled server");
        let address = listener.local_addr().expect("stalled server address");
        thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("accept stalled request");
            thread::sleep(Duration::from_millis(100));
        });
        let adapter = Mem0Adapter::new(Mem0Config {
            base_url: format!("http://{address}"),
            api_key: None,
            timeout: Duration::from_millis(20),
        })
        .expect("adapter config");

        let error = adapter
            .upsert(ProjectionUpsert {
                operation_id: Uuid::new_v4(),
                namespace_key: "opaque-timeout-namespace".into(),
                memory_record_id: Uuid::new_v4(),
                memory_version_id: Uuid::new_v4(),
                head_revision: 1,
                generation: "generation-1".into(),
                content: "synthetic timeout probe".into(),
                content_hash: "sha256:timeout".into(),
            })
            .await
            .expect_err("stalled mutation must time out");

        assert!(matches!(error, IndexError::UnknownResult));
    }

    #[tokio::test]
    async fn lookup_filters_by_stable_operation_and_canonical_ids() {
        let record_id = Uuid::new_v4();
        let version_id = Uuid::new_v4();
        let operation_id = Uuid::new_v4();
        let (base_url, captured) = mock_server("200 OK", r#"{"results":[]}"#);
        let adapter = adapter(base_url);

        let hits = adapter
            .lookup(ProjectionLookup {
                namespace_key: "opaque-tenant-scope-key".into(),
                generation: "generation-1".into(),
                operation_id: Some(operation_id),
                memory_record_id: Some(record_id),
                memory_version_id: Some(version_id),
                limit: 25,
            })
            .await
            .expect("lookup succeeds");
        assert!(hits.is_empty());

        let raw_request = captured.recv().expect("captured request");
        let body = raw_request.split("\r\n\r\n").nth(1).expect("request body");
        let json: Value = serde_json::from_str(body).expect("JSON body");
        assert_eq!(json["filters"]["user_id"], "opaque-tenant-scope-key");
        assert_eq!(json["filters"]["generation"]["eq"], "generation-1");
        assert_eq!(
            json["filters"]["operation_id"]["eq"],
            operation_id.to_string()
        );
        assert_eq!(
            json["filters"]["memory_record_id"]["eq"],
            record_id.to_string()
        );
        assert_eq!(
            json["filters"]["memory_version_id"]["eq"],
            version_id.to_string()
        );
        assert_eq!(json["top_k"], 25);
    }
}
