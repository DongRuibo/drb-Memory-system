use std::env;
use std::time::Duration;

use mem0_adapter::{Mem0Adapter, Mem0Config};
use memory_application::{ProjectionDelete, ProjectionSearch, ProjectionUpsert, RetrievalIndex};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let command = env::args().nth(1).unwrap_or_else(|| "health".into());
    let base_url = env::var("MEM0_BASE_URL").unwrap_or_else(|_| "http://localhost:18888".into());
    let api_key = env::var("MEM0_API_KEY").ok();
    let adapter = Mem0Adapter::new(Mem0Config {
        base_url,
        api_key,
        timeout: Duration::from_secs(10),
    })?;

    match command.as_str() {
        "health" => {
            adapter.health_check().await?;
            println!("Mem0 health check succeeded");
        }
        "cycle" => run_cycle(&adapter).await?,
        "persist-write" => run_persistence_write(&adapter).await?,
        "persist-verify" => run_persistence_verify(&adapter).await?,
        "duplicate-probe" => run_duplicate_probe(&adapter).await?,
        _ => {
            return Err(format!(
                "unknown command {command:?}; use health, cycle, persist-write, persist-verify, or duplicate-probe"
            )
            .into());
        }
    }

    Ok(())
}

const PERSIST_NAMESPACE: &str = "poc-opaque-persistence-tenant";
const PERSIST_GENERATION: &str = "poc-persistence-generation-1";

fn persistence_upsert() -> ProjectionUpsert {
    let content = "POC persistence marker: canonical ledger remains authoritative.";
    ProjectionUpsert {
        operation_id: Uuid::parse_str("10000000-0000-0000-0000-000000000001").unwrap(),
        namespace_key: PERSIST_NAMESPACE.into(),
        memory_record_id: Uuid::parse_str("10000000-0000-0000-0000-000000000002").unwrap(),
        memory_version_id: Uuid::parse_str("10000000-0000-0000-0000-000000000003").unwrap(),
        head_revision: 1,
        generation: PERSIST_GENERATION.into(),
        content: content.into(),
        content_hash: format!("sha256:{}", hex::encode(Sha256::digest(content.as_bytes()))),
    }
}

async fn search_persistence(
    adapter: &Mem0Adapter,
) -> Result<Vec<memory_application::ProjectionHit>, Box<dyn std::error::Error>> {
    Ok(adapter
        .search(ProjectionSearch {
            namespace_key: PERSIST_NAMESPACE.into(),
            query: "What remains authoritative?".into(),
            generation: PERSIST_GENERATION.into(),
            limit: 10,
        })
        .await?)
}

async fn run_persistence_write(adapter: &Mem0Adapter) -> Result<(), Box<dyn std::error::Error>> {
    let receipt = adapter.upsert(persistence_upsert()).await?;
    let provider_object_id = receipt
        .provider_object_id
        .ok_or("Mem0 did not return a provider object ID")?;
    println!("persistence marker ready: {provider_object_id}");
    Ok(())
}

async fn run_persistence_verify(adapter: &Mem0Adapter) -> Result<(), Box<dyn std::error::Error>> {
    let expected = persistence_upsert();
    let hits = search_persistence(adapter).await?;
    let hit = hits
        .into_iter()
        .find(|hit| {
            hit.memory_record_id == expected.memory_record_id
                && hit.memory_version_id == expected.memory_version_id
                && hit.content_hash == expected.content_hash
        })
        .ok_or("persistence marker was not searchable after restart")?;
    println!("persistence marker recovered: {}", hit.provider_object_id);

    adapter
        .delete(ProjectionDelete {
            operation_id: Uuid::parse_str("10000000-0000-0000-0000-000000000004")?,
            namespace_key: PERSIST_NAMESPACE.into(),
            provider_object_id: hit.provider_object_id,
            memory_record_id: expected.memory_record_id,
            head_revision: 2,
            generation: PERSIST_GENERATION.into(),
        })
        .await?;

    if search_persistence(adapter)
        .await?
        .iter()
        .any(|candidate| candidate.memory_record_id == expected.memory_record_id)
    {
        return Err("persistence marker remained searchable after cleanup".into());
    }
    println!("persistence marker cleanup confirmed");
    Ok(())
}

