use std::io::{self, BufRead, Write};
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use memory_application::{ChineseTokenizer, MemoryReranker, ProjectionHit, RerankCandidate, RerankRequest,
    TransparentReranker, weighted_reciprocal_rank_fusion};
use memory_postgres::*;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
static EVAL_TOKENIZER: LazyLock<ChineseTokenizer> = LazyLock::new(ChineseTokenizer::default);

#[derive(Deserialize)]
struct Request {
    action: String,
    tenant: String,
    #[serde(default = "user")]
    user: String,
    #[serde(default)]
    projects: Vec<String>,
    #[serde(default)]
    data: Value,
}

fn user() -> String { "evaluator".into() }
fn string<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().ok_or_else(|| format!("missing string: {k}").into())
}
fn id(v: &Value, k: &str) -> Result<Uuid> { Ok(Uuid::parse_str(string(v, k)?)?) }

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("EVAL_DATABASE_URL")?;
    // 仅接受 Runner 创建的一次性数据库，绝不读取生产 DATABASE_URL。
    if !url.starts_with("postgres://drb_eval@127.0.0.1:") || !url.contains("/drb_eval_") {
        return Err("only a disposable drb_eval loopback database is allowed".into());
    }
    let repo = MemoryRepository::connect(&url).await?;
    repo.migrate().await?;
    let pool = PgPool::connect(&url).await?;
    println!("{}", json!({"ready":true}));
    for line in io::stdin().lock().lines() {
        let request: Request = serde_json::from_str(&line?)?;
        let mut authority = DemoAuthority::user(&request.tenant, &request.user);
        authority.project_ids.extend(request.projects);
        let result = execute(&repo, &pool, &authority, &request.action, &request.data).await;
        match result {
            Ok(value) => println!("{}", json!({"ok":value})),
            Err(error) => println!("{}", json!({"error":error.to_string()})),
        }
        io::stdout().flush()?;
    }
    Ok(())
}

