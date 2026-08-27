//! Elasticsearch 8.11.3 retrieval projection adapter.
//!
//! PostgreSQL remains authoritative. Elasticsearch receives accepted Memory
//! Versions only and every hit must still be revalidated by the authority store.

use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use memory_application::{
    IndexError, ProjectionDelete, ProjectionHit, ProjectionLookup, ProjectionReceipt,
    ProjectionSearch, ProjectionStatus, ProjectionUpsert, RetrievalIndex,
};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 2_048;

#[derive(Debug, Clone)]
pub struct ElasticsearchConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub index_prefix: String,
    pub required_version: String,
    pub allow_insecure_index_http: bool,
    pub timeout: Duration,
    pub embedding_base_url: String,
    pub embedding_api_key: Option<String>,
    pub embedding_model: String,
    pub embedding_dimensions: i32,
    pub embedding_request_dialect: String,
    pub allow_insecure_embedding_http: bool,
    pub embedding_timeout: Duration,
}

pub struct ElasticsearchAdapter {
    base_url: String,
    index_prefix: String,
    required_version: String,
    embedding_base_url: String,
    embedding_model: String,
    embedding_dimensions: i32,
    include_input_type: bool,
    index_client: reqwest::Client,
    embedding_client: reqwest::Client,
}