async fn run_duplicate_probe(adapter: &Mem0Adapter) -> Result<(), Box<dyn std::error::Error>> {
    let content = "POC duplicate operation marker.";
    let request = ProjectionUpsert {
        operation_id: Uuid::parse_str("20000000-0000-0000-0000-000000000001")?,
        namespace_key: "poc-opaque-duplicate-tenant".into(),
        memory_record_id: Uuid::parse_str("20000000-0000-0000-0000-000000000002")?,
        memory_version_id: Uuid::parse_str("20000000-0000-0000-0000-000000000003")?,
        head_revision: 1,
        generation: "poc-duplicate-generation-1".into(),
        content: content.into(),
        content_hash: format!("sha256:{}", hex::encode(Sha256::digest(content.as_bytes()))),
    };
    let first = adapter
        .upsert(request.clone())
        .await?
        .provider_object_id
        .ok_or("first duplicate probe did not return an ID")?;
    let second = adapter
        .upsert(request.clone())
        .await?
        .provider_object_id
        .ok_or("second duplicate probe did not return an ID")?;

    let hits = adapter
        .search(ProjectionSearch {
            namespace_key: request.namespace_key.clone(),
            query: "duplicate operation".into(),
            generation: request.generation.clone(),
            limit: 10,
        })
        .await?;
    let duplicate_count = hits
        .iter()
        .filter(|hit| {
            hit.memory_record_id == request.memory_record_id
                && hit.memory_version_id == request.memory_version_id
        })
        .count();
    println!(
        "same operation returned provider IDs first={first} second={second}; matching projections={duplicate_count}"
    );

    for provider_object_id in [first, second] {
        adapter
            .delete(ProjectionDelete {
                operation_id: Uuid::new_v4(),
                namespace_key: request.namespace_key.clone(),
                provider_object_id,
                memory_record_id: request.memory_record_id,
                head_revision: 2,
                generation: request.generation.clone(),
            })
            .await?;
    }
    if duplicate_count < 2 {
        return Err("duplicate probe expected two provider projections".into());
    }
    let after_cleanup = adapter
        .search(ProjectionSearch {
            namespace_key: request.namespace_key,
            query: "duplicate operation".into(),
            generation: request.generation,
            limit: 10,
        })
        .await?;
    if after_cleanup
        .iter()
        .any(|hit| hit.memory_record_id == request.memory_record_id)
    {
        return Err("duplicate probe cleanup left a searchable projection".into());
    }
    println!("duplicate probe confirms Mem0 has no operation-level idempotency");
    Ok(())
}

async fn run_cycle(adapter: &Mem0Adapter) -> Result<(), Box<dyn std::error::Error>> {
    let operation_id = Uuid::new_v4();
    let record_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let content = "POC user prefers principle-first technical explanations.";
    let content_hash = format!("sha256:{}", hex::encode(Sha256::digest(content.as_bytes())));
    let namespace_key = "poc-opaque-tenant-user-alice".to_owned();
    let generation = "poc-generation-1".to_owned();

    let receipt = adapter
        .upsert(ProjectionUpsert {
            operation_id,
            namespace_key: namespace_key.clone(),
            memory_record_id: record_id,
            memory_version_id: version_id,
            head_revision: 1,
            generation: generation.clone(),
            content: content.into(),
            content_hash: content_hash.clone(),
        })
        .await?;
    let provider_object_id = receipt
        .provider_object_id
        .ok_or("Mem0 did not return a provider object ID")?;
    println!("projection ready: {provider_object_id}");

    let hits = adapter
        .search(ProjectionSearch {
            namespace_key: namespace_key.clone(),
            query: "How should technical topics be explained?".into(),
            generation: generation.clone(),
            limit: 5,
        })
        .await?;
    println!("authorized mapping candidates returned: {}", hits.len());
    for hit in &hits {
        println!(
            "record={} version={} revision={} score={}",
            hit.memory_record_id, hit.memory_version_id, hit.head_revision, hit.provider_score
        );
    }
    let expected_hit = hits
        .iter()
        .find(|hit| hit.provider_object_id == provider_object_id)
        .ok_or("newly indexed projection was not returned by scoped search")?;
    if expected_hit.memory_record_id != record_id
        || expected_hit.memory_version_id != version_id
        || expected_hit.head_revision != 1
        || expected_hit.generation != generation
        || expected_hit.content_hash != content_hash
    {
        return Err("provider mapping fields do not match the canonical projection".into());
    }

    let wrong_namespace = adapter
        .search(ProjectionSearch {
            namespace_key: "poc-opaque-other-tenant-user-alice".into(),
            query: "How should technical topics be explained?".into(),
            generation: generation.clone(),
            limit: 5,
        })
        .await?;
    let wrong_generation = adapter
        .search(ProjectionSearch {
            namespace_key: namespace_key.clone(),
            query: "How should technical topics be explained?".into(),
            generation: "poc-generation-other".into(),
            limit: 5,
        })
        .await?;
    if wrong_namespace
        .iter()
        .chain(&wrong_generation)
        .any(|hit| hit.provider_object_id == provider_object_id)
    {
        return Err("projection escaped its namespace or generation filter".into());
    }
    println!("namespace and generation isolation confirmed");

    adapter
        .delete(ProjectionDelete {
            operation_id: Uuid::new_v4(),
            namespace_key: namespace_key.clone(),
            provider_object_id: provider_object_id.clone(),
            memory_record_id: record_id,
            head_revision: 2,
            generation: generation.clone(),
        })
        .await?;
    println!("projection deleted");

    let after_delete = adapter
        .search(ProjectionSearch {
            namespace_key,
            query: "How should technical topics be explained?".into(),
            generation,
            limit: 5,
        })
        .await?;
    if after_delete
        .iter()
        .any(|hit| hit.provider_object_id == provider_object_id)
    {
        return Err("deleted provider projection remained searchable".into());
    }
    println!("post-delete scoped search confirmed removal");

    Ok(())
}
