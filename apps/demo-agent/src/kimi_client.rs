use std::future::Future;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DemoError;
use crate::http_util::{normalize_kimi_base_url, require_success};
use crate::model::ModelUsage;

const KIMI_SERVICE: &str = "Kimi";

#[derive(Clone)]
pub struct KimiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub reasoning_effort: String,
    pub max_completion_tokens: u32,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct CompletionCommand {
    pub task_id: Uuid,
    pub system_prompt: String,
    pub user_payload: String,
}

#[derive(Debug, Clone)]
pub struct ModelAnswer {
    pub content: String,
    pub model: String,
    pub request_id: Option<String>,
    pub usage: ModelUsage,
}

#[async_trait]
pub trait ChatModel: Send + Sync {
    async fn complete(&self, command: &CompletionCommand) -> Result<ModelAnswer, DemoError>;
}

#[derive(Clone)]
pub struct KimiClient {
    base_url: String,
    model: String,
    reasoning_effort: String,
    max_completion_tokens: u32,
    client: reqwest::Client,
}

impl KimiClient {
    pub fn new(config: KimiConfig) -> Result<Self, DemoError> {
        let base_url = normalize_kimi_base_url(&config.base_url)?;
        if config.model != "k3" {
            return Err(DemoError::Config(
                "KIMI_MODEL must be k3 for this verified Kimi Code demo".into(),
            ));
        }

        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", config.api_key.trim()))
            .map_err(|_| {
                DemoError::Config("KIMI_API_KEY contains invalid header characters".into())
            })?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(Policy::none())
            .timeout(config.timeout)
            .user_agent(concat!(
                "lazytt-memory-demo-agent/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|source| DemoError::Client {
                service: KIMI_SERVICE,
                source,
            })?;

        Ok(Self {
            base_url,
            model: config.model,
            reasoning_effort: config.reasoning_effort,
            max_completion_tokens: config.max_completion_tokens,
            client,
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    pub async fn stream_to<F, Fut>(
        &self,
        command: &CompletionCommand,
        mut on_delta: F,
    ) -> Result<ModelAnswer, DemoError>
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = Result<(), DemoError>>,
    {
        let payload = ChatCompletionRequest {
            model: &self.model,
            messages: [
                ChatMessage {
                    role: "system",
                    content: &command.system_prompt,
                },
                ChatMessage {
                    role: "user",
                    content: &command.user_payload,
                },
            ],
            reasoning_effort: &self.reasoning_effort,
            max_completion_tokens: self.max_completion_tokens,
            prompt_cache_key: command.task_id.to_string(),
            stream: true,
        };
        let response = self
            .client
            .post(self.endpoint())
            .json(&payload)
            .send()
            .await
            .map_err(|source| DemoError::Transport {
                service: KIMI_SERVICE,
                source,
            })?;
        let header_request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let mut response = require_success(response, KIMI_SERVICE).await?;
        let mut buffered = Vec::<u8>::new();
        let mut answer = String::new();
        let mut response_id = None;
        let mut response_model = None;
        let mut finish_reason = None;
        let mut usage = ModelUsage {
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        };

        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|source| DemoError::Transport {
                service: KIMI_SERVICE,
                source,
            })?
        {
            buffered.extend_from_slice(&chunk);
            while let Some(newline) = buffered.iter().position(|byte| *byte == b'\n') {
                let mut line = buffered.drain(..=newline).collect::<Vec<_>>();
                while matches!(line.last(), Some(b'\n' | b'\r')) {
                    line.pop();
                }
                let line =
                    std::str::from_utf8(&line).map_err(|error| DemoError::InvalidResponse {
                        service: KIMI_SERVICE,
                        detail: format!("stream contained invalid UTF-8: {error}"),
                    })?;
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim_start();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let event: ChatCompletionChunk =
                    serde_json::from_str(data).map_err(|error| DemoError::InvalidResponse {
                        service: KIMI_SERVICE,
                        detail: format!("invalid stream event: {error}"),
                    })?;
                response_id = response_id.or(event.id);
                response_model = response_model.or(event.model);
                if let Some(event_usage) = event.usage {
                    usage = ModelUsage {
                        prompt_tokens: event_usage.prompt_tokens,
                        completion_tokens: event_usage.completion_tokens,
                        total_tokens: event_usage.total_tokens,
                    };
                }
                for choice in event.choices {
                    if let Some(reason) = choice.finish_reason {
                        finish_reason = Some(reason);
                    }
                    if let Some(content) = choice.delta.content.filter(|value| !value.is_empty()) {
                        answer.push_str(&content);
                        on_delta(content).await?;
                    }
                }
            }
        }
        if !buffered.iter().all(u8::is_ascii_whitespace) {
            return Err(DemoError::InvalidResponse {
                service: KIMI_SERVICE,
                detail: "stream ended in the middle of an event".into(),
            });
        }
        if finish_reason.as_deref() == Some("length") {
            return Err(DemoError::InvalidResponse {
                service: KIMI_SERVICE,
                detail: "answer was truncated at max_completion_tokens".into(),
            });
        }
        if answer.trim().is_empty() {
            return Err(DemoError::InvalidResponse {
                service: KIMI_SERVICE,
                detail: "stream contained no answer text".into(),
            });
        }
        Ok(ModelAnswer {
            content: answer,
            model: response_model.unwrap_or_else(|| self.model.clone()),
            request_id: header_request_id.or(response_id),
            usage,
        })
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 2],
    reasoning_effort: &'a str,
    max_completion_tokens: u32,
    prompt_cache_key: String,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    id: Option<String>,
    model: Option<String>,
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    id: Option<String>,
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChunkChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChunkChoice {
    #[serde(default)]
    delta: ChatDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ChatDelta {
    content: Option<String>,
}

#[async_trait]
impl ChatModel for KimiClient {
    async fn complete(&self, command: &CompletionCommand) -> Result<ModelAnswer, DemoError> {
        let payload = ChatCompletionRequest {
            model: &self.model,
            messages: [
                ChatMessage {
                    role: "system",
                    content: &command.system_prompt,
                },
                ChatMessage {
                    role: "user",
                    content: &command.user_payload,
                },
            ],
            reasoning_effort: &self.reasoning_effort,
            max_completion_tokens: self.max_completion_tokens,
            prompt_cache_key: command.task_id.to_string(),
            stream: false,
        };
        let response = self
            .client
            .post(self.endpoint())
            .json(&payload)
            .send()
            .await
            .map_err(|source| DemoError::Transport {
                service: KIMI_SERVICE,
                source,
            })?;
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let response = require_success(response, KIMI_SERVICE).await?;
        let response: ChatCompletionResponse =
            response
                .json()
                .await
                .map_err(|error| DemoError::InvalidResponse {
                    service: KIMI_SERVICE,
                    detail: error.to_string(),
                })?;
        let request_id = request_id.or_else(|| response.id.clone());
        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| DemoError::InvalidResponse {
                    service: KIMI_SERVICE,
                    detail: "choices was empty".into(),
                })?;
        if choice.finish_reason.as_deref() == Some("length") {
            return Err(DemoError::InvalidResponse {
                service: KIMI_SERVICE,
                detail: "answer was truncated at max_completion_tokens".into(),
            });
        }
        let content = choice
            .message
            .content
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| DemoError::InvalidResponse {
                service: KIMI_SERVICE,
                detail: "the first choice contained no answer text".into(),
            })?;
        let usage = response.usage.unwrap_or(ChatUsage {
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        });

        Ok(ModelAnswer {
            content,
            model: response.model.unwrap_or_else(|| self.model.clone()),
            request_id,
            usage: ModelUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{HeaderMap, Response};
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Clone)]
    struct Captured {
        requests: Arc<Mutex<Vec<Value>>>,
    }

    async fn completion_handler(
        State(state): State<Captured>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        assert_eq!(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-key")
        );
        assert!(
            headers
                .get("user-agent")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("lazytt-memory-demo-agent/"))
        );
        state.requests.lock().unwrap().push(body);
        Json(json!({
            "id": "chatcmpl-test",
            "model": "k3",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "reasoning_content": "test reasoning",
                    "content": "test answer"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 11,
                "completion_tokens": 22,
                "total_tokens": 33
            }
        }))
    }

    async fn stream_handler(
        State(state): State<Captured>,
        Json(body): Json<Value>,
    ) -> Response<Body> {
        state.requests.lock().unwrap().push(body);
        let events = concat!(
            "data: {\"id\":\"chatcmpl-stream\",\"model\":\"k3\",\"choices\":[{\"delta\":{\"content\":\"你\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"好\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
            "data: [DONE]\n\n"
        );
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(events))
            .unwrap()
    }