async fn execute(repo: &MemoryRepository, pool: &PgPool, a: &DemoAuthority,
                 action: &str, d: &Value) -> Result<Value> {
    let now: DateTime<Utc> = "2026-01-01T00:00:00Z".parse()?;
    let generation = repo.current_projection_generation(a).await?;
    match action {
        "sql_probe" => {
            // 冻结 SQL 只在临时评测库执行，用相同绑定参数比较执行计划及分值。
            let sql = match string(d,"variant")? {
                "before" => include_str!("../../evidence/phase2/bm25-before.sql"),
                "materialized" | "materialized_uncached" => include_str!("../../evidence/phase2/bm25-materialized.sql"),
                "lateral" => include_str!("../../evidence/phase2/bm25-lateral.sql"),
                "version_lengths" => include_str!("../../evidence/phase2/bm25-version-lengths.sql"),
                _ => return Err("invalid SQL variant".into()),
            };
            let uncached=matches!(d["variant"].as_str(),Some("materialized_uncached"|"lateral"));
            let sql=if uncached {format!("{sql} /* eval_uncached */")} else {sql.to_owned()};
            let tokenize_started=std::time::Instant::now();
            let mut terms=EVAL_TOKENIZER.tokenize(string(d,"query")?);
            terms.sort(); terms.dedup();
            let tokenize_ms=tokenize_started.elapsed().as_secs_f64()*1000.0;
            let namespace=authorized_namespace_key(a,"user",Some(&a.user_id)).ok_or("unauthorized")?;
            let mut tx=pool.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)").bind(&a.tenant_id).execute(&mut *tx).await?;
            let mode=d["plan_mode"].as_str().unwrap_or("auto");
            if !["auto","force_generic_plan","force_custom_plan"].contains(&mode) {return Err("invalid plan mode".into());}
            sqlx::query("SELECT set_config('plan_cache_mode',$1,true)").bind(mode).execute(&mut *tx).await?;
            let started=std::time::Instant::now();
            if d["explain"].as_bool().unwrap_or(false) {
                if d["plan_timeout_ms"].as_u64()==Some(10000) {
                    sqlx::query("SET LOCAL statement_timeout='10000ms'").execute(&mut *tx).await?;
                }
                // 显式 PREPARE/EXECUTE，确保记录的是参数化语句实际选用的 generic/custom 计划。
                let statement=format!("eval_{}",Uuid::new_v4().simple());
                sqlx::query(&format!("PREPARE {statement}(text,text,text,text[],bigint) AS {sql}"))
                    .execute(&mut *tx).await?;
                let quote=|s:&str| format!("'{}'",s.replace('\'',"''"));
                let array=format!("ARRAY[{}]::text[]",terms.iter().map(|s|quote(s)).collect::<Vec<_>>().join(","));
                let options=if d["analyze"].as_bool().unwrap_or(true) {"ANALYZE,BUFFERS,FORMAT JSON"} else {"FORMAT JSON"};
                let plan:Value=sqlx::query_scalar(&format!("EXPLAIN ({options}) EXECUTE {statement}({},{},{},{array},20)",
                    quote(&a.tenant_id),quote(&namespace),quote(&generation)))
                    .fetch_one(&mut *tx).await?;
                sqlx::query(&format!("DEALLOCATE {statement}")).execute(&mut *tx).await?;
                tx.commit().await?;
                return Ok(json!({"plan":plan,"sql_ms":started.elapsed().as_secs_f64()*1000.0,"terms":terms,"tokenize_ms":tokenize_ms}));
            }
            let result=sqlx::query(&sql).persistent(!uncached)
                .bind(&a.tenant_id).bind(&namespace).bind(&generation).bind(&terms).bind(20_i64)
                .fetch_all(&mut *tx).await;
            let sql_ms=started.elapsed().as_secs_f64()*1000.0;
            match result {
                Ok(rows) => {
                    let rows=rows.iter().map(|r| Ok(json!({
                        "memory_record_id":r.try_get::<Uuid,_>("memory_record_id")?,
                        "memory_version_id":r.try_get::<Uuid,_>("memory_version_id")?,
                        "bm25_score":r.try_get::<f64,_>("bm25_score")?,
                    }))).collect::<Result<Vec<_>>>()?;
                    tx.commit().await?; Ok(json!({"rows":rows,"sql_ms":sql_ms,"terms":terms,"tokenize_ms":tokenize_ms}))
                },
                Err(error) if error.to_string().contains("statement timeout") => Ok(json!({"rows":[],"sql_ms":sql_ms,"error":"db_statement_timeout","terms":terms,"tokenize_ms":tokenize_ms})),
                Err(error) => Err(error.into()),
            }
        }
        "terms" => {
            let mut tx=pool.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)").bind(&a.tenant_id).execute(&mut *tx).await?;
            let rows:Value=sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(t)),'[]'::jsonb) FROM memory_search.retrieval_terms t WHERE tenant_id=$1")
                .bind(&a.tenant_id).fetch_one(&mut *tx).await?;
            tx.commit().await?; Ok(rows)
        }
        "namespace" => Ok(json!(authorized_namespace_key(a,"user",Some(&a.user_id)))),
        "ingest" => {
            let content = string(d, "content")?.to_owned();
            let key = string(d, "key")?;
            let scope = d["scope"].as_str().unwrap_or("user");
            let scope_id = if scope == "project" {string(d, "scope_id")?} else {&a.user_id};
            let result = repo.ingest_extracted(a, IngestCandidateCommand {
                idempotency_key: key.into(), event_id: key.into(),
                source_type: "conversation_message".into(), source_ref: format!("eval://{key}"),
                source_content: d["source"].as_str().unwrap_or(&content).into(),
                memory_content: content,
                occurred_at: d["occurred_at"].as_str().map(str::parse).transpose()?.unwrap_or(now), event_time: None,
                structured_content: None,
                memory_type: d["memory_type"].as_str().unwrap_or("episodic_summary").into(),
                semantic_key: string(d, "semantic_key")?.into(), confidence: 0.99,
                importance: 0.5, inference_kind: "explicit".into(),
                extractor_name: "eval-adapter".into(), extractor_version: "v1".into(),
                scope_kind: scope.into(), scope_id: Some(scope_id.into()),
            }).await?;
            Ok(serde_json::to_value(result)?)
        }
        "drain" => {
            // 只确认本地投影，不伪装成真实 Mem0/ES/模型调用。
            let worker = DemoAuthority::worker(&a.tenant_id);
            let mut count = 0;
            while let Some(work) = repo.claim_projection(&worker, "eval-local-projector").await? {
                let live = work.expected_head_revision == work.current_head_revision
                    && work.current_head_state == "active"
                    && work.memory_version_id == work.current_version_id;
                match work.operation_type {
                    ProjectionOperationType::Upsert if live => repo.complete_ready(
                        &worker, &work, &format!("eval-local:{}", work.operation_id)).await?,
                    ProjectionOperationType::Upsert => repo.complete_stale(&worker, &work, "EVAL_STALE_HEAD").await?,
                    ProjectionOperationType::Delete => repo.complete_deleted(&worker, &work).await?,
                }
                count += 1;
                if count > 10000 { return Err("projection drain budget exceeded".into()); }
            }
            Ok(json!({"processed":count}))
        }
        "search" => {
            let query = string(d, "query")?;
            let limit = d["budget"].as_u64().unwrap_or(20) as usize;
            let scope = d["scope"].as_str().unwrap_or("user");
            let scope_id = if scope == "project" {string(d,"scope_id")?} else {&a.user_id};
            let bm25 = repo.search_bm25_projection(a,scope,Some(scope_id),query,&generation,limit).await?;
            Ok(serde_json::to_value(bm25)?)
        }
        "rank" => {
            let method = string(d, "method")?;
            let bm25: Vec<ProjectionHit> = serde_json::from_value(d["bm25"].clone())?;
            let vector: Vec<ProjectionHit> = serde_json::from_value(d["vector"].clone())?;
            let budget = d["budget"].as_u64().unwrap_or(20) as usize;
            let mut hits = match method {
                "bm25" => bm25,
                "vector_tfidf" => vector,
                "hybrid" | "hybrid_rerank" => weighted_reciprocal_rank_fusion(
                    &[(1.15, bm25.as_slice()), (1.0, vector.as_slice())], budget),
                _ => return Err("unknown method".into()),
            };
            let authorized = repo.authorized_hits(a, &hits).await?;
            if method == "hybrid_rerank" || d["rerank_for_context"].as_bool().unwrap_or(false) {
                let ranks = TransparentReranker.rerank(RerankRequest {
                    query: string(d,"query")?.into(), limit: budget,
                    candidates: authorized.iter().map(|m| RerankCandidate {
                        memory_version_id:m.memory_version_id,content:m.content.clone(),
                        memory_type:m.memory_type.clone(),retrieval_score:m.score,
                    }).collect(),
                }).await?;
                hits = ranks.iter().filter_map(|r| hits.iter()
                    .find(|h| h.memory_version_id == r.memory_version_id).map(|h| {
                        let mut h=h.clone(); h.provider_score=r.relevance_score; h
                    })).collect();
            } else {
                hits.retain(|h| authorized.iter().any(|m| m.memory_version_id==h.memory_version_id));
            }
            hits.truncate(10);
            let memories = repo.authorized_hits(a, &hits).await?;
            Ok(json!({"hits":hits,"memories":memories}))
        }
        "context" => {
            let hits: Vec<ProjectionHit> = serde_json::from_value(d["hits"].clone())?;
            let attempt = Uuid::new_v4();
            let task = Uuid::new_v4();
            let pack = repo.compile_context_pack(a, &ContextPackCommand {
                task_id:task,attempt_id:attempt,purpose:"answer_user".into(),
                query:string(d,"query")?.into(),limit:10,
                memory_types:vec!["preference".into(),"profile_fact".into(),"semantic_fact".into(),"episodic_summary".into()],
                ranking_version:"eval-v1".into(),projection_generation:generation,
                token_budget:d["token_budget"].as_i64().unwrap_or(4096) as i32,
                tokenizer_id:DEMO_CONTEXT_TOKENIZER_ID.into(),memory_required:false,
                pinned_memory_version_ids:d["pinned_versions"].as_array().map(|xs| xs.iter().map(|v| {
                    Uuid::parse_str(v.as_str().unwrap_or("")).map_err(Into::into)
                }).collect::<Result<Vec<_>>>()).transpose()?.unwrap_or_default(),temporal_window_hash:None,
                reference_time:d["reference_time"].as_str().map(str::parse).transpose()?.unwrap_or(now),timezone:"UTC".into(),
            }, &hits).await?;
            let mut value=serde_json::to_value(repo.consume_context_pack(a,pack.context_pack_id,
                &ConsumeContextPackCommand {idempotency_key:format!("eval-consume-{attempt}"),
                    task_id:task,attempt_id:attempt,purpose:"answer_user".into()}).await?)?;
            value["omission_summary"]=serde_json::to_value(pack.omission_summary)?;
            Ok(value)
        }
        "correct" => Ok(serde_json::to_value(repo.correct(a,CorrectionCommand {
            idempotency_key:string(d,"key")?.into(), memory_record_id:id(d,"record")?,
            expected_revision:d["revision"].as_i64().ok_or("missing revision")?,
            new_content:string(d,"content")?.into(),reason:"隔离评测事实更新".into(),
            source_ref:format!("eval://{}",string(d,"key")?),effective_from:Some(now),
            new_structured_content:None,
        }).await?)?),
        "forget" => Ok(serde_json::to_value(repo.forget(a,ForgetCommand {
            idempotency_key:string(d,"key")?.into(),memory_record_id:id(d,"record")?,
            expected_revision:d["revision"].as_i64().ok_or("missing revision")?,reason:"隔离评测遗忘".into(),
        }).await?)?),
        "review" => Ok(serde_json::to_value(repo.decide_candidate(a,ReviewDecisionCommand {
            idempotency_key:string(d,"key")?.into(),candidate_id:id(d,"candidate")?,
            expected_candidate_revision:0,outcome:ReviewOutcome::Accept,reason:"隔离评测人工确认".into(),
        }).await?)?),
        "erase" => Ok(serde_json::to_value(repo.create_erasure_request(a,ErasureCommand {
            idempotency_key:string(d,"key")?.into(),target_kind:ErasureTargetKind::MemoryRecord,
            target_id:string(d,"record")?.into(),reason:"隔离评测逻辑擦除".into(),
        }).await?)?),
        "inspect" => Ok(serde_json::to_value(repo.inspect_memory(a,id(d,"record")?).await?)?),
        "state" => {
            let mut tx=pool.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)").bind(&a.tenant_id).execute(&mut *tx).await?;
            let mut state=serde_json::Map::new();
            for table in ["memory_records","memory_versions","memory_heads","source_events","memory_candidates","memory_version_sources"] {
                let value: Value=sqlx::query_scalar(&format!(
                    "SELECT COALESCE(jsonb_agg(to_jsonb(t)),'[]'::jsonb) FROM memory_core.{table} t"))
                    .fetch_one(&mut *tx).await?;
                state.insert(table.into(),value);
            }
            let value: Value=sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(t)),'[]'::jsonb) FROM memory_search.retrieval_documents t")
                .fetch_one(&mut *tx).await?;
            state.insert("retrieval_documents".into(),value);
            tx.commit().await?;
            Ok(Value::Object(state))
        }
        _ => Err("unknown action".into()),
    }
}
