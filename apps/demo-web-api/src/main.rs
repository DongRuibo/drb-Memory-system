mod orchestrator;
mod store;

use std::collections::VecDeque;
use std::convert::Infallible;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use demo_agent::{DemoConfig, KimiClient, memory_client_from_config};
use futures_util::stream::{self, Stream};
use memory_client::{
    CorrectionCommand, ErasureCommand, ErasureRequestView, ForgetCommand, MemoryPort,
};
use orchestrator::{ChatRuntime, MemoryAction, TurnWork};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use store::{ChatStore, CreateTurnCommand, StoreError};
use tokio::net::TcpListener;
use tokio::time::{Instant, sleep};
use tower_http::cors::CorsLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    runtime: Arc<ChatRuntime>,
    legacy_demo_running: Arc<AtomicBool>,
}

#[derive(Serialize)]
struct ReadinessResponse {
    ready: bool,
    memory_service: &'static str,
    chat_store: &'static str,
    model_configuration: &'static str,
    detail: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let bind: SocketAddr = env::var("DEMO_WEB_API_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8090".into())
        .parse()?;
    if !bind.ip().is_loopback() {
        return Err("Demo Web API must bind to a loopback address".into());
    }
    let config = DemoConfig::from_env()?;
    let memory = Arc::new(memory_client_from_config(&config)?);
    let model = Arc::new(KimiClient::new(config.kimi.clone())?);
    let tenant_id = env::var("DEMO_TENANT_ID").unwrap_or_else(|_| "demo-tenant".into());
    let project_id =
        env::var("DEMO_PROJECT_ID").unwrap_or_else(|_| "lazytt-memory-system-demo".into());
    if project_id.trim().is_empty()
        || project_id.len() > 256
        || project_id.chars().any(char::is_control)
    {
        return Err("DEMO_PROJECT_ID must contain 1 to 256 visible characters".into());
    }
    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://memory:memory-local-only@127.0.0.1:55432/memory".into());
    let store =
        ChatStore::connect_and_migrate(&database_url, tenant_id, config.user_id.clone()).await?;
    let runtime = Arc::new(ChatRuntime {
        store,
        memory,
        model,
        user_id: config.user_id,
        project_id,
    });
    let state = AppState {
        runtime,
        legacy_demo_running: Arc::new(AtomicBool::new(false)),
    };

    let cors = CorsLayer::new()
        .allow_origin(allowed_origins()?)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([
            header::CONTENT_TYPE,
            HeaderName::from_static("last-event-id"),
        ]);
    let app = Router::new()
        .route("/api/health", get(readiness))
        .route("/api/demo/run", post(run_demo))
        .route("/api/demo/reset", post(reset_demo))
        .route(
            "/api/chat/sessions",
            get(list_sessions).post(create_session),
        )
        .route("/api/chat/sessions/{session_id}", get(get_session))
        .route("/api/chat/turns", post(create_turn))
        .route("/api/chat/turns/{turn_id}", get(get_turn))
        .route("/api/chat/turns/{turn_id}/events", get(turn_events))
        .route("/api/chat/turns/{turn_id}/cancel", post(cancel_turn))
        .route("/api/memories", get(list_memories))
        .route("/api/memories/{memory_record_id}", get(get_memory))
        .route(
            "/api/memories/{memory_record_id}/correct",
            post(correct_memory),
        )
        .route(
            "/api/memories/{memory_record_id}/forget",
            post(forget_memory),
        )
        .route("/api/memories/{memory_record_id}/erase", post(erase_memory))
        .layer(cors)
        .with_state(state);

