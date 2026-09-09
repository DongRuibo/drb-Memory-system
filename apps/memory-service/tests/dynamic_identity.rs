use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde_json::{Value, json};
use uuid::Uuid;

type Rows = Arc<Mutex<Vec<Value>>>;

async fn infer(State(rows): State<Rows>, Json(body): Json<Value>) -> Json<Value> {
    let memory = json!({
        "id": Uuid::new_v4(),
        "user_id": body["user_id"],
        "memory": body["messages"][0]["content"],
    });
    rows.lock().unwrap().push(memory.clone());
    Json(json!({"results": [memory]}))
}

async fn search(State(rows): State<Rows>, Json(body): Json<Value>) -> Json<Value> {
    let found: Vec<_> = rows
        .lock()
        .unwrap()
        .iter()
        .filter(|row| row["user_id"] == body["filters"]["user_id"])
        .cloned()
        .collect();
    Json(json!({"results": found}))
}

struct Service(Child);

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn primary_dynamic_identities_start_without_registration_and_remain_isolated() {
    // Only the HTTP provider contract is simulated: no database, ES, or model calls.
    let rows = Rows::default();
    let provider = Router::new()
        .route(
            "/auth/setup-status",
            get(|| async { Json(json!({"configured": true})) }),
        )
        .route("/memories", post(infer))
        .route("/search", post(search))
        .with_state(rows.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_url = format!("http://{}", listener.local_addr().unwrap());
    let provider_task = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });

    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reserved.local_addr().unwrap();
    drop(reserved);
    let base = format!("http://{address}");
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_memory-service"))
            .env_clear()
            .env("MEMORY_BIND", address.to_string())
            .env("MEMORY_BACKEND_MODE", "mem0_primary")
            .env("MEMORY_AUTH_MODE", "trusted_user_id")
            .env("MEMORY_ALLOW_DYNAMIC_TENANT", "true")
            .env("MEM0_BASE_URL", provider_url)
            .current_dir(std::env::temp_dir())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let root = loop {
        assert!(
            service.0.try_wait().unwrap().is_none(),
            "Memory exited before listening"
        );
        if let Ok(response) = client.get(&base).send().await {
            if response.status().is_success() {
                break response.json::<Value>().await.unwrap();
            }
        }
        assert!(Instant::now() < deadline, "Memory did not start");
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(root["authentication"], "trusted_user_id_dynamic_tenant");
    for path in ["/health/live", "/health/ready"] {
        assert_eq!(
            client
                .get(format!("{base}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
    }

    let identity = |request: reqwest::RequestBuilder, tenant: &str, user: &str| {
        request
            .header("X-Tenant-Id", tenant)
            .header("X-User-Id", user)
    };
    let retrieve = format!("{base}/v1/runtime/memory/retrieve");
    let query = json!({"query": "What is my preference?", "limit": 5});
    let missing = client.post(&retrieve).json(&query).send().await.unwrap();
    assert_eq!(missing.status(), 400);
    let error = missing.json::<Value>().await.unwrap();
    assert_eq!(error["code"], "INVALID_TENANT_ID");
    assert!(
        !error["detail"]
            .as_str()
            .unwrap()
            .contains("MEMORY_TENANT_IDS")
    );
    let oversized = "x".repeat(257);
    for (tenant, user) in [
        ("", "alice"),
        ("new-tenant", ""),
        (oversized.as_str(), "alice"),
        ("new-tenant", oversized.as_str()),
    ] {
        let response = identity(client.post(&retrieve), tenant, user)
            .json(&query)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
    }
    let mut headers = reqwest::header::HeaderMap::new();
    headers.append("X-Tenant-Id", "new-tenant".parse().unwrap());
    headers.append("X-Tenant-Id", "another-tenant".parse().unwrap());
    headers.insert("X-User-Id", "alice".parse().unwrap());
    assert_eq!(
        client
            .post(&retrieve)
            .headers(headers)
            .json(&query)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert!(rows.lock().unwrap().is_empty());

    let tenant_a = format!("tenant-{}", Uuid::new_v4());
    let tenant_b = format!("tenant-{}", Uuid::new_v4());
    let identities = [
        (tenant_a.as_str(), "alice"),
        (tenant_a.as_str(), "bob"),
        (tenant_b.as_str(), "alice"),
    ];
    let mut ids = Vec::new();
    for (index, (tenant, user)) in identities.iter().enumerate() {
        let body = json!({"session_id": Uuid::new_v4(), "turn_id": Uuid::new_v4(),
            "user_content": format!("My preferred test color is shade {index}."), "assistant_content": ""});
        let response = identity(
            client.post(format!("{base}/v1/runtime/memory/extract")),
            tenant,
            user,
        )
        .header("Idempotency-Key", Uuid::new_v4().to_string())
        .json(&body)
        .send()
        .await
        .unwrap();
        assert_eq!(response.status(), 202);
        let body = response.json::<Value>().await.unwrap();
        assert_eq!(body["tenant_id"], *tenant);
        assert_eq!(body["user_id"], *user);
        assert_eq!(body["memories"].as_array().unwrap().len(), 1);
        ids.push(body["memories"][0]["id"].as_str().unwrap().to_owned());
    }
    for (index, (tenant, user)) in identities.iter().enumerate() {
        let response = identity(client.post(&retrieve), tenant, user)
            .json(&query)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.json::<Value>().await.unwrap();
        assert_eq!(body["memories"].as_array().unwrap().len(), 1);
        assert_eq!(body["memories"][0]["id"], ids[index]);
    }
    let fresh = identity(
        client.post(&retrieve),
        "never-registered",
        "never-registered",
    )
    .json(&query)
    .send()
    .await
    .unwrap()
    .json::<Value>()
    .await
    .unwrap();
    assert!(fresh["memories"].as_array().unwrap().is_empty());
    let denied = identity(
        client.delete(format!("{base}/v1/runtime/memory/{}", ids[0])),
        &tenant_a,
        "alice",
    )
    .header("X-Permissions", "memory:*")
    .send()
    .await
    .unwrap();
    assert_eq!(denied.status(), 403);
    assert_eq!(rows.lock().unwrap().len(), 3);
    provider_task.abort();
}
