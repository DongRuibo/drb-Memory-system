use std::env;
use std::time::Duration;

use memory_postgres::ProjectionModelManifest;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum EmbeddingProbeError {
    #[error("embedding configuration is invalid: {0}")]
    Config(String),
    #[error("embedding endpoint is unavailable: {0}")]
    Transport(String),
    #[error("embedding endpoint violated the projection manifest: {0}")]
    InvalidResponse(String),
}

pub struct EmbeddingProbe {
    base_url: String,
    manifest: ProjectionModelManifest,
    include_input_type: bool,
    client: reqwest::Client,
}

impl EmbeddingProbe {
    pub fn from_env(manifest: ProjectionModelManifest) -> Result<Self, EmbeddingProbeError> {
        manifest
            .validate_integrity()
            .map_err(EmbeddingProbeError::Config)?;
        let default_base_url = if manifest.semantic_embedding {
            "http://127.0.0.1:18082"
        } else {
            "http://127.0.0.1:18080"
        };
        let allow_local_container_http = env::var("MEMORY_ALLOW_INSECURE_MODEL_HTTP")
            .is_ok_and(|value| value.eq_ignore_ascii_case("true"));
        let base_url = normalize_base_url(
            &env::var("MEMORY_EMBEDDING_BASE_URL").unwrap_or_else(|_| default_base_url.into()),
            allow_local_container_http,
        )?;
        let include_input_type = match env::var("MEMORY_EMBEDDING_REQUEST_DIALECT")
            .unwrap_or_else(|_| "lazytt".into())
            .as_str()
        {
            "lazytt" => true,
            "openai" => false,
            _ => {
                return Err(EmbeddingProbeError::Config(
                    "MEMORY_EMBEDDING_REQUEST_DIALECT must be lazytt or openai".into(),
                ));
            }
        };
        let mut headers = HeaderMap::new();
        if let Some(api_key) = embedding_api_key_from_env(&manifest.embedding_provider) {
            let mut authorization = HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
                .map_err(|_| {
                    EmbeddingProbeError::Config(
                        "MEMORY_EMBEDDING_API_KEY is not a valid header value".into(),
                    )
                })?;
            authorization.set_sensitive(true);
            headers.insert(AUTHORIZATION, authorization);
        }
        let timeout_seconds = env::var("MEMORY_EMBEDDING_TIMEOUT_SECONDS")
            .unwrap_or_else(|_| "10".into())
            .parse::<u64>()
            .map_err(|_| {
                EmbeddingProbeError::Config(
                    "MEMORY_EMBEDDING_TIMEOUT_SECONDS must be an integer".into(),
                )
            })?;
        if !(1..=120).contains(&timeout_seconds) {
            return Err(EmbeddingProbeError::Config(
                "MEMORY_EMBEDDING_TIMEOUT_SECONDS must be between 1 and 120".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(Policy::none())
            .timeout(Duration::from_secs(timeout_seconds))
            .user_agent(concat!(
                "lazytt-embedding-probe/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|error| EmbeddingProbeError::Config(error.to_string()))?;
        Ok(Self {
            base_url,
            manifest,
            include_input_type,
            client,
        })
    }

    pub async fn health_check(&self) -> Result<(), EmbeddingProbeError> {
        let response = self
            .client
            .post(format!("{}/v1/embeddings", self.base_url))
            .json(&EmbeddingRequest {
                model: &self.manifest.embedding_model,
                input: "LazyTT semantic embedding readiness probe",
                dimensions: self.manifest.embedding_dimensions,
                input_type: self.include_input_type.then_some("query"),
            })
            .send()
            .await
            .map_err(|error| EmbeddingProbeError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            return Err(EmbeddingProbeError::Transport(format!(
                "HTTP {}",
                response.status().as_u16()
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| EmbeddingProbeError::Transport(error.to_string()))?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(EmbeddingProbeError::InvalidResponse(
                "response exceeded 1 MiB".into(),
            ));
        }
        validate_response(&self.manifest, &bytes)
    }
}

pub fn embedding_api_key_from_env(provider: &str) -> Option<String> {
    if provider == "aliyun-bailian" {
        env::var("OPENAI_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                env::var("MEMORY_EMBEDDING_API_KEY")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
    } else {
        env::var("MEMORY_EMBEDDING_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
    }
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'static str,
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
    index: usize,
    embedding: Vec<f64>,
}

fn validate_response(
    manifest: &ProjectionModelManifest,
    bytes: &[u8],
) -> Result<(), EmbeddingProbeError> {
    let response: EmbeddingResponse = serde_json::from_slice(bytes)
        .map_err(|error| EmbeddingProbeError::InvalidResponse(error.to_string()))?;
    if response.model != manifest.embedding_model || response.data.len() != 1 {
        return Err(EmbeddingProbeError::InvalidResponse(
            "model or vector count did not match the manifest".into(),
        ));
    }
    let vector = &response.data[0];
    if vector.index != 0
        || vector.embedding.len() != manifest.embedding_dimensions as usize
        || vector.embedding.iter().any(|value| !value.is_finite())
        || vector.embedding.iter().all(|value| *value == 0.0)
    {
        return Err(EmbeddingProbeError::InvalidResponse(
            "vector index, dimensions, finiteness, or norm was invalid".into(),
        ));
    }
    if manifest.embedding_normalization == "l2" {
        let norm = vector
            .embedding
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt();
        if (norm - 1.0).abs() > 0.01 {
            return Err(EmbeddingProbeError::InvalidResponse(
                "vector was not L2 normalized as declared".into(),
            ));
        }
    }
    Ok(())
}

fn normalize_base_url(
    raw: &str,
    allow_local_container_http: bool,
) -> Result<String, EmbeddingProbeError> {
    let mut normalized = raw.trim_end_matches('/').to_owned();
    if let Some(without_v1) = normalized.strip_suffix("/v1") {
        normalized = without_v1.to_owned();
    }
    let url = reqwest::Url::parse(&normalized)
        .map_err(|error| EmbeddingProbeError::Config(format!("invalid base URL: {error}")))?;
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    let local_container_http =
        allow_local_container_http && url.scheme() == "http" && is_local_container_model_host(&url);
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) && !local_container_http {
        return Err(EmbeddingProbeError::Config(
            "base URL must use HTTPS, loopback HTTP, or an explicitly allowed local model container"
                .into(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        return Err(EmbeddingProbeError::Config(
            "base URL must not contain credentials, query, or fragment".into(),
        ));
    }
    Ok(normalized)
}

fn is_local_container_model_host(url: &reqwest::Url) -> bool {
    matches!(
        url.host_str(),
        Some(
            "retrieval-models-cpu"
                | "retrieval-models-cuda"
                | "host.docker.internal"
                | "gateway.docker.internal"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ProjectionModelManifest {
        ProjectionModelManifest::new(
            "mem0".into(),
            "local-qwen".into(),
            "Qwen/Qwen3-Embedding-0.6B".into(),
            "main".into(),
            3,
            "l2".into(),
            "memory-query-v1".into(),
            true,
        )
        .unwrap()
    }

    #[test]
    fn validates_model_dimensions_and_norm() {
        validate_response(
            &manifest(),
            br#"{"model":"Qwen/Qwen3-Embedding-0.6B","data":[{"index":0,"embedding":[1.0,0.0,0.0]}]}"#,
        )
        .unwrap();
        assert!(
            validate_response(
                &manifest(),
                br#"{"model":"wrong","data":[{"index":0,"embedding":[1.0,0.0,0.0]}]}"#,
            )
            .is_err()
        );
    }

    #[test]
    fn external_plain_http_is_rejected() {
        assert!(normalize_base_url("http://example.com/v1", true).is_err());
        assert_eq!(
            normalize_base_url("http://127.0.0.1:18082/v1", false).unwrap(),
            "http://127.0.0.1:18082"
        );
        assert_eq!(
            normalize_base_url("http://retrieval-models-cpu:18082", true).unwrap(),
            "http://retrieval-models-cpu:18082"
        );
        assert!(normalize_base_url("http://retrieval-models-cpu:18082", false).is_err());
    }
}