impl ElasticsearchAdapter {
    pub fn new(config: ElasticsearchConfig) -> Result<Self, IndexError> {
        let base_url = normalize_base_url(
            &config.base_url,
            config.allow_insecure_index_http,
            "Elasticsearch",
        )?;
        let normalized_embedding_base_url = normalize_base_url(
            &config.embedding_base_url,
            config.allow_insecure_embedding_http,
            "embedding",
        )?;
        let embedding_base_url = normalized_embedding_base_url
            .strip_suffix("/v1")
            .unwrap_or(&normalized_embedding_base_url)
            .to_owned();
        validate_index_prefix(&config.index_prefix)?;
        if config.required_version.trim().is_empty() {
            return Err(IndexError::Transport(
                "Elasticsearch required version must not be blank".into(),
            ));
        }
        if config.embedding_model.trim().is_empty() {
            return Err(IndexError::Transport(
                "embedding model must not be blank".into(),
            ));
        }
        if !(1..=4096).contains(&config.embedding_dimensions) {
            return Err(IndexError::Transport(
                "embedding dimensions must be between 1 and 4096".into(),
            ));
        }
        let include_input_type = match config.embedding_request_dialect.as_str() {
            "openai" => false,
            "lazytt" => true,
            _ => {
                return Err(IndexError::Transport(
                    "embedding request dialect must be openai or lazytt".into(),
                ));
            }
        };

        let index_client = build_client(
            elasticsearch_headers(
                config.api_key.as_deref(),
                config.username.as_deref(),
                config.password.as_deref(),
            )?,
            config.timeout,
        )?;
        let embedding_client = build_client(
            bearer_headers(config.embedding_api_key.as_deref())?,
            config.embedding_timeout,
        )?;
        Ok(Self {
            base_url,
            index_prefix: config.index_prefix,
            required_version: config.required_version,
            embedding_base_url,
            embedding_model: config.embedding_model,
            embedding_dimensions: config.embedding_dimensions,
            include_input_type,
            index_client,
            embedding_client,
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn index_name(&self, generation: &str) -> String {
        generation_index_name(&self.index_prefix, generation)
    }

    async fn embed(&self, input: &str, input_type: &'static str) -> Result<Vec<f32>, IndexError> {
        let response = self
            .embedding_client
            .post(format!("{}/v1/embeddings", self.embedding_base_url))
            .json(&EmbeddingRequest {
                model: &self.embedding_model,
                input,
                dimensions: self.embedding_dimensions,
                input_type: self.include_input_type.then_some(input_type),
            })
            .send()
            .await
            .map_err(map_embedding_error)?;
        let response = require_success(response).await?;
        let bytes = bounded_bytes(response).await?;
        let body: EmbeddingResponse = serde_json::from_slice(&bytes)
            .map_err(|error| IndexError::InvalidResponse(error.to_string()))?;
        if body.model != self.embedding_model || body.data.len() != 1 {
            return Err(IndexError::InvalidResponse(
                "embedding response model or vector count did not match".into(),
            ));
        }
        let vector = body
            .data
            .into_iter()
            .next()
            .expect("length checked")
            .embedding;
        if vector.len() != self.embedding_dimensions as usize
            || vector.iter().any(|value| !value.is_finite())
        {
            return Err(IndexError::InvalidResponse(
                "embedding response dimensions or values did not match".into(),
            ));
        }
        Ok(vector)
    }

    async fn ensure_index(&self, generation: &str) -> Result<String, IndexError> {
        let index = self.index_name(generation);
        let response = self
            .index_client
            .head(self.endpoint(&format!("/{index}")))
            .send()
            .await
            .map_err(map_index_error)?;
        match response.status() {
            status if status.is_success() => {
                self.validate_mapping(&index).await?;
                return Ok(index);
            }
            StatusCode::NOT_FOUND => {}
            _ => {
                require_success(response).await?;
            }
        }
        let response = self
            .index_client
            .put(self.endpoint(&format!("/{index}")))
            .json(&json!({
                "settings": {"number_of_shards": 1, "number_of_replicas": 0},
                "mappings": {
                    "dynamic": "strict",
                    "properties": {
                        "operation_id": {"type": "keyword"},
                        "namespace_key": {"type": "keyword"},
                        "generation": {"type": "keyword"},
                        "memory_record_id": {"type": "keyword"},
                        "memory_version_id": {"type": "keyword"},
                        "head_revision": {"type": "long"},
                        "content_hash": {"type": "keyword"},
                        "content": {"type": "text"},
                        "embedding": {
                            "type": "dense_vector",
                            "dims": self.embedding_dimensions,
                            "index": true,
                            "similarity": "cosine"
                        }
                    }
                }
            }))
            .send()
            .await
            .map_err(map_index_error)?;
        if !response.status().is_success() {
            let status = response.status();
            let bytes = bounded_error_bytes(response).await?;
            let error_type = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/error/type")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                });
            if status != StatusCode::BAD_REQUEST
                || error_type.as_deref() != Some("resource_already_exists_exception")
            {
                return Err(rejected_from_bytes(status, &bytes));
            }
        }
        self.validate_mapping(&index).await?;
        Ok(index)
    }

    async fn validate_mapping(&self, index: &str) -> Result<(), IndexError> {
        let response = self
            .index_client
            .get(self.endpoint(&format!("/{index}/_mapping")))
            .send()
            .await
            .map_err(map_index_error)?;
        let response = require_success(response).await?;
        let bytes = bounded_bytes(response).await?;
        let body: Value = serde_json::from_slice(&bytes)
            .map_err(|error| IndexError::InvalidResponse(error.to_string()))?;
        let dimensions = body
            .pointer(&format!("/{index}/mappings/properties/embedding/dims"))
            .and_then(Value::as_i64);
        if dimensions != Some(i64::from(self.embedding_dimensions)) {
            return Err(IndexError::InvalidResponse(format!(
                "Elasticsearch index {index} uses incompatible embedding dimensions"
            )));
        }
        Ok(())
    }

    async fn search_index(
        &self,
        generation: &str,
        payload: Value,
    ) -> Result<Vec<ProjectionHit>, IndexError> {
        let index = self.index_name(generation);
        let response = self
            .index_client
            .post(self.endpoint(&format!("/{index}/_search")))
            .json(&payload)
            .send()
            .await
            .map_err(map_index_error)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        let response = require_success(response).await?;
        let bytes = bounded_bytes(response).await?;
        let body: ElasticsearchSearchResponse = serde_json::from_slice(&bytes)
            .map_err(|error| IndexError::InvalidResponse(error.to_string()))?;
        body.hits.hits.into_iter().map(parse_hit).collect()
    }
}

#[async_trait]
impl RetrievalIndex for ElasticsearchAdapter {
    async fn upsert(&self, request: ProjectionUpsert) -> Result<ProjectionReceipt, IndexError> {
        let vector = self.embed(&request.content, "document").await?;
        let index = self.ensure_index(&request.generation).await?;
        let provider_object_id = request.operation_id.to_string();
        let response = self
            .index_client
            .put(self.endpoint(&format!(
                "/{index}/_doc/{provider_object_id}?refresh=wait_for"
            )))
            .json(&ElasticsearchDocument {
                operation_id: request.operation_id,
                namespace_key: request.namespace_key,
                generation: request.generation,
                memory_record_id: request.memory_record_id,
                memory_version_id: request.memory_version_id,
                head_revision: request.head_revision,
                content_hash: request.content_hash,
                content: request.content,
                embedding: vector,
            })
            .send()
            .await
            .map_err(map_index_error)?;
        require_success(response).await?;
        Ok(ProjectionReceipt {
            operation_id: request.operation_id,
            provider_object_id: Some(provider_object_id),
            status: ProjectionStatus::Ready,
        })
    }

    async fn delete(&self, request: ProjectionDelete) -> Result<ProjectionReceipt, IndexError> {
        let index = self.index_name(&request.generation);
        let response = self
            .index_client
            .delete(self.endpoint(&format!(
                "/{index}/_doc/{}?refresh=wait_for",
                request.provider_object_id
            )))
            .send()
            .await
            .map_err(map_index_error)?;
        if response.status() != StatusCode::NOT_FOUND {
            require_success(response).await?;
        }
        Ok(ProjectionReceipt {
            operation_id: request.operation_id,
            provider_object_id: Some(request.provider_object_id),
            status: ProjectionStatus::Deleted,
        })
    }

    async fn search(&self, request: ProjectionSearch) -> Result<Vec<ProjectionHit>, IndexError> {
        if request.limit == 0 {
            return Ok(Vec::new());
        }
        let vector = self.embed(&request.query, "query").await?;
        let candidates = request.limit.saturating_mul(10).max(100).min(10_000);
        self.search_index(
            &request.generation,
            json!({
                "size": request.limit,
                "knn": {
                    "field": "embedding",
                    "query_vector": vector,
                    "k": request.limit,
                    "num_candidates": candidates,
                    "filter": {"bool": {"filter": [
                        {"term": {"namespace_key": request.namespace_key}},
                        {"term": {"generation": request.generation}}
                    ]}}
                }
            }),
        )
        .await
    }

    async fn lookup(&self, request: ProjectionLookup) -> Result<Vec<ProjectionHit>, IndexError> {
        if request.limit == 0 {
            return Ok(Vec::new());
        }
        let mut filters = vec![
            json!({"term": {"namespace_key": request.namespace_key}}),
            json!({"term": {"generation": request.generation}}),
        ];
        if let Some(operation_id) = request.operation_id {
            filters.push(json!({"term": {"operation_id": operation_id}}));
        }
        if let Some(memory_record_id) = request.memory_record_id {
            filters.push(json!({"term": {"memory_record_id": memory_record_id}}));
        }
        if let Some(memory_version_id) = request.memory_version_id {
            filters.push(json!({"term": {"memory_version_id": memory_version_id}}));
        }
        self.search_index(
            &request.generation,
            json!({
                "size": request.limit,
                "query": {"bool": {"filter": filters}}
            }),
        )
        .await
    }

    async fn health_check(&self) -> Result<(), IndexError> {
        let response = self
            .index_client
            .get(&self.base_url)
            .send()
            .await
            .map_err(map_index_error)?;
        let response = require_success(response).await?;
        let bytes = bounded_bytes(response).await?;
        let body: ElasticsearchRoot = serde_json::from_slice(&bytes)
            .map_err(|error| IndexError::InvalidResponse(error.to_string()))?;
        if body.version.number != self.required_version {
            return Err(IndexError::InvalidResponse(format!(
                "Elasticsearch version {} does not match required {}",
                body.version.number, self.required_version
            )));
        }
        let response = self
            .index_client
            .get(self.endpoint("/_cluster/health"))
            .send()
            .await
            .map_err(map_index_error)?;
        require_success(response).await?;
        Ok(())
    }
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a str,
    dimensions: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_type: Option<&'static str>,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    model: String,
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
struct ElasticsearchDocument {
    operation_id: Uuid,
    namespace_key: String,
    generation: String,
    memory_record_id: Uuid,
    memory_version_id: Uuid,
    head_revision: u64,
    content_hash: String,
    content: String,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct ElasticsearchSearchResponse {
    hits: ElasticsearchHits,
}

#[derive(Deserialize)]
struct ElasticsearchHits {
    hits: Vec<ElasticsearchHit>,
}

#[derive(Deserialize)]
struct ElasticsearchHit {
    #[serde(rename = "_id")]
    id: String,
    #[serde(rename = "_score", default)]
    score: Option<f64>,
    #[serde(rename = "_source")]
    source: ElasticsearchDocument,
}

#[derive(Deserialize)]
struct ElasticsearchRoot {
    version: ElasticsearchVersion,
}

#[derive(Deserialize)]
struct ElasticsearchVersion {
    number: String,
}

fn parse_hit(hit: ElasticsearchHit) -> Result<ProjectionHit, IndexError> {
    let source = hit.source;
    Ok(ProjectionHit {
        provider_object_id: hit.id,
        memory_record_id: source.memory_record_id,
        memory_version_id: source.memory_version_id,
        head_revision: source.head_revision,
        generation: source.generation,
        content_hash: source.content_hash,
        provider_score: hit.score.unwrap_or(0.0),
    })
}

fn generation_index_name(prefix: &str, generation: &str) -> String {
    let hash = hex::encode(Sha256::digest(generation.as_bytes()));
    format!("{prefix}-{}", &hash[..24])
}

fn validate_index_prefix(prefix: &str) -> Result<(), IndexError> {
    let valid = (1..=128).contains(&prefix.len())
        && !matches!(prefix.chars().next(), Some('_' | '-' | '+'))
        && prefix.chars().all(|value| {
            value.is_ascii_lowercase() || value.is_ascii_digit() || "-_".contains(value)
        });
    if !valid {
        return Err(IndexError::Transport(
            "Elasticsearch index prefix must use 1 to 128 lowercase letters, digits, '-' or '_'"
                .into(),
        ));
    }
    Ok(())
}

fn normalize_base_url(
    raw: &str,
    allow_insecure_http: bool,
    label: &str,
) -> Result<String, IndexError> {
    let url = Url::parse(raw.trim_end_matches('/'))
        .map_err(|error| IndexError::Transport(format!("invalid {label} base URL: {error}")))?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(IndexError::Transport(format!(
            "{label} base URL must not contain credentials, query parameters or fragments"
        )));
    }
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    if url.scheme() != "https" && !(url.scheme() == "http" && (loopback || allow_insecure_http)) {
        return Err(IndexError::Transport(format!(
            "{label} base URL must use HTTPS unless insecure container HTTP is explicitly allowed"
        )));
    }
    Ok(raw.trim_end_matches('/').to_owned())
}

fn elasticsearch_headers(
    api_key: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<HeaderMap, IndexError> {
    let mut headers = HeaderMap::new();
    if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
        let mut value = HeaderValue::from_str(&format!("ApiKey {}", api_key.trim()))
            .map_err(|_| IndexError::Transport("invalid Elasticsearch API key".into()))?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
        return Ok(headers);
    }

    match (
        username.filter(|value| !value.trim().is_empty()),
        password.filter(|value| !value.is_empty()),
    ) {
        (Some(username), Some(password)) => {
            let username = username.trim();
            if username.contains(':') {
                return Err(IndexError::Transport(
                    "Elasticsearch username must not contain ':'".into(),
                ));
            }
            let credentials = STANDARD.encode(format!("{username}:{password}"));
            let mut value = HeaderValue::from_str(&format!("Basic {credentials}"))
                .map_err(|_| IndexError::Transport("invalid Elasticsearch credentials".into()))?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        (None, None) => {}
        _ => {
            return Err(IndexError::Transport(
                "Elasticsearch username and password must be configured together".into(),
            ));
        }
    }
    Ok(headers)
}

fn bearer_headers(api_key: Option<&str>) -> Result<HeaderMap, IndexError> {
    let mut headers = HeaderMap::new();
    if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
        let mut value = HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
            .map_err(|_| IndexError::Transport("invalid embedding API key".into()))?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
    }
    Ok(headers)
}

fn build_client(headers: HeaderMap, timeout: Duration) -> Result<reqwest::Client, IndexError> {
    reqwest::Client::builder()
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(|error| IndexError::Transport(error.to_string()))
}

fn map_index_error(error: reqwest::Error) -> IndexError {
    if error.is_timeout() {
        IndexError::UnknownResult
    } else {
        IndexError::Transport(error.to_string())
    }
}

fn map_embedding_error(error: reqwest::Error) -> IndexError {
    IndexError::Transport(error.to_string())
}

async fn bounded_bytes(response: reqwest::Response) -> Result<Vec<u8>, IndexError> {
    let bytes = response
        .bytes()
        .await
        .map_err(|error| IndexError::Transport(error.to_string()))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(IndexError::InvalidResponse(
            "provider response exceeded 4 MiB".into(),
        ));
    }
    Ok(bytes.to_vec())
}

async fn bounded_error_bytes(mut response: reqwest::Response) -> Result<Vec<u8>, IndexError> {
    let mut body = Vec::new();
    while body.len() < MAX_ERROR_BODY_BYTES {
        let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| IndexError::Transport(error.to_string()))?
        else {
            break;
        };
        let remaining = MAX_ERROR_BODY_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    Ok(body)
}

async fn require_success(response: reqwest::Response) -> Result<reqwest::Response, IndexError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let bytes = bounded_error_bytes(response).await?;
    Err(rejected_from_bytes(status, &bytes))
}

fn rejected_from_bytes(status: StatusCode, bytes: &[u8]) -> IndexError {
    let value = serde_json::from_slice::<Value>(bytes).ok();
    let code = value
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/type")
                .or_else(|| value.get("code"))
                .and_then(Value::as_str)
        })
        .unwrap_or("REMOTE_REJECTED");
    let request_id = value
        .as_ref()
        .and_then(|value| value.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    IndexError::Rejected {
        status: status.as_u16(),
        body: format!("code={code}; request_id={request_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ElasticsearchConfig {
        ElasticsearchConfig {
            base_url: "http://127.0.0.1:9200".into(),
            api_key: None,
            username: None,
            password: None,
            index_prefix: "lazytt-memory".into(),
            required_version: "8.11.3".into(),
            allow_insecure_index_http: false,
            timeout: Duration::from_secs(1),
            embedding_base_url: "http://127.0.0.1:18082".into(),
            embedding_api_key: None,
            embedding_model: "test-embedding".into(),
            embedding_dimensions: 1024,
            embedding_request_dialect: "openai".into(),
            allow_insecure_embedding_http: false,
            embedding_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn generation_names_are_opaque_and_stable() {
        let first = generation_index_name("lazytt-memory", "Generation A/用户");
        let second = generation_index_name("lazytt-memory", "Generation A/用户");
        assert_eq!(first, second);
        assert_eq!(first.len(), "lazytt-memory-".len() + 24);
        assert!(
            first
                .chars()
                .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == '-')
        );
    }

    #[test]
    fn rejects_remote_plaintext_http() {
        let mut config = config();
        config.base_url = "http://elasticsearch.internal:9200".into();
        let error = ElasticsearchAdapter::new(config)
            .err()
            .expect("remote plaintext must fail");
        assert!(error.to_string().contains("must use HTTPS"));
    }

    #[test]
    fn permits_explicit_container_http() {
        let mut config = config();
        config.base_url = "http://elasticsearch:9200".into();
        config.allow_insecure_index_http = true;
        ElasticsearchAdapter::new(config).expect("explicit container HTTP is allowed");
    }

    #[test]
    fn api_key_takes_precedence_over_basic_auth() {
        let headers = elasticsearch_headers(Some("api-key"), Some("elastic"), Some("secret"))
            .expect("valid credentials");
        assert_eq!(headers[AUTHORIZATION], "ApiKey api-key");
        assert!(headers[AUTHORIZATION].is_sensitive());
    }

    #[test]
    fn builds_sensitive_basic_auth_header() {
        let headers = elasticsearch_headers(None, Some("elastic"), Some("secret"))
            .expect("valid credentials");
        assert_eq!(headers[AUTHORIZATION], "Basic ZWxhc3RpYzpzZWNyZXQ=");
        assert!(headers[AUTHORIZATION].is_sensitive());
    }

    #[test]
    fn rejects_incomplete_basic_auth() {
        let error = elasticsearch_headers(None, Some("elastic"), None)
            .expect_err("incomplete credentials must fail");
        assert!(error.to_string().contains("configured together"));
    }

    #[test]
    fn rejects_invalid_index_prefix() {
        let mut config = config();
        config.index_prefix = "LazyTT/Memory".into();
        let error = ElasticsearchAdapter::new(config)
            .err()
            .expect("invalid prefix must fail");
        assert!(error.to_string().contains("index prefix"));
    }
}
