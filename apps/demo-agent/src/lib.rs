mod config;
mod demo;
mod error;
mod http_util;
mod kimi_client;
mod model;
mod prompt;

pub use config::DemoConfig;
pub use error::DemoError;
pub use kimi_client::{CompletionCommand, KimiClient, KimiConfig, ModelAnswer};
pub use memory_client::OperationResponse;
pub use model::{AnswerTurn, DemoReport, ModelUsage};
pub use prompt::{PreparedPrompt, SYSTEM_PROMPT, assemble_prompt};

use demo::{DemoAgent, DemoRunConfig};
use memory_client::{MemoryClientConfig, MemoryHttpClient, MemoryPort};

pub async fn check_readiness() -> Result<(), DemoError> {
    let config = DemoConfig::from_env()?;
    let memory = memory_client_from_config(&config)?;
    Ok(memory.ready().await?)
}

pub async fn run_from_env() -> Result<DemoReport, DemoError> {
    let config = DemoConfig::from_env()?;
    let memory = memory_client_from_config(&config)?;
    let model = KimiClient::new(config.kimi)?;
    DemoAgent::new(
        memory,
        model,
        DemoRunConfig {
            user_id: config.user_id,
            preference: config.preference,
            question: config.question,
            automatic_memory: config.automatic_memory,
            projection_timeout: config.projection_timeout,
        },
    )
    .run()
    .await
}

pub fn memory_client_from_config(config: &DemoConfig) -> Result<MemoryHttpClient, DemoError> {
    let client_config = match &config.memory_bearer_token {
        Some(token) => {
            MemoryClientConfig::authenticated(&config.memory_base_url, config.memory_timeout, token)
        }
        None => MemoryClientConfig::loopback(&config.memory_base_url, config.memory_timeout),
    };
    Ok(MemoryHttpClient::new(client_config)?)
}
