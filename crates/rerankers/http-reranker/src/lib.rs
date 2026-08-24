use std::collections::HashSet;
use std::env;
use std::time::Duration;

use async_trait::async_trait;
use memory_application::{
    MemoryReranker, RerankRequest, RerankedCandidate, RerankerError,
    is_memory_applicable_without_semantic_threshold,
};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};

const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const DEFAULT_INSTRUCTION: &str = "Given an agent memory query, judge whether each memory is relevant to completing the current task";

#[derive(Debug, Clone)]
pub struct HttpRerankerConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub revision: String,
    pub dialect: String,
    pub instruction: String,
    pub minimum_score: f64,
    pub timeout: Duration,
    pub allow_insecure_http_for_local_development: bool,
    pub response_model_required: bool,
}

impl HttpRerankerConfig {
    pub fn from_env() -> Result<Self, RerankerError> {
        let dialect =
            env::var("MEMORY_DEDICATED_RERANK_DIALECT").unwrap_or_else(|_| "default".into());
        let minimum_score = env::var("MEMORY_DEDICATED_RERANK_MINIMUM_SCORE")
            .unwrap_or_else(|_| "0.10".into())
            .parse::<f64>()
            .map_err(|_| {
                RerankerError::Config(
                    "MEMORY_DEDICATED_RERANK_MINIMUM_SCORE must be a number".into(),
                )
            })?;
        let timeout_seconds = env::var("MEMORY_DEDICATED_RERANK_TIMEOUT_SECONDS")
            .unwrap_or_else(|_| "10".into())
            .parse::<u64>()
            .map_err(|_| {
                RerankerError::Config(
                    "MEMORY_DEDICATED_RERANK_TIMEOUT_SECONDS must be an integer".into(),
                )
            })?;
        Ok(Self {
            base_url: env::var("MEMORY_DEDICATED_RERANK_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:18082".into()),
            api_key: if dialect == "bailian_qwen3" {
                env::var("OPENAI_API_KEY")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| {
                        env::var("MEMORY_DEDICATED_RERANK_API_KEY")
                            .ok()
                            .filter(|value| !value.trim().is_empty())
                    })
            } else {
                env::var("MEMORY_DEDICATED_RERANK_API_KEY")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            },
            model: env::var("MEMORY_DEDICATED_RERANK_MODEL")
                .unwrap_or_else(|_| "Qwen/Qwen3-Reranker-0.6B".into()),
            revision: env::var("MEMORY_DEDICATED_RERANK_REVISION")
                .unwrap_or_else(|_| "e61197e".into()),
            dialect,
            instruction: env::var("MEMORY_DEDICATED_RERANK_INSTRUCTION")
                .unwrap_or_else(|_| DEFAULT_INSTRUCTION.into()),
            minimum_score,
            timeout: Duration::from_secs(timeout_seconds),
            allow_insecure_http_for_local_development: env::var("MEMORY_ALLOW_INSECURE_MODEL_HTTP")
                .is_ok_and(|value| value.eq_ignore_ascii_case("true")),
            response_model_required: env::var("MEMORY_DEDICATED_RERANK_RESPONSE_MODEL_REQUIRED")
                .map_or(true, |value| !value.eq_ignore_ascii_case("false")),
        })
    }
}

pub struct HttpReranker {
    base_url: String,
    model: String,
    dialect: HttpRerankerDialect,
    instruction: String,
    minimum_score: f64,
    response_model_required: bool,
    version: String,
    client: reqwest::Client,
}

