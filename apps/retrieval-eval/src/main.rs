use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use mem0_adapter::{Mem0Adapter, Mem0Config};
use memory_application::{ProjectionSearch, RetrievalIndex, lexical_relevance_score};
use memory_postgres::{DEMO_GENERATION, DemoAuthority, authorized_namespace_key};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep};
use uuid::Uuid;

type EvalResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const DEFAULT_DATASET: &str = "evals/retrieval-v1/dataset.json";
const DEFAULT_MEMORY_BASE_URL: &str = "http://127.0.0.1:18084";
const DEFAULT_MEM0_BASE_URL: &str = "http://127.0.0.1:18888";
const EVAL_TENANT_ID: &str = "retrieval-eval-v1";
const EVAL_USER_ID: &str = "evaluator";

#[derive(Debug, Deserialize, Serialize)]
struct Dataset {
    version: String,
    documents: Vec<EvalDocument>,
    positive_queries: Vec<EvalQuery>,
    negative_queries: Vec<NegativeQuery>,
    thresholds: EvalThresholds,
}

#[derive(Debug, Deserialize, Serialize)]
struct EvalDocument {
    id: String,
    content: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EvalQuery {
    id: String,
    query: String,
    relevant_document_ids: Vec<String>,
    top_k: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct NegativeQuery {
    id: String,
    query: String,
    top_k: usize,
}

#[derive(Debug, Deserialize, Serialize)]
struct EvalThresholds {
    recall_at_3: f64,
    mean_reciprocal_rank: f64,
    negative_query_hit_rate: f64,
    forgotten_injection_count: usize,
}

#[derive(Debug, Deserialize)]
struct OperationResponse {
    operation_id: Uuid,
    memory_record_id: Option<Uuid>,
    head_revision: i64,
    projection_status: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    hits: Vec<SearchHit>,
    ranking_version: String,
    projection_generation: String,
    degraded: bool,
    degradation_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchHit {
    memory_record_id: Uuid,
    memory_version_id: Uuid,
    score: f64,
}

#[derive(Debug, Deserialize)]
struct ServiceInfo {
    reranker: String,
}

#[derive(Debug, Clone, Serialize)]
struct QueryResult {
    query_id: String,
    returned_document_ids: Vec<String>,
    scores: Vec<f64>,
    ranking_version: Option<String>,
    projection_generation: Option<String>,
    degraded: bool,
    degradation_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct MetricSet {
    sample_count: usize,
    recall_at_1: f64,
    recall_at_3: f64,
    mean_reciprocal_rank: f64,
    mean_ndcg_at_k: f64,
    irrelevant_injection_rate: f64,
}

#[derive(Debug, Serialize)]
struct MetricsArtifact {
    dataset_version: String,
    generated_at: DateTime<Utc>,
    methods: BTreeMap<String, MetricSet>,
    negative_query_hit_rate: f64,
    forgotten_injection_count: usize,
    quality_gate_passed: bool,
    safety_gate_passed: bool,
    release_gate_passed: bool,
}

#[derive(Debug, Serialize)]
struct EvaluationManifest {
    evaluation_id: Uuid,
    dataset_version: String,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
    memory_service: String,
    provider: String,
    provider_profile: String,
    generation: String,
    ranking_version: String,
    query_concurrency: usize,
    tenant_id: String,
    synthetic_data_only: bool,
}

#[derive(Debug, Serialize)]
struct FailureCase {
    method: String,
    query_id: String,
    expected_document_ids: Vec<String>,
    returned_document_ids: Vec<String>,
    reason: String,
}

struct EvalConfig {
    dataset_path: PathBuf,
    output_dir: PathBuf,
    memory_base_url: String,
    mem0_base_url: String,
    provider_profile: String,
    provider_generation: String,
    bearer_token: Option<String>,
    enforce_gate: bool,
    query_concurrency: usize,
}

#[derive(Clone)]
struct MemoryClient {
    base_url: String,
    client: reqwest::Client,
}

impl MemoryClient {
    fn new(base_url: &str, bearer_token: Option<&str>) -> EvalResult<Self> {
        let parsed = reqwest::Url::parse(base_url)?;
        if parsed.scheme() != "http"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed
                .host_str()
                .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"))
        {
            return Err("retrieval eval only permits a loopback HTTP Memory Service".into());
        }
        let mut headers = HeaderMap::new();
        if let Some(token) = bearer_token {
            let mut value = HeaderValue::from_str(&format!("Bearer {}", token.trim()))?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            client: reqwest::Client::builder()
                .default_headers(headers)
                .timeout(Duration::from_secs(120))
                .build()?,
        })
    }

    async fn ready(&self) -> EvalResult<()> {
        self.client
            .get(format!("{}/health/ready", self.base_url))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn info(&self) -> EvalResult<ServiceInfo> {
        Ok(self
            .client
            .get(&self.base_url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn ingest(&self, run_id: Uuid, document: &EvalDocument) -> EvalResult<OperationResponse> {
        let idempotency_key = format!("retrieval-eval-{run_id}-{}", document.id);
        let response = self
            .client
            .post(format!("{}/v1/memory/events", self.base_url))
            .header("Idempotency-Key", &idempotency_key)
            .json(&json!({
                "event_id": idempotency_key,
                "source_type": "conversation_message",
                "source_ref": format!("evaluation:{}:{}", run_id, document.id),
                "content": format!("请记住：{}", document.content),
                "occurred_at": Utc::now(),
                "scope_selector": {"kind": "user", "id": EVAL_USER_ID}
            }))
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    async fn operation(&self, operation_id: Uuid) -> EvalResult<OperationResponse> {
        Ok(self
            .client
            .get(format!(
                "{}/v1/memory/operations/{operation_id}",
                self.base_url
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn wait_ready(&self, operation_id: Uuid) -> EvalResult<OperationResponse> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let operation = self.operation(operation_id).await?;
            match operation.projection_status.as_str() {
                "ready" => return Ok(operation),
                "failed" | "deleted" | "not_applicable" => {
                    return Err(format!(
                        "projection {operation_id} reached unexpected status {}",
                        operation.projection_status
                    )
                    .into());
                }
                _ if Instant::now() >= deadline => {
                    return Err(format!("projection {operation_id} did not become ready").into());
                }
                _ => sleep(Duration::from_millis(100)).await,
            }
        }
    }

    async fn search(&self, query: &str, top_k: usize) -> EvalResult<SearchResponse> {
        let response = self
            .client
            .post(format!("{}/v1/memory/search", self.base_url))
            .json(&json!({
                "task_context": {
                    "task_id": Uuid::new_v4(),
                    "attempt_id": Uuid::new_v4(),
                    "purpose": "answer_user"
                },
                "query": query,
                "scope_selectors": [{"kind": "user", "id": EVAL_USER_ID}],
                "memory_types": ["preference"],
                "limit": top_k
            }))
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    async fn forget(
        &self,
        run_id: Uuid,
        document_id: &str,
        memory_record_id: Uuid,
        expected_revision: i64,
    ) -> EvalResult<()> {
        self.client
            .post(format!(
                "{}/v1/memories/{memory_record_id}/forget",
                self.base_url
            ))
            .header(
                "Idempotency-Key",
                format!("retrieval-eval-forget-{run_id}-{document_id}"),
            )
            .json(&json!({
                "expected_revision": expected_revision,
                "reason": "synthetic retrieval evaluation cleanup"
            }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> EvalResult<()> {
    let config = load_config()?;
    let dataset: Dataset = serde_json::from_str(&fs::read_to_string(&config.dataset_path)?)?;
    validate_dataset(&dataset)?;
    let evaluation_id = Uuid::new_v4();
    let started_at = Utc::now();
    let client = MemoryClient::new(&config.memory_base_url, config.bearer_token.as_deref())?;
    client.ready().await?;
    let service_info = client.info().await?;

    let provider = Mem0Adapter::new(Mem0Config {
        base_url: config.mem0_base_url.clone(),
        api_key: env::var("MEM0_API_KEY").ok(),
        timeout: Duration::from_secs(10),
    })?;
    let authority = DemoAuthority::user(EVAL_TENANT_ID, EVAL_USER_ID);
    let namespace = authorized_namespace_key(&authority, "user", Some(EVAL_USER_ID))
        .ok_or("failed to derive evaluation namespace")?;

    let mut record_to_document = HashMap::new();
    let mut ingested = Vec::new();
    for document in &dataset.documents {
        let accepted = match client.ingest(evaluation_id, document).await {
            Ok(accepted) => accepted,
            Err(error) => {
                cleanup_ingested(&client, evaluation_id, &ingested).await;
                return Err(error);
            }
        };
        let Some(record_id) = accepted.memory_record_id else {
            cleanup_ingested(&client, evaluation_id, &ingested).await;
            return Err("accepted evaluation document had no memory_record_id".into());
        };
        ingested.push((document.id.clone(), record_id, accepted.head_revision));
        let ready = match client.wait_ready(accepted.operation_id).await {
            Ok(ready) => ready,
            Err(error) => {
                cleanup_ingested(&client, evaluation_id, &ingested).await;
                return Err(error);
            }
        };
        record_to_document.insert(record_id, document.id.clone());
        if let Some(entry) = ingested.last_mut() {
            entry.2 = ready.head_revision;
        }
    }

    let no_memory_results = dataset
        .positive_queries
        .iter()
        .map(|query| QueryResult {
            query_id: query.id.clone(),
            returned_document_ids: Vec::new(),
            scores: Vec::new(),
            ranking_version: None,
            projection_generation: None,
            degraded: false,
            degradation_reason: None,
        })
        .collect::<Vec<_>>();
    let local_results = evaluate_local(&dataset);
    let provider_results = evaluate_provider(
        &provider,
        &namespace,
        &config.provider_generation,
        &dataset,
        &record_to_document,
    )
    .await?;
    let hybrid_results = evaluate_hybrid(
        &client,
        &dataset,
        &record_to_document,
        config.query_concurrency,
    )
    .await?;
    let oracle_results = dataset
        .positive_queries
        .iter()
        .map(|query| QueryResult {
            query_id: query.id.clone(),
            returned_document_ids: query.relevant_document_ids.clone(),
            scores: vec![1.0; query.relevant_document_ids.len()],
            ranking_version: Some("oracle-v1".into()),
            projection_generation: None,
            degraded: false,
            degradation_reason: None,
        })
        .collect::<Vec<_>>();
    let negative_query_hit_rate = evaluate_negative_queries(
        &client,
        &dataset,
        &record_to_document,
        config.query_concurrency,
    )
    .await?;

    for (document_id, record_id, revision) in &ingested {
        let _ = client
            .forget(evaluation_id, document_id, *record_id, *revision)
            .await;
    }
    let forgotten_injection_count = count_forgotten_injections(
        &client,
        &dataset,
        &record_to_document,
        config.query_concurrency,
    )
    .await?;

    let methods = BTreeMap::from([
        (
            "no_memory".into(),
            evaluate_metrics(&dataset, &no_memory_results),
        ),
        (
            "local_lexical".into(),
            evaluate_metrics(&dataset, &local_results),
        ),
        (
            "current_provider".into(),
            evaluate_metrics(&dataset, &provider_results),
        ),
        (
            "hybrid_service".into(),
            evaluate_metrics(&dataset, &hybrid_results),
        ),
        ("oracle".into(), evaluate_metrics(&dataset, &oracle_results)),
    ]);
    let hybrid_metrics = methods
        .get("hybrid_service")
        .ok_or("hybrid metrics missing")?;
    let hybrid_recall_at_3 = hybrid_metrics.recall_at_3;
    let hybrid_mean_reciprocal_rank = hybrid_metrics.mean_reciprocal_rank;
    let quality_gate_passed = hybrid_metrics.recall_at_3 >= dataset.thresholds.recall_at_3
        && hybrid_metrics.mean_reciprocal_rank >= dataset.thresholds.mean_reciprocal_rank;
    let safety_gate_passed = negative_query_hit_rate <= dataset.thresholds.negative_query_hit_rate
        && forgotten_injection_count <= dataset.thresholds.forgotten_injection_count;
    let metrics = MetricsArtifact {
        dataset_version: dataset.version.clone(),
        generated_at: Utc::now(),
        methods,
        negative_query_hit_rate,
        forgotten_injection_count,
        quality_gate_passed,
        safety_gate_passed,
        release_gate_passed: quality_gate_passed && safety_gate_passed,
    };
    let failures = collect_failures(
        &dataset,
        &[
            ("local_lexical", &local_results),
            ("current_provider", &provider_results),
            ("hybrid_service", &hybrid_results),
        ],
    );
    let observed_generations = hybrid_results
        .iter()
        .filter_map(|result| result.projection_generation.as_deref())
        .collect::<HashSet<_>>();
    let observed_generation = if observed_generations.len() == 1 {
        observed_generations
            .into_iter()
            .next()
            .unwrap_or(config.provider_generation.as_str())
            .to_owned()
    } else if observed_generations.is_empty() {
        config.provider_generation.clone()
    } else {
        "mixed-generations-invalid".into()
    };
    let manifest = EvaluationManifest {
        evaluation_id,
        dataset_version: dataset.version.clone(),
        started_at,
        completed_at: Utc::now(),
        memory_service: config.memory_base_url.clone(),
        provider: config.mem0_base_url.clone(),
        provider_profile: config.provider_profile,
        generation: observed_generation,
        ranking_version: format!("hybrid-rrf60-{}", service_info.reranker),
        query_concurrency: config.query_concurrency,
        tenant_id: EVAL_TENANT_ID.into(),
        synthetic_data_only: true,
    };
    write_artifacts(
        &config.output_dir,
        &manifest,
        &metrics,
        &failures,
        &BTreeMap::from([
            ("local_lexical", local_results),
            ("current_provider", provider_results),
            ("hybrid_service", hybrid_results),
        ]),
    )?;
    println!(
        "retrieval evaluation: recall@3={:.3}, MRR={:.3}, negative_hit_rate={:.3}, forgotten_injections={}, gate={}",
        hybrid_recall_at_3,
        hybrid_mean_reciprocal_rank,
        negative_query_hit_rate,
        forgotten_injection_count,
        if metrics.release_gate_passed {
            "PASS"
        } else {
            "FAIL"
        }
    );
    println!("artifacts: {}", config.output_dir.display());
    if config.enforce_gate && !metrics.release_gate_passed {
        return Err("retrieval evaluation gate failed".into());
    }
    Ok(())
}

fn load_config() -> EvalResult<EvalConfig> {
    let dataset_path =
        PathBuf::from(env::var("EVAL_DATASET").unwrap_or_else(|_| DEFAULT_DATASET.into()));
    let output_dir = PathBuf::from(
        env::var("EVAL_OUTPUT_DIR")
            .unwrap_or_else(|_| format!("target/evals/retrieval-v1/{}", Uuid::new_v4())),
    );
    let query_concurrency = env::var("EVAL_QUERY_CONCURRENCY")
        .unwrap_or_else(|_| "2".into())
        .parse::<usize>()
        .map_err(|_| "EVAL_QUERY_CONCURRENCY must be an integer between 1 and 4")?;
    if !(1..=4).contains(&query_concurrency) {
        return Err("EVAL_QUERY_CONCURRENCY must be between 1 and 4".into());
    }
    Ok(EvalConfig {
        dataset_path,
        output_dir,
        memory_base_url: env::var("MEMORY_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_MEMORY_BASE_URL.into()),
        mem0_base_url: env::var("MEM0_BASE_URL").unwrap_or_else(|_| DEFAULT_MEM0_BASE_URL.into()),
        provider_profile: env::var("EVAL_PROVIDER_PROFILE").unwrap_or_else(|_| "unknown".into()),
        provider_generation: env::var("EVAL_GENERATION").unwrap_or_else(|_| DEMO_GENERATION.into()),
        bearer_token: env::var("MEMORY_BEARER_TOKEN").ok(),
        enforce_gate: env::var("EVAL_ENFORCE_GATE").is_ok_and(|value| value == "true"),
        query_concurrency,
    })
}

fn validate_dataset(dataset: &Dataset) -> EvalResult<()> {
    if dataset.version.trim().is_empty()
        || !(5..=100).contains(&dataset.documents.len())
        || dataset.positive_queries.is_empty()
        || dataset.negative_queries.is_empty()
    {
        return Err("dataset shape is outside the retrieval-v1 contract".into());
    }
    let document_ids = dataset
        .documents
        .iter()
        .map(|document| document.id.as_str())
        .collect::<HashSet<_>>();
    if document_ids.len() != dataset.documents.len()
        || dataset
            .documents
            .iter()
            .any(|document| document.content.trim().is_empty())
    {
        return Err("dataset document IDs/content must be unique and non-empty".into());
    }
    for query in &dataset.positive_queries {
        if query.query.trim().is_empty()
            || query.relevant_document_ids.is_empty()
            || !(1..=10).contains(&query.top_k)
            || query
                .relevant_document_ids
                .iter()
                .any(|id| !document_ids.contains(id.as_str()))
        {
            return Err(format!("invalid positive query {}", query.id).into());
        }
    }
    Ok(())
}

fn evaluate_local(dataset: &Dataset) -> Vec<QueryResult> {
    dataset
        .positive_queries
        .iter()
        .map(|query| {
            let mut ranked = dataset
                .documents
                .iter()
                .map(|document| {
                    (
                        document.id.clone(),
                        lexical_relevance_score(&query.query, &document.content),
                    )
                })
                .filter(|(_, score)| *score > 0.0)
                .collect::<Vec<_>>();
            ranked.sort_by(|left, right| {
                right
                    .1
                    .total_cmp(&left.1)
                    .then_with(|| left.0.cmp(&right.0))
            });
            ranked.truncate(query.top_k);
            QueryResult {
                query_id: query.id.clone(),
                returned_document_ids: ranked.iter().map(|(id, _)| id.clone()).collect(),
                scores: ranked.iter().map(|(_, score)| *score).collect(),
                ranking_version: Some("local-lexical-v1".into()),
                projection_generation: None,
                degraded: false,
                degradation_reason: None,
            }
        })
        .collect()
}

async fn evaluate_provider(
    provider: &Mem0Adapter,
    namespace: &str,
    generation: &str,
    dataset: &Dataset,
    record_to_document: &HashMap<Uuid, String>,
) -> EvalResult<Vec<QueryResult>> {
    let mut results = Vec::new();
    for query in &dataset.positive_queries {
        let hits = match provider
            .search(ProjectionSearch {
                namespace_key: namespace.into(),
                query: query.query.clone(),
                generation: generation.into(),
                limit: query.top_k,
            })
            .await
        {
            Ok(hits) => hits,
            Err(error) => {
                results.push(QueryResult {
                    query_id: query.id.clone(),
                    returned_document_ids: Vec::new(),
                    scores: Vec::new(),
                    ranking_version: Some("provider-native-v1".into()),
                    projection_generation: Some(generation.into()),
                    degraded: true,
                    degradation_reason: Some(format!("PROVIDER_ERROR:{error}")),
                });
                continue;
            }
        };
        let mapped = hits
            .into_iter()
            .filter_map(|hit| {
                record_to_document
                    .get(&hit.memory_record_id)
                    .cloned()
                    .map(|id| (id, hit.provider_score))
            })
            .collect::<Vec<_>>();
        results.push(QueryResult {
            query_id: query.id.clone(),
            returned_document_ids: mapped.iter().map(|(id, _)| id.clone()).collect(),
            scores: mapped.iter().map(|(_, score)| *score).collect(),
            ranking_version: Some("provider-native-v1".into()),
            projection_generation: Some(generation.into()),
            degraded: false,
            degradation_reason: None,
        });
    }
    Ok(results)
}

async fn evaluate_hybrid(
    client: &MemoryClient,
    dataset: &Dataset,
    record_to_document: &HashMap<Uuid, String>,
    concurrency: usize,
) -> EvalResult<Vec<QueryResult>> {
    let searches = run_searches(
        client,
        dataset
            .positive_queries
            .iter()
            .map(|query| SearchCase {
                id: query.id.clone(),
                query: query.query.clone(),
                top_k: query.top_k,
            })
            .collect(),
        concurrency,
    )
    .await?;
    let mut results = Vec::with_capacity(searches.len());
    for search in searches {
        let response = match search.response {
            Ok(response) => response,
            Err(error) => {
                results.push(QueryResult {
                    query_id: search.id,
                    returned_document_ids: Vec::new(),
                    scores: Vec::new(),
                    ranking_version: None,
                    projection_generation: None,
                    degraded: true,
                    degradation_reason: Some(format!("HYBRID_ERROR:{error}")),
                });
                continue;
            }
        };
        let mapped = response
            .hits
            .into_iter()
            .filter_map(|hit| {
                let _version_binding = hit.memory_version_id;
                record_to_document
                    .get(&hit.memory_record_id)
                    .cloned()
                    .map(|id| (id, hit.score))
            })
            .collect::<Vec<_>>();
        results.push(QueryResult {
            query_id: search.id,
            returned_document_ids: mapped.iter().map(|(id, _)| id.clone()).collect(),
            scores: mapped.iter().map(|(_, score)| *score).collect(),
            ranking_version: Some(response.ranking_version),
            projection_generation: Some(response.projection_generation),
            degraded: response.degraded,
            degradation_reason: response.degradation_reason,
        });
    }
    Ok(results)
}

async fn evaluate_negative_queries(
    client: &MemoryClient,
    dataset: &Dataset,
    record_to_document: &HashMap<Uuid, String>,
    concurrency: usize,
) -> EvalResult<f64> {
    let mut queries_with_hits = 0_usize;
    let searches = run_searches(
        client,
        dataset
            .negative_queries
            .iter()
            .map(|query| SearchCase {
                id: query.id.clone(),
                query: query.query.clone(),
                top_k: query.top_k,
            })
            .collect(),
        concurrency,
    )
    .await?;
    for search in searches {
        let response = match search.response {
            Ok(response) => response,
            Err(_) => {
                queries_with_hits += 1;
                continue;
            }
        };
        if response
            .hits
            .iter()
            .any(|hit| record_to_document.contains_key(&hit.memory_record_id))
        {
            queries_with_hits += 1;
        }
    }
    Ok(queries_with_hits as f64 / dataset.negative_queries.len() as f64)
}

async fn count_forgotten_injections(
    client: &MemoryClient,
    dataset: &Dataset,
    record_to_document: &HashMap<Uuid, String>,
    concurrency: usize,
) -> EvalResult<usize> {
    let mut count = 0;
    let searches = run_searches(
        client,
        dataset
            .positive_queries
            .iter()
            .map(|query| SearchCase {
                id: query.id.clone(),
                query: query.query.clone(),
                top_k: query.top_k,
            })
            .collect(),
        concurrency,
    )
    .await?;
    for search in searches {
        let response = match search.response {
            Ok(response) => response,
            Err(_) => {
                count += 1;
                continue;
            }
        };
        count += response
            .hits
            .iter()
            .filter(|hit| record_to_document.contains_key(&hit.memory_record_id))
            .count();
    }
    Ok(count)
}

struct SearchCase {
    id: String,
    query: String,
    top_k: usize,
}

struct SearchAttempt {
    index: usize,
    id: String,
    response: Result<SearchResponse, String>,
}

async fn run_searches(
    client: &MemoryClient,
    cases: Vec<SearchCase>,
    concurrency: usize,
) -> EvalResult<Vec<SearchAttempt>> {
    let mut pending = cases.into_iter().enumerate();
    let mut tasks = JoinSet::new();
    let mut results = Vec::new();
    loop {
        while tasks.len() < concurrency {
            let Some((index, case)) = pending.next() else {
                break;
            };
            let client = client.clone();
            tasks.spawn(async move {
                let response = client
                    .search(&case.query, case.top_k)
                    .await
                    .map_err(|error| error.to_string());
                SearchAttempt {
                    index,
                    id: case.id,
                    response,
                }
            });
        }
        if tasks.is_empty() {
            break;
        }
        let result = tasks
            .join_next()
            .await
            .ok_or("bounded search task set ended unexpectedly")?
            .map_err(|error| format!("bounded search task failed: {error}"))?;
        results.push(result);
    }
    results.sort_by_key(|result| result.index);
    Ok(results)
}

async fn cleanup_ingested(
    client: &MemoryClient,
    evaluation_id: Uuid,
    ingested: &[(String, Uuid, i64)],
) {
    for (document_id, record_id, revision) in ingested {
        let _ = client
            .forget(evaluation_id, document_id, *record_id, *revision)
            .await;
    }
}

fn evaluate_metrics(dataset: &Dataset, results: &[QueryResult]) -> MetricSet {
    let by_query = results
        .iter()
        .map(|result| (result.query_id.as_str(), result))
        .collect::<HashMap<_, _>>();
    let mut recall_at_1 = 0.0;
    let mut recall_at_3 = 0.0;
    let mut reciprocal_rank = 0.0;
    let mut ndcg = 0.0;
    let mut irrelevant = 0_usize;
    let mut returned = 0_usize;
    for query in &dataset.positive_queries {
        let result = by_query
            .get(query.id.as_str())
            .expect("validated evaluation result must cover every query");
        let relevant = query
            .relevant_document_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let hits_at_1 = result
            .returned_document_ids
            .iter()
            .take(1)
            .filter(|id| relevant.contains(id.as_str()))
            .count();
        let hits_at_3 = result
            .returned_document_ids
            .iter()
            .take(3)
            .filter(|id| relevant.contains(id.as_str()))
            .count();
        recall_at_1 += hits_at_1 as f64 / relevant.len() as f64;
        recall_at_3 += hits_at_3 as f64 / relevant.len() as f64;
        if let Some(rank) = result
            .returned_document_ids
            .iter()
            .position(|id| relevant.contains(id.as_str()))
        {
            reciprocal_rank += 1.0 / (rank as f64 + 1.0);
            ndcg += 1.0 / (rank as f64 + 2.0).log2();
        }
        irrelevant += result
            .returned_document_ids
            .iter()
            .filter(|id| !relevant.contains(id.as_str()))
            .count();
        returned += result.returned_document_ids.len();
    }
    let samples = dataset.positive_queries.len() as f64;
    MetricSet {
        sample_count: dataset.positive_queries.len(),
        recall_at_1: recall_at_1 / samples,
        recall_at_3: recall_at_3 / samples,
        mean_reciprocal_rank: reciprocal_rank / samples,
        mean_ndcg_at_k: ndcg / samples,
        irrelevant_injection_rate: if returned == 0 {
            0.0
        } else {
            irrelevant as f64 / returned as f64
        },
    }
}

fn collect_failures(dataset: &Dataset, methods: &[(&str, &Vec<QueryResult>)]) -> Vec<FailureCase> {
    let query_by_id = dataset
        .positive_queries
        .iter()
        .map(|query| (query.id.as_str(), query))
        .collect::<HashMap<_, _>>();
    let mut failures = Vec::new();
    for (method, results) in methods {
        for result in *results {
            let query = query_by_id[result.query_id.as_str()];
            if !query
                .relevant_document_ids
                .iter()
                .any(|expected| result.returned_document_ids.contains(expected))
            {
                failures.push(FailureCase {
                    method: (*method).into(),
                    query_id: query.id.clone(),
                    expected_document_ids: query.relevant_document_ids.clone(),
                    returned_document_ids: result.returned_document_ids.clone(),
                    reason: "NO_RELEVANT_DOCUMENT_IN_TOP_K".into(),
                });
            }
        }
    }
    failures
}

fn write_artifacts(
    output_dir: &Path,
    manifest: &EvaluationManifest,
    metrics: &MetricsArtifact,
    failures: &[FailureCase],
    raw_results: &BTreeMap<&str, Vec<QueryResult>>,
) -> EvalResult<()> {
    fs::create_dir_all(output_dir)?;
    fs::write(
        output_dir.join("evaluation-manifest.json"),
        serde_json::to_vec_pretty(manifest)?,
    )?;
    fs::write(
        output_dir.join("metric-results.json"),
        serde_json::to_vec_pretty(metrics)?,
    )?;
    fs::write(
        output_dir.join("raw-results.json"),
        serde_json::to_vec_pretty(raw_results)?,
    )?;
    let mut failure_jsonl = String::new();
    for failure in failures {
        writeln!(&mut failure_jsonl, "{}", serde_json::to_string(failure)?)?;
    }
    fs::write(output_dir.join("failure-cases.jsonl"), failure_jsonl)?;

    let mut report = String::from("# Retrieval Evaluation Comparison\n\n");
    writeln!(
        &mut report,
        "Dataset `{}`，样本 {} 条；Release Gate：**{}**。\n",
        metrics.dataset_version,
        metrics
            .methods
            .get("hybrid_service")
            .map_or(0, |metric| metric.sample_count),
        if metrics.release_gate_passed {
            "PASS"
        } else {
            "FAIL"
        }
    )?;
    report.push_str("| 方法 | Recall@1 | Recall@3 | MRR | nDCG | 无关注入率 |\n");
    report.push_str("|---|---:|---:|---:|---:|---:|\n");
    for (method, metric) in &metrics.methods {
        writeln!(
            &mut report,
            "| {method} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
            metric.recall_at_1,
            metric.recall_at_3,
            metric.mean_reciprocal_rank,
            metric.mean_ndcg_at_k,
            metric.irrelevant_injection_rate
        )?;
    }
    writeln!(
        &mut report,
        "\n- Negative query hit rate: {:.3}\n- Forgotten injection count: {}\n- Quality gate: {}\n- Safety gate: {}\n- Failure cases: {}\n",
        metrics.negative_query_hit_rate,
        metrics.forgotten_injection_count,
        metrics.quality_gate_passed,
        metrics.safety_gate_passed,
        failures.len()
    )?;
    report.push_str(
        "本报告只使用合成数据。`current_provider` 表示直接 Mem0 结果；\
         `hybrid_service` 表示在线 Memory API 的 Provider + PostgreSQL 本地候选、\
         RRF、canonical 重新授权和服务当前配置的 Reranker。\n",
    );
    fs::write(output_dir.join("comparison-report.md"), report)?;
    Ok(())
}