    #[tokio::test]
    async fn sends_the_verified_kimi_chat_completions_contract() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Captured {
            requests: Arc::clone(&requests),
        };
        let app = Router::new()
            .route("/coding/v1/chat/completions", post(completion_handler))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = KimiClient::new(KimiConfig {
            base_url: format!("http://{address}/coding/v1"),
            api_key: "test-key".into(),
            model: "k3".into(),
            reasoning_effort: "high".into(),
            max_completion_tokens: 2_048,
            timeout: Duration::from_secs(2),
        })
        .unwrap();
        let answer = client
            .complete(&CompletionCommand {
                task_id: Uuid::new_v4(),
                system_prompt: "system".into(),
                user_payload: "{\"question\":\"hello\",\"memory_context\":[]}".into(),
            })
            .await
            .unwrap();
        server.abort();

        assert_eq!(answer.content, "test answer");
        assert_eq!(answer.model, "k3");
        assert_eq!(answer.request_id.as_deref(), Some("chatcmpl-test"));
        assert_eq!(answer.usage.total_tokens, Some(33));
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["model"], "k3");
        assert_eq!(requests[0]["reasoning_effort"], "high");
        assert_eq!(requests[0]["max_completion_tokens"], 2_048);
        assert_eq!(requests[0]["stream"], false);
        assert_eq!(requests[0]["messages"][0]["role"], "system");
        assert_eq!(requests[0]["messages"][1]["role"], "user");
        assert!(
            requests[0]["prompt_cache_key"]
                .as_str()
                .is_some_and(|value| Uuid::parse_str(value).is_ok())
        );
    }

    #[tokio::test]
    async fn streams_openai_compatible_deltas_in_order() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Captured {
            requests: Arc::clone(&requests),
        };
        let app = Router::new()
            .route("/coding/v1/chat/completions", post(stream_handler))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = KimiClient::new(KimiConfig {
            base_url: format!("http://{address}/coding/v1"),
            api_key: "test-key".into(),
            model: "k3".into(),
            reasoning_effort: "high".into(),
            max_completion_tokens: 2_048,
            timeout: Duration::from_secs(2),
        })
        .unwrap();
        let deltas = Arc::new(Mutex::new(Vec::<String>::new()));
        let captured_deltas = Arc::clone(&deltas);
        let answer = client
            .stream_to(
                &CompletionCommand {
                    task_id: Uuid::new_v4(),
                    system_prompt: "system".into(),
                    user_payload: "{\"question\":\"hello\",\"memory_context\":[]}".into(),
                },
                move |delta| {
                    let captured_deltas = Arc::clone(&captured_deltas);
                    async move {
                        captured_deltas.lock().unwrap().push(delta);
                        Ok(())
                    }
                },
            )
            .await
            .unwrap();
        server.abort();

        assert_eq!(*deltas.lock().unwrap(), vec!["你", "好"]);
        assert_eq!(answer.content, "你好");
        assert_eq!(answer.request_id.as_deref(), Some("chatcmpl-stream"));
        assert_eq!(answer.usage.total_tokens, Some(5));
        assert_eq!(requests.lock().unwrap()[0]["stream"], true);
    }
}
