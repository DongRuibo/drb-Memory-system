mod api;
mod auth;
mod embedding;
mod worker;

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http_reranker::{HttpReranker, HttpRerankerConfig};
use kimi_extractor::{KimiExtractor, KimiExtractorConfig};
use kimi_reranker::{KimiReranker, KimiRerankerConfig};
use mem0_adapter::{Mem0Adapter, Mem0Config};
use memory_application::{MemoryExtractor, MemoryReranker, RetrievalIndex, TransparentReranker};
use memory_postgres::{
    DemoAuthority, MemoryRepository, ProjectionModelManifest, ProjectionRebuildCommand,
};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::api::{AppState, router};
use crate::auth::Authenticator;
use crate::embedding::EmbeddingProbe;
use crate::worker::{ConsolidationWorker, ProjectionWorker};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into());
    let mem0_base_url =
        env::var("MEM0_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:18888".into());
    let bind: SocketAddr = env::var("MEMORY_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()?;
    let mut authority = DemoAuthority::user(
        env::var("DEMO_TENANT_ID").unwrap_or_else(|_| "demo-tenant".into()),
        env::var("DEMO_USER_ID").unwrap_or_else(|_| "semantic-demo".into()),
    );
    let demo_project_id =
        env::var("DEMO_PROJECT_ID").unwrap_or_else(|_| "lazytt-memory-system-demo".into());
    if demo_project_id.trim().is_empty()
        || demo_project_id.len() > 256
        || demo_project_id.chars().any(char::is_control)
    {
        return Err("DEMO_PROJECT_ID must contain 1 to 256 visible characters".into());
    }
    authority.project_ids.insert(demo_project_id);
    let authenticator = Authenticator::from_env(authority.clone())?;
    let projection_model_manifest = projection_model_manifest_from_env()?;
    let embedding_probe = Arc::new(EmbeddingProbe::from_env(projection_model_manifest.clone())?);
    if authenticator.is_demo() && !bind.ip().is_loopback() {
        return Err("MEMORY_AUTH_MODE=demo may only bind to a loopback address".into());
    }

    let repository = MemoryRepository::connect(&database_url).await?;
    if env::var("AUTO_MIGRATE").map_or(true, |value| value != "false") {
        repository.migrate().await?;
    }
    let index: Arc<dyn RetrievalIndex> = Arc::new(Mem0Adapter::new(Mem0Config {
        base_url: mem0_base_url,
        api_key: env::var("MEM0_API_KEY").ok(),
        timeout: Duration::from_secs(5),
    })?);
    let reranker: Arc<dyn MemoryReranker> = match env::var("MEMORY_RERANK_MODE")
        .unwrap_or_else(|_| "transparent".into())
        .as_str()
    {
        "transparent" => Arc::new(TransparentReranker),
        "kimi" => Arc::new(KimiReranker::new(KimiRerankerConfig::from_env_or_dotenv()?)?),
        "dedicated" => Arc::new(HttpReranker::new(HttpRerankerConfig::from_env()?)?),
        _ => return Err("MEMORY_RERANK_MODE must be transparent, kimi, or dedicated".into()),
    };
    let extractor: Option<Arc<dyn MemoryExtractor>> = match env::var("MEMORY_EXTRACT_MODE")
        .unwrap_or_else(|_| "disabled".into())
        .as_str()
    {
        "disabled" => None,
        "kimi" => Some(Arc::new(KimiExtractor::new(
            KimiExtractorConfig::from_env_or_dotenv()?,
        )?)),
        _ => return Err("MEMORY_EXTRACT_MODE must be disabled or kimi".into()),
    };
    let worker_authorities = authenticator
        .tenant_ids()
        .into_iter()
        .map(DemoAuthority::worker)
        .collect::<Vec<_>>();
    if env::var("MEMORY_BOOTSTRAP_EMPTY_PROJECTION")
        .is_ok_and(|value| value.eq_ignore_ascii_case("true"))
    {
        for worker_authority in &worker_authorities {
            bootstrap_empty_projection_generation(
                &repository,
                worker_authority,
                &projection_model_manifest,
            )
            .await?;
        }
    }
    let state = Arc::new(AppState {
        repository: repository.clone(),
        index: Arc::clone(&index),
        reranker: Arc::clone(&reranker),
        extractor: extractor.clone(),
        embedding_probe,
        projection_model_manifest: projection_model_manifest.clone(),
        readiness_authorities: worker_authorities.clone(),
        authenticator: authenticator.clone(),
    });

    let mut worker_handles = Vec::new();
    for worker_authority in worker_authorities {
        let projection_worker = ProjectionWorker::new(
            repository.clone(),
            Arc::clone(&index),
            worker_authority.clone(),
            projection_model_manifest.manifest_hash.clone(),
        );
        worker_handles.push(tokio::spawn(projection_worker.run()));
        if let Some(extractor) = extractor.as_ref() {
            let consolidation_worker = ConsolidationWorker::new(
                repository.clone(),
                Arc::clone(extractor),
                worker_authority,
            );
            worker_handles.push(tokio::spawn(consolidation_worker.run()));
        }
    }
    let listener = TcpListener::bind(bind).await?;
    info!(
        %bind,
        authentication = authenticator.mode_name(),
        embedding_model = projection_model_manifest.embedding_model,
        embedding_revision = projection_model_manifest.embedding_revision,
        semantic_embedding = projection_model_manifest.semantic_embedding,
        reranker = reranker.version(),
        extractor = extractor
            .as_ref()
            .map_or("disabled", |extractor| extractor.version()),
        "LazyTT Memory Service listening"
    );
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    for worker_handle in worker_handles {
        worker_handle.abort();
    }
    Ok(())
}