    let listener = TcpListener::bind(bind).await?;
    info!(%bind, "LazyTT interactive Demo Web API listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn allowed_origins() -> Result<Vec<HeaderValue>, Box<dyn std::error::Error>> {
    let mut origins = vec![
        "http://localhost:3000".to_owned(),
        "http://127.0.0.1:3000".to_owned(),
    ];
    if let Ok(configured) = env::var("DEMO_WEB_ALLOWED_ORIGINS") {
        origins.extend(
            configured
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        );
    }
    origins.sort();
    origins.dedup();
    origins
        .into_iter()
        .map(|origin| Ok(HeaderValue::from_str(&origin)?))
        .collect()
}

async fn readiness(State(state): State<AppState>) -> Json<ReadinessResponse> {
    let result = async {
        state.runtime.memory.ready().await?;
        state.runtime.store.list_sessions().await.map_err(|error| {
            memory_client::MemoryClientError::Contract(format!("chat store failed: {error}"))
        })?;
        Ok::<(), memory_client::MemoryClientError>(())
    }
    .await;
    match result {
        Ok(()) => Json(ReadinessResponse {
            ready: true,
            memory_service: "ready",
            chat_store: "ready",
            model_configuration: "ready",
            detail: None,
        }),
        Err(error) => Json(ReadinessResponse {
            ready: false,
            memory_service: "check_required",
            chat_store: "check_required",
            model_configuration: "configured",
            detail: Some(error.to_string()),
        }),
    }
}

async fn run_demo(
    State(state): State<AppState>,
) -> Result<Json<demo_agent::DemoReport>, WebApiError> {
    let _permit = RunPermit::acquire(Arc::clone(&state.legacy_demo_running))?;
    let report = demo_agent::run_from_env()
        .await
        .map_err(WebApiError::upstream)?;
    Ok(Json(report))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateSessionRequest {
    #[serde(default = "default_session_title")]
    title: String,
    #[serde(default = "default_session_mode")]
    mode: String,
}

fn default_session_title() -> String {
    "新的对话".into()
}

fn default_session_mode() -> String {
    "guided".into()
}

async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Value>), WebApiError> {
    if request.title.trim().is_empty()
        || request.title.len() > 120
        || !matches!(request.mode.as_str(), "guided" | "free")
    {
        return Err(WebApiError::bad_request(
            "INVALID_SESSION",
            "title or mode is invalid",
        ));
    }
    let session = state
        .runtime
        .store
        .create_session(request.title.trim(), &request.mode)
        .await?;
    Ok((StatusCode::CREATED, Json(json!({"session": session}))))
}

async fn list_sessions(State(state): State<AppState>) -> Result<Json<Value>, WebApiError> {
    let sessions = state.runtime.store.list_sessions().await?;
    Ok(Json(json!({"sessions": sessions})))
}

async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<Value>, WebApiError> {
    let session = state.runtime.store.session(session_id).await?;
    Ok(Json(json!({"session": session})))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CreateTurnRequest {
    session_id: Uuid,
    message: String,
    client_message_id: String,
    idempotency_key: String,
    #[serde(default)]
    memory_action: Option<MemoryAction>,
}

async fn create_turn(
    State(state): State<AppState>,
    Json(request): Json<CreateTurnRequest>,
) -> Result<(StatusCode, Json<Value>), WebApiError> {
    if request.message.trim().is_empty()
        || request.message.len() > 16_384
        || !(1..=128).contains(&request.client_message_id.len())
        || !(16..=128).contains(&request.idempotency_key.len())
    {
        return Err(WebApiError::bad_request(
            "INVALID_TURN",
            "message or idempotency fields violate the chat contract",
        ));
    }
    if let Some(MemoryAction::Correct { new_content, .. }) = &request.memory_action
        && (new_content.trim().is_empty() || new_content.len() > 2_048)
    {
        return Err(WebApiError::bad_request(
            "INVALID_CORRECTION",
            "new_content must contain 1 to 2048 characters",
        ));
    }
    let request_hash = request_hash(&request)?;
    let created = state
        .runtime
        .store
        .create_turn(CreateTurnCommand {
            session_id: request.session_id,
            content: request.message.clone(),
            client_message_id: request.client_message_id,
            idempotency_key: request.idempotency_key,
            request_hash,
        })
        .await?;
    if created.created {
        Arc::clone(&state.runtime).spawn_turn(TurnWork {
            turn: created.turn.clone(),
            content: request.message,
            memory_action: request.memory_action,
        });
    }
    Ok((
        if created.created {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        },
        Json(json!({"turn": created.turn, "created": created.created})),
    ))
}

async fn get_turn(
    State(state): State<AppState>,
    Path(turn_id): Path<Uuid>,
) -> Result<Json<Value>, WebApiError> {
    Ok(Json(
        json!({"turn": state.runtime.store.turn(turn_id).await?}),
    ))
}

#[derive(Deserialize)]
struct EventsQuery {
    #[serde(default)]
    after: i64,
}

async fn turn_events(
    State(state): State<AppState>,
    Path(turn_id): Path<Uuid>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, WebApiError> {
    state.runtime.store.turn(turn_id).await?;
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let initial = EventStreamState {
        store: state.runtime.store.clone(),
        turn_id,
        cursor: query.after.max(last_event_id).max(0),
        queued: VecDeque::new(),
    };
    let stream = stream::unfold(initial, |mut state| async move {
        loop {
            if let Some(event) = state.queued.pop_front() {
                state.cursor = event.sequence_no;
                let data = serde_json::to_string(&json!({
                    "sequence_no": event.sequence_no,
                    "type": event.event_type,
                    "payload": event.payload,
                    "created_at": event.created_at,
                }))
                .expect("turn event JSON encoding cannot fail");
                let sse = Event::default()
                    .id(event.sequence_no.to_string())
                    .event(event.event_type)
                    .data(data);
                return Some((Ok(sse), state));
            }
            match state.store.events_after(state.turn_id, state.cursor).await {
                Ok((events, status)) => {
                    state.queued.extend(events);
                    if state.queued.is_empty()
                        && matches!(status.as_str(), "completed" | "failed" | "cancelled")
                    {
                        return None;
                    }
                }
                Err(_) => return None,
            }
            if state.queued.is_empty() {
                sleep(Duration::from_millis(250)).await;
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    ))
}

struct EventStreamState {
    store: ChatStore,
    turn_id: Uuid,
    cursor: i64,
    queued: VecDeque<store::TurnEventView>,
}

async fn cancel_turn(
    State(state): State<AppState>,
    Path(turn_id): Path<Uuid>,
) -> Result<Json<Value>, WebApiError> {
    let turn = state.runtime.store.request_cancel(turn_id).await?;
    Ok(Json(json!({"turn": turn})))
}

async fn list_memories(State(state): State<AppState>) -> Result<Json<Value>, WebApiError> {
    let memories = state.runtime.memory.export_memories().await?;
    Ok(Json(json!({"memories": memories})))
}

async fn get_memory(
    State(state): State<AppState>,
    Path(memory_record_id): Path<Uuid>,
) -> Result<Json<Value>, WebApiError> {
    let memory = state
        .runtime
        .memory
        .inspect_memory(memory_record_id)
        .await?;
    Ok(Json(json!({"memory": memory})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorrectMemoryRequest {
    new_content: String,
}

async fn correct_memory(
    State(state): State<AppState>,
    Path(memory_record_id): Path<Uuid>,
    Json(request): Json<CorrectMemoryRequest>,
) -> Result<(StatusCode, Json<Value>), WebApiError> {
    if request.new_content.trim().is_empty() || request.new_content.len() > 2_048 {
        return Err(WebApiError::bad_request(
            "INVALID_CORRECTION",
            "new_content must contain 1 to 2048 characters",
        ));
    }
    let current = state
        .runtime
        .memory
        .inspect_memory(memory_record_id)
        .await?;
    let operation = state
        .runtime
        .memory
        .correct(&CorrectionCommand {
            memory_record_id,
            idempotency_key: format!("web-correct-{}", Uuid::new_v4()),
            expected_revision: current.revision,
            new_content: request.new_content,
            reason: "User corrected memory from memory panel".into(),
            source_ref: format!("memory-panel:{memory_record_id}:{}", Uuid::new_v4()),
            effective_from: Some(chrono::Utc::now()),
        })
        .await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": operation}))))
}

async fn forget_memory(
    State(state): State<AppState>,
    Path(memory_record_id): Path<Uuid>,
) -> Result<(StatusCode, Json<Value>), WebApiError> {
    let current = state
        .runtime
        .memory
        .inspect_memory(memory_record_id)
        .await?;
    let operation = state
        .runtime
        .memory
        .forget(&ForgetCommand {
            memory_record_id,
            idempotency_key: format!("web-forget-{}", Uuid::new_v4()),
            expected_revision: current.revision,
            reason: "User requested Forget from memory panel".into(),
            request_erasure: false,
        })
        .await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": operation}))))
}

async fn erase_memory(
    State(state): State<AppState>,
    Path(memory_record_id): Path<Uuid>,
) -> Result<(StatusCode, Json<Value>), WebApiError> {
    let erasure = state
        .runtime
        .memory
        .create_erasure_request(&ErasureCommand {
            idempotency_key: format!("web-erase-{}", Uuid::new_v4()),
            target_kind: "memory_record".into(),
            target_id: memory_record_id.to_string(),
            reason: "User requested physical Erase from memory panel".into(),
        })
        .await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"erasure": erasure}))))
}

async fn reset_demo(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<Value>), WebApiError> {
    let targets = [
        ("project", state.runtime.project_id.as_str()),
        ("user", state.runtime.store.user_id()),
    ];
    let mut erasures = Vec::with_capacity(targets.len());
    for (target_kind, target_id) in targets {
        let erasure = state
            .runtime
            .memory
            .create_erasure_request(&ErasureCommand {
                idempotency_key: format!("demo-reset-{}", Uuid::new_v4()),
                target_kind: target_kind.into(),
                target_id: target_id.to_owned(),
                reason: format!("Reset the interactive demo {target_kind} scope"),
            })
            .await?;
        erasures.push(wait_for_erasure(&state, erasure).await?);
    }
    state.runtime.store.reset_sessions().await?;
    let session = state
        .runtime
        .store
        .create_session("记忆演示", "guided")
        .await?;
    Ok((
        StatusCode::OK,
        Json(json!({"session": session, "erasures": erasures})),
    ))
}

async fn wait_for_erasure(
    state: &AppState,
    mut current: ErasureRequestView,
) -> Result<ErasureRequestView, WebApiError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while current.status != "complete" {
        if current.status == "failed" {
            return Err(WebApiError::upstream_detail(
                "ERASURE_FAILED",
                "demo memory Erase failed",
            ));
        }
        if Instant::now() >= deadline {
            return Err(WebApiError::upstream_detail(
                "ERASURE_TIMEOUT",
                "demo reset did not finish physical Erase within 30 seconds",
            ));
        }
        sleep(Duration::from_millis(250)).await;
        current = state
            .runtime
            .memory
            .erasure_request(current.erasure_request_id)
            .await?;
    }
    Ok(current)
}

fn request_hash<T: Serialize>(value: &T) -> Result<String, WebApiError> {
    let encoded = serde_json::to_vec(value).map_err(|error| WebApiError {
        status: StatusCode::BAD_REQUEST,
        code: "INVALID_JSON",
        detail: error.to_string(),
    })?;
    Ok(format!(
        "sha256:{}",
        hex::encode(Sha256::digest(encoded.as_slice()))
    ))
}

struct RunPermit {
    running: Arc<AtomicBool>,
}

impl RunPermit {
    fn acquire(running: Arc<AtomicBool>) -> Result<Self, WebApiError> {
        running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| WebApiError {
                status: StatusCode::CONFLICT,
                code: "DEMO_ALREADY_RUNNING",
                detail: "A legacy scripted demo is already running".into(),
            })?;
        Ok(Self { running })
    }
}

impl Drop for RunPermit {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
    }
}

struct WebApiError {
    status: StatusCode,
    code: &'static str,
    detail: String,
}

impl WebApiError {
    fn bad_request(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            detail: detail.into(),
        }
    }

    fn upstream(error: impl std::fmt::Display) -> Self {
        Self::upstream_detail("UPSTREAM_FAILED", error.to_string())
    }

    fn upstream_detail(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code,
            detail: detail.into(),
        }
    }
}

impl From<StoreError> for WebApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "NOT_FOUND",
                detail: "chat resource was not found".into(),
            },
            StoreError::Conflict(detail) => Self {
                status: StatusCode::CONFLICT,
                code: "CHAT_CONFLICT",
                detail,
            },
            StoreError::Database(error) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "CHAT_STORE_FAILED",
                detail: error.to_string(),
            },
            StoreError::Migration(error) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "CHAT_MIGRATION_FAILED",
                detail: error.to_string(),
            },
        }
    }
}

impl From<memory_client::MemoryClientError> for WebApiError {
    fn from(error: memory_client::MemoryClientError) -> Self {
        Self::upstream(error)
    }
}

impl IntoResponse for WebApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "code": self.code,
                "detail": self.detail,
            })),
        )
            .into_response()
    }
}
