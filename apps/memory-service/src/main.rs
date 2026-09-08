mod api;
mod auth;
mod embedding;
mod server;
mod worker;

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http_reranker::{HttpReranker, HttpRerankerConfig};
use kimi_extractor::{
    KimiExtractor, KimiExtractorConfig, OpenAiCompatibleExtractor, OpenAiCompatibleExtractorConfig,
};
use kimi_reranker::{KimiReranker, KimiRerankerConfig};
use mem0_adapter::{Mem0Adapter, Mem0Config};
use memory_application::{MemoryExtractor, MemoryReranker, RetrievalIndex, TransparentReranker};
use memory_postgres::{DemoAuthority, MemoryRepository, ProjectionModelManifest};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::api::{
    AppState, PrimaryAppState, health_router, primary_health_router, primary_router, router,
};
use crate::auth::Authenticator;
use crate::embedding::EmbeddingProbe;
use crate::worker::{
    ConsolidationWorker, DynamicTenantWorkers, ProjectionWorker,
    bootstrap_empty_projection_generation,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into());
    let bind: SocketAddr = env::var("MEMORY_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()?;
    let health_bind: SocketAddr = env::var("MEMORY_HEALTH_BIND")?.parse()?;
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
    let backend_mode = env::var("MEMORY_BACKEND_MODE").unwrap_or_else(|_| "governed".into());
    if backend_mode == "mem0_primary" {
        if authenticator.allows_dynamic_tenants() {
            return Err("MEMORY_BACKEND_MODE=mem0_primary requires configured tenant IDs".into());
        }
        let mem0 = Arc::new(Mem0Adapter::new(Mem0Config {
            base_url: env::var("MEM0_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:18888".into()),
            api_key: env::var("MEM0_API_KEY").ok(),
            timeout: duration_from_env("MEMORY_INDEX_TIMEOUT_SECONDS", 120)?,
        })?);
        let state = Arc::new(PrimaryAppState {
            mem0,
            authenticator: authenticator.clone(),
        });
        let listeners = server::bind(bind, health_bind).await?;
        info!(
            %bind,
            authentication = authenticator.mode_name(),
            backend_mode = "mem0_primary",
            vector_store = "elasticsearch",
            "Mem0-primary Memory Service listening"
        );
        server::serve(
            listeners,
            primary_router(Arc::clone(&state)),
            primary_health_router(state),
            shutdown_signal(),
        )
        .await?;
        return Ok(());
    }
    if backend_mode != "governed" {
        return Err("MEMORY_BACKEND_MODE must be governed or mem0_primary".into());
    }
    let projection_model_manifest = projection_model_manifest_from_env()?;
    let embedding_probe = Arc::new(EmbeddingProbe::from_env(projection_model_manifest.clone())?);
    if authenticator.is_demo() && !bind.ip().is_loopback() {
        return Err("MEMORY_AUTH_MODE=demo may only bind to a loopback address".into());
    }

    let repository = MemoryRepository::connect(&database_url).await?;
    if env::var("AUTO_MIGRATE").map_or(true, |value| value != "false") {
        repository.migrate().await?;
    }
    let index: Arc<dyn RetrievalIndex> = match projection_model_manifest.index_provider.as_str() {
        "mem0" => Arc::new(Mem0Adapter::new(Mem0Config {
            base_url: env::var("MEM0_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:18888".into()),
            api_key: env::var("MEM0_API_KEY").ok(),
            timeout: duration_from_env("MEMORY_INDEX_TIMEOUT_SECONDS", 5)?,
        })?),
        _ => {
            return Err(
                "MEMORY_INDEX_PROVIDER must be mem0; direct ElasticsearchAdapter wiring is disabled"
                    .into(),
            );
        }
    };
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
        "openai_compatible" => Some(Arc::new(OpenAiCompatibleExtractor::new_openai_compatible(
            OpenAiCompatibleExtractorConfig::from_env_or_dotenv()?,
        )?)),
        _ => {
            return Err("MEMORY_EXTRACT_MODE must be disabled, kimi, or openai_compatible".into());
        }
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
    let dynamic_tenant_workers = authenticator.allows_dynamic_tenants().then(|| {
        DynamicTenantWorkers::new(
            repository.clone(),
            Arc::clone(&index),
            extractor.clone(),
            projection_model_manifest.clone(),
        )
    });
    let state = Arc::new(AppState {
        repository: repository.clone(),
        index: Arc::clone(&index),
        reranker: Arc::clone(&reranker),
        extractor: extractor.clone(),
        embedding_probe,
        projection_model_manifest: projection_model_manifest.clone(),
        readiness_authorities: worker_authorities.clone(),
        dynamic_tenant_workers,
        authenticator: authenticator.clone(),
    });

    let listeners = server::bind(bind, health_bind).await?;
    let mut worker_handles = Vec::new();
    for worker_authority in worker_authorities {
        let projection_worker = ProjectionWorker::new(
            repository.clone(),
            Arc::clone(&index),
            worker_authority.clone(),
            projection_model_manifest.index_provider.clone(),
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
    info!(
        %bind,
        authentication = authenticator.mode_name(),
        index_provider = projection_model_manifest.index_provider,
        embedding_model = projection_model_manifest.embedding_model,
        embedding_revision = projection_model_manifest.embedding_revision,
        semantic_embedding = projection_model_manifest.semantic_embedding,
        reranker = reranker.version(),
        extractor = extractor
            .as_ref()
            .map_or("disabled", |extractor| extractor.version()),
        "LazyTT Memory Service listening"
    );
    let result = server::serve(
        listeners,
        router(Arc::clone(&state)),
        health_router(state),
        shutdown_signal(),
    )
    .await;
    for worker_handle in worker_handles {
        worker_handle.abort();
    }
    result?;
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

fn duration_from_env(
    name: &str,
    default_seconds: u64,
) -> Result<Duration, Box<dyn std::error::Error>> {
    let seconds = env::var(name)
        .unwrap_or_else(|_| default_seconds.to_string())
        .parse::<u64>()?;
    if !(1..=120).contains(&seconds) {
        return Err(format!("{name} must be between 1 and 120").into());
    }
    Ok(Duration::from_secs(seconds))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