async fn bootstrap_empty_projection_generation(
    repository: &MemoryRepository,
    authority: &DemoAuthority,
    manifest: &ProjectionModelManifest,
) -> Result<(), Box<dyn std::error::Error>> {
    let current = repository
        .current_projection_generation_view(authority)
        .await?;
    if current.model_manifest.manifest_hash == manifest.manifest_hash {
        return Ok(());
    }
    if current.expected_records != 0 || current.ready_records != 0 || current.failed_records != 0 {
        return Err(format!(
            "MEMORY_BOOTSTRAP_EMPTY_PROJECTION cannot replace non-empty generation {}; run the controlled projection rebuild instead",
            current.generation
        )
        .into());
    }

    let hash_prefix = manifest
        .manifest_hash
        .get(..16)
        .ok_or("projection model manifest hash is unexpectedly short")?;
    let generation = format!("bootstrap-{hash_prefix}");
    let rebuild_key = format!("bootstrap-rebuild-{hash_prefix}");
    let activate_key = format!("bootstrap-activate-{hash_prefix}");
    let finalize_key = format!("bootstrap-finalize-{hash_prefix}");
    let mut target = repository
        .start_projection_rebuild(
            authority,
            ProjectionRebuildCommand {
                generation: generation.clone(),
                idempotency_key: rebuild_key,
                model_manifest: manifest.clone(),
            },
        )
        .await?;
    if target.status == "ready" {
        target = repository
            .activate_projection_generation(
                authority,
                &generation,
                &current.generation,
                &activate_key,
            )
            .await?;
    }
    if target.status == "canary" {
        target = repository
            .finalize_projection_generation(authority, &generation, &finalize_key)
            .await?;
    }
    if target.status != "active" || target.model_manifest.manifest_hash != manifest.manifest_hash {
        return Err(format!(
            "empty projection bootstrap did not converge: generation={} status={}",
            target.generation, target.status
        )
        .into());
    }
    info!(
        tenant_id = authority.tenant_id,
        generation = target.generation,
        manifest_hash = manifest.manifest_hash,
        "bootstrapped empty projection generation"
    );
    Ok(())
}

fn projection_model_manifest_from_env()
-> Result<ProjectionModelManifest, Box<dyn std::error::Error>> {
    let semantic_embedding = match env::var("MEMORY_EMBEDDING_SEMANTIC")
        .unwrap_or_else(|_| "false".into())
        .as_str()
    {
        "true" => true,
        "false" => false,
        _ => return Err("MEMORY_EMBEDDING_SEMANTIC must be true or false".into()),
    };
    let dimensions = env::var("MEMORY_EMBEDDING_DIMENSIONS")
        .unwrap_or_else(|_| "1536".into())
        .parse::<i32>()?;
    let manifest = ProjectionModelManifest::new(
        env::var("MEMORY_INDEX_PROVIDER").unwrap_or_else(|_| "mem0".into()),
        env::var("MEMORY_EMBEDDING_PROVIDER").unwrap_or_else(|_| "local-poc".into()),
        env::var("MEMORY_EMBEDDING_MODEL").unwrap_or_else(|_| "poc-deterministic-embedding".into()),
        env::var("MEMORY_EMBEDDING_REVISION").unwrap_or_else(|_| "poc-v1".into()),
        dimensions,
        env::var("MEMORY_EMBEDDING_NORMALIZATION").unwrap_or_else(|_| "none".into()),
        env::var("MEMORY_EMBEDDING_INSTRUCTION_VERSION").unwrap_or_else(|_| "none".into()),
        semantic_embedding,
    )?;
    if manifest.semantic_embedding
        && (manifest.embedding_provider == "local-poc"
            || manifest.embedding_model == "poc-deterministic-embedding")
    {
        return Err("POC deterministic embeddings cannot be declared semantic".into());
    }
    Ok(manifest)
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