impl HttpReranker {
    pub fn new(config: HttpRerankerConfig) -> Result<Self, RerankerError> {
        validate_config(&config)?;
        let dialect = HttpRerankerDialect::parse(&config.dialect)?;
        let base_url = normalize_base_url(
            &config.base_url,
            config.allow_insecure_http_for_local_development,
        )?;
        let mut headers = HeaderMap::new();
        if let Some(api_key) = config.api_key {
            let mut authorization = HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
                .map_err(|_| {
                    RerankerError::Config("reranker API key is not a valid header value".into())
                })?;
            authorization.set_sensitive(true);
            headers.insert(AUTHORIZATION, authorization);
        }
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(Policy::none())
            .timeout(config.timeout)
            .user_agent(concat!("lazytt-http-reranker/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| RerankerError::Config(error.to_string()))?;
        let version = match dialect {
            HttpRerankerDialect::Default => {
                format!("dedicated-http:{}@{}-v1", config.model, config.revision)
            }
            HttpRerankerDialect::BailianQwen3 => format!(
                "dedicated-http:{}@{}-bailian-qwen3-v1",
                config.model, config.revision
            ),
        };
        Ok(Self {
            base_url,
            model: config.model,
            dialect,
            instruction: config.instruction,
            minimum_score: config.minimum_score,
            response_model_required: config.response_model_required,
            version,
            client,
        })
    }

    fn endpoint(&self) -> String {
        match self.dialect {
            HttpRerankerDialect::Default => format!("{}/v1/rerank", self.base_url),
            HttpRerankerDialect::BailianQwen3 => {
                format!("{}/compatible-api/v1/reranks", self.base_url)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpRerankerDialect {
    Default,
    BailianQwen3,
}

impl HttpRerankerDialect {
    fn parse(value: &str) -> Result<Self, RerankerError> {
        match value {
            "default" => Ok(Self::Default),
            "bailian_qwen3" => Ok(Self::BailianQwen3),
            _ => Err(RerankerError::Config(
                "MEMORY_DEDICATED_RERANK_DIALECT must be default or bailian_qwen3".into(),
            )),
        }
    }
}

#[derive(Serialize)]
struct HttpRerankRequest<'a> {
    model: &'a str,
    query: &'a str,
    documents: Vec<String>,
    top_n: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    instruction: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instruct: Option<&'a str>,
}

#[derive(Deserialize)]
struct HttpRerankResponse {
    #[serde(default)]
    model: Option<String>,
    results: Vec<HttpRerankResult>,
}

#[derive(Deserialize)]
struct HttpRerankResult {
    index: usize,
    relevance_score: f64,
}

#[async_trait]
impl MemoryReranker for HttpReranker {
    fn version(&self) -> &str {
        &self.version
    }

    async fn rerank(
        &self,
        request: RerankRequest,
    ) -> Result<Vec<RerankedCandidate>, RerankerError> {
        validate_request(&request)?;
        if request.candidates.is_empty() {
            return Ok(Vec::new());
        }
        let response = self
            .client
            .post(self.endpoint())
            .json(&HttpRerankRequest {
                model: &self.model,
                query: &request.query,
                documents: request
                    .candidates
                    .iter()
                    .map(|candidate| {
                        format!(
                            "[memory_type={}]\n{}",
                            candidate.memory_type, candidate.content
                        )
                    })
                    .collect(),
                top_n: request.candidates.len(),
                instruction: (self.dialect == HttpRerankerDialect::Default)
                    .then_some(self.instruction.as_str()),
                instruct: (self.dialect == HttpRerankerDialect::BailianQwen3)
                    .then_some(self.instruction.as_str()),
            })
            .send()
            .await
            .map_err(|error| RerankerError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            return Err(RerankerError::Transport(format!(
                "dedicated reranker returned HTTP {}",
                response.status().as_u16()
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| RerankerError::Transport(error.to_string()))?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(RerankerError::InvalidResponse(
                "response exceeded 256 KiB".into(),
            ));
        }
        parse_response(
            &request,
            &self.model,
            self.minimum_score,
            self.response_model_required,
            &bytes,
        )
    }
}

fn parse_response(
    request: &RerankRequest,
    expected_model: &str,
    minimum_score: f64,
    response_model_required: bool,
    bytes: &[u8],
) -> Result<Vec<RerankedCandidate>, RerankerError> {
    let response: HttpRerankResponse = serde_json::from_slice(bytes)
        .map_err(|error| RerankerError::InvalidResponse(error.to_string()))?;
    if response
        .model
        .as_deref()
        .is_some_and(|model| model != expected_model)
        || (response_model_required && response.model.is_none())
    {
        return Err(RerankerError::InvalidResponse(
            "response model did not match the configured model".into(),
        ));
    }
    if response.results.len() != request.candidates.len() {
        return Err(RerankerError::InvalidResponse(
            "response must score every input candidate exactly once".into(),
        ));
    }
    let mut seen = HashSet::with_capacity(response.results.len());
    let mut ranked = Vec::with_capacity(response.results.len());
    for result in response.results {
        if result.index >= request.candidates.len() || !seen.insert(result.index) {
            return Err(RerankerError::InvalidResponse(
                "response contained an out-of-range or duplicate index".into(),
            ));
        }
        if !result.relevance_score.is_finite() || !(0.0..=1.0).contains(&result.relevance_score) {
            return Err(RerankerError::InvalidResponse(
                "response score must be finite and between 0 and 1".into(),
            ));
        }
        let candidate = &request.candidates[result.index];
        let type_applicable =
            is_memory_applicable_without_semantic_threshold(&request.query, &candidate.memory_type);
        if result.relevance_score >= minimum_score || type_applicable {
            ranked.push(RerankedCandidate {
                memory_version_id: candidate.memory_version_id,
                relevance_score: result.relevance_score,
                reason_code: if type_applicable {
                    "DEDICATED_RERANK_POLICY_APPLICABILITY"
                } else {
                    "DEDICATED_SEMANTIC_RERANK"
                }
                .into(),
            });
        }
    }
    ranked.sort_by(|left, right| {
        right
            .relevance_score
            .total_cmp(&left.relevance_score)
            .then_with(|| left.memory_version_id.cmp(&right.memory_version_id))
    });
    ranked.truncate(request.limit);
    Ok(ranked)
}

fn validate_config(config: &HttpRerankerConfig) -> Result<(), RerankerError> {
    HttpRerankerDialect::parse(&config.dialect)?;
    for (name, value, max_len) in [
        ("model", config.model.as_str(), 256usize),
        ("revision", config.revision.as_str(), 256usize),
        ("instruction", config.instruction.as_str(), 1024usize),
    ] {
        if value.is_empty()
            || value.len() > max_len
            || value.trim() != value
            || value.chars().any(char::is_control)
        {
            return Err(RerankerError::Config(format!(
                "{name} must be trimmed, non-empty, bounded, and contain no control characters"
            )));
        }
    }
    if !config.minimum_score.is_finite() || !(0.0..=1.0).contains(&config.minimum_score) {
        return Err(RerankerError::Config(
            "minimum score must be between 0 and 1".into(),
        ));
    }
    if !(1..=120).contains(&config.timeout.as_secs()) {
        return Err(RerankerError::Config(
            "timeout must be between 1 and 120 seconds".into(),
        ));
    }
    Ok(())
}

fn validate_request(request: &RerankRequest) -> Result<(), RerankerError> {
    if request.query.trim().is_empty()
        || request.query.len() > 4096
        || request.limit == 0
        || request.limit > 50
        || request.candidates.len() > 50
        || request.candidates.iter().any(|candidate| {
            candidate.content.is_empty()
                || candidate.content.len() > 32_768
                || candidate.memory_type.trim().is_empty()
                || candidate.memory_type.len() > 64
                || candidate.memory_type.trim() != candidate.memory_type
                || candidate.memory_type.chars().any(char::is_control)
                || !candidate.retrieval_score.is_finite()
                || !(0.0..=1.0).contains(&candidate.retrieval_score)
        })
    {
        return Err(RerankerError::Config(
            "request exceeded the bounded dedicated reranker contract".into(),
        ));
    }
    Ok(())
}

fn normalize_base_url(
    raw: &str,
    allow_local_container_http: bool,
) -> Result<String, RerankerError> {
    let url = reqwest::Url::parse(raw.trim_end_matches('/'))
        .map_err(|error| RerankerError::Config(format!("invalid base URL: {error}")))?;
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    let local_container_http =
        allow_local_container_http && url.scheme() == "http" && is_local_container_model_host(&url);
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) && !local_container_http {
        return Err(RerankerError::Config(
            "base URL must use HTTPS, loopback HTTP, or an explicitly allowed local model container"
                .into(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        return Err(RerankerError::Config(
            "base URL must not contain credentials, query, or fragment".into(),
        ));
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
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
    use memory_application::RerankCandidate;
    use uuid::Uuid;

    use super::*;

    fn request() -> RerankRequest {
        RerankRequest {
            query: "回答缓存问题应该参考什么？".into(),
            candidates: vec![
                RerankCandidate {
                    memory_version_id: Uuid::from_u128(1),
                    content: "缓存问题先查权威账本".into(),
                    memory_type: "semantic_fact".into(),
                    retrieval_score: 0.5,
                },
                RerankCandidate {
                    memory_version_id: Uuid::from_u128(2),
                    content: "无关内容".into(),
                    memory_type: "episodic_summary".into(),
                    retrieval_score: 0.8,
                },
            ],
            limit: 1,
        }
    }

    #[test]
    fn parses_complete_bounded_response_and_filters() {
        let result = parse_response(
            &request(),
            "Qwen/Qwen3-Reranker-0.6B",
            0.35,
            true,
            br#"{"model":"Qwen/Qwen3-Reranker-0.6B","results":[{"index":1,"relevance_score":0.1},{"index":0,"relevance_score":0.95}]}"#,
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].memory_version_id, Uuid::from_u128(1));
        assert_eq!(result[0].reason_code, "DEDICATED_SEMANTIC_RERANK");
    }

    #[test]
    fn rejects_duplicate_indexes() {
        let error = parse_response(
            &request(),
            "Qwen/Qwen3-Reranker-0.6B",
            0.0,
            true,
            br#"{"model":"Qwen/Qwen3-Reranker-0.6B","results":[{"index":0,"relevance_score":0.9},{"index":0,"relevance_score":0.8}]}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn retains_preference_below_semantic_threshold_without_inflating_score() {
        let mut request = request();
        request.candidates[1].memory_type = "preference".into();
        request.limit = 2;
        let result = parse_response(
            &request,
            "Qwen/Qwen3-Reranker-0.6B",
            0.35,
            true,
            br#"{"model":"Qwen/Qwen3-Reranker-0.6B","results":[{"index":0,"relevance_score":0.01},{"index":1,"relevance_score":0.02}]}"#,
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].memory_version_id, Uuid::from_u128(2));
        assert_eq!(result[0].relevance_score, 0.02);
        assert_eq!(
            result[0].reason_code,
            "DEDICATED_RERANK_POLICY_APPLICABILITY"
        );
    }

    #[test]
    fn external_plain_http_is_rejected() {
        assert!(normalize_base_url("http://example.com:8080", true).is_err());
        assert_eq!(
            normalize_base_url("http://127.0.0.1:18082/", false).unwrap(),
            "http://127.0.0.1:18082"
        );
        assert_eq!(
            normalize_base_url("http://retrieval-models-cuda:18082", true).unwrap(),
            "http://retrieval-models-cuda:18082"
        );
        assert!(normalize_base_url("http://retrieval-models-cuda:18082", false).is_err());
    }

    #[test]
    fn accepts_siliconflow_response_without_model_when_explicitly_configured() {
        let result = parse_response(
            &request(),
            "Qwen/Qwen3-Reranker-0.6B",
            0.0,
            false,
            br#"{"id":"rerank-test","results":[{"index":0,"relevance_score":0.9},{"index":1,"relevance_score":0.1}],"meta":{"tokens":{"input_tokens":12}}}"#,
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].memory_version_id, Uuid::from_u128(1));
    }

    #[test]
    fn bailian_qwen3_uses_plural_endpoint_and_instruct_field() {
        let reranker = HttpReranker::new(HttpRerankerConfig {
            base_url: "https://dashscope.aliyuncs.com".into(),
            api_key: Some("test-key".into()),
            model: "qwen3-rerank".into(),
            revision: "bailian-qwen3-rerank-2026-08".into(),
            dialect: "bailian_qwen3".into(),
            instruction: DEFAULT_INSTRUCTION.into(),
            minimum_score: 0.1,
            timeout: Duration::from_secs(10),
            allow_insecure_http_for_local_development: false,
            response_model_required: true,
        })
        .unwrap();
        assert_eq!(
            reranker.endpoint(),
            "https://dashscope.aliyuncs.com/compatible-api/v1/reranks"
        );

        let body = serde_json::to_value(HttpRerankRequest {
            model: "qwen3-rerank",
            query: "query",
            documents: vec!["document".into()],
            top_n: 1,
            instruction: None,
            instruct: Some(DEFAULT_INSTRUCTION),
        })
        .unwrap();
        assert_eq!(body["instruct"], DEFAULT_INSTRUCTION);
        assert!(body.get("instruction").is_none());
    }

    #[test]
    fn rejects_unknown_reranker_dialect() {
        assert!(HttpRerankerDialect::parse("unknown").is_err());
    }
}
