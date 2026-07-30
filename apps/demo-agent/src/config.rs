use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::DemoError;
use crate::kimi_client::KimiConfig;

const DEFAULT_MEMORY_BASE_URL: &str = "http://127.0.0.1:8080";
const DEFAULT_KIMI_BASE_URL: &str = "https://api.kimi.com/coding/v1";
const DEFAULT_KIMI_MODEL: &str = "k3";
const DEFAULT_PREFERENCE: &str =
    "回答技术问题时请固定使用【原理】和【示例】两个标题，并先给一个生活类比";
const DEFAULT_QUESTION: &str = "请解释事务型 Outbox 为什么能避免数据库提交成功但消息丢失。";

pub struct DemoConfig {
    pub memory_base_url: String,
    pub memory_bearer_token: Option<String>,
    pub memory_timeout: Duration,
    pub projection_timeout: Duration,
    pub user_id: String,
    pub preference: String,
    pub question: String,
    pub automatic_memory: bool,
    pub kimi: KimiConfig,
}

impl DemoConfig {
    pub fn from_env() -> Result<Self, DemoError> {
        let local = LocalConfig::load()?;
        let api_key = local.value("KIMI_API_KEY", &["apikey"]).ok_or_else(|| {
            DemoError::Config(
                "KIMI_API_KEY is not set and no apikey was found in the local .env".into(),
            )
        })?;
        if api_key.trim().is_empty() {
            return Err(DemoError::Config(
                "the configured Kimi API key is empty".into(),
            ));
        }

        let reasoning_effort = local
            .value("KIMI_REASONING_EFFORT", &[])
            .unwrap_or_else(|| "low".into());
        if !matches!(reasoning_effort.as_str(), "low" | "high" | "max") {
            return Err(DemoError::Config(
                "KIMI_REASONING_EFFORT must be low, high, or max".into(),
            ));
        }

        let max_completion_tokens =
            parse_bounded_u32(&local, "KIMI_MAX_COMPLETION_TOKENS", 1_024, 256, 8_192)?;
        let kimi_timeout_seconds = parse_bounded_u64(&local, "KIMI_TIMEOUT_SECONDS", 90, 5, 300)?;
        let memory_timeout_seconds =
            parse_bounded_u64(&local, "MEMORY_TIMEOUT_SECONDS", 30, 1, 30)?;
        let projection_timeout_seconds =
            parse_bounded_u64(&local, "PROJECTION_TIMEOUT_SECONDS", 30, 1, 120)?;

        let user_id = local
            .value("DEMO_USER_ID", &[])
            .unwrap_or_else(|| "semantic-demo".into());
        if user_id.trim().is_empty() || user_id.len() > 256 {
            return Err(DemoError::Config(
                "DEMO_USER_ID must contain 1 to 256 characters".into(),
            ));
        }

        let preference = local
            .value("DEMO_AGENT_MEMORY", &[])
            .unwrap_or_else(|| DEFAULT_PREFERENCE.into());
        if preference.trim().is_empty() || preference.len() > 8_192 {
            return Err(DemoError::Config(
                "DEMO_AGENT_MEMORY must contain 1 to 8192 characters".into(),
            ));
        }

        let question = local
            .value("DEMO_AGENT_QUESTION", &[])
            .unwrap_or_else(|| DEFAULT_QUESTION.into());
        if question.trim().is_empty() || question.len() > 4_096 {
            return Err(DemoError::Config(
                "DEMO_AGENT_QUESTION must contain 1 to 4096 characters".into(),
            ));
        }
        let automatic_memory = match local
            .value("DEMO_AGENT_MEMORY_MODE", &[])
            .unwrap_or_else(|| "explicit".into())
            .as_str()
        {
            "explicit" => false,
            "automatic" => true,
            _ => {
                return Err(DemoError::Config(
                    "DEMO_AGENT_MEMORY_MODE must be explicit or automatic".into(),
                ));
            }
        };

        Ok(Self {
            memory_base_url: local
                .value("MEMORY_BASE_URL", &[])
                .unwrap_or_else(|| DEFAULT_MEMORY_BASE_URL.into()),
            memory_bearer_token: local
                .value("MEMORY_BEARER_TOKEN", &[])
                .filter(|value| !value.trim().is_empty()),
            memory_timeout: Duration::from_secs(memory_timeout_seconds),
            projection_timeout: Duration::from_secs(projection_timeout_seconds),
            user_id,
            preference,
            question,
            automatic_memory,
            kimi: KimiConfig {
                base_url: local
                    .value("KIMI_BASE_URL", &["base_url"])
                    .unwrap_or_else(|| DEFAULT_KIMI_BASE_URL.into()),
                api_key,
                model: local
                    .value("KIMI_MODEL", &["model_name"])
                    .unwrap_or_else(|| DEFAULT_KIMI_MODEL.into()),
                reasoning_effort,
                max_completion_tokens,
                timeout: Duration::from_secs(kimi_timeout_seconds),
            },
        })
    }
}

struct LocalConfig {
    values: HashMap<String, String>,
}

impl LocalConfig {
    fn load() -> Result<Self, DemoError> {
        let path = env::var_os("DEMO_AGENT_ENV_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".env"));
        if !path.exists() {
            return Ok(Self {
                values: HashMap::new(),
            });
        }
        validate_secret_file(&path)?;
        let content = fs::read_to_string(&path).map_err(|error| {
            DemoError::Config(format!(
                "failed to read local config {}: {error}",
                path.display()
            ))
        })?;
        if content.len() > 64 * 1024 {
            return Err(DemoError::Config(format!(
                "local config {} exceeds 64 KiB",
                path.display()
            )));
        }
        Ok(Self {
            values: parse_dotenv(&content)?,
        })
    }

    fn value(&self, primary: &str, aliases: &[&str]) -> Option<String> {
        env::var(primary)
            .ok()
            .or_else(|| self.values.get(primary).cloned())
            .or_else(|| {
                aliases
                    .iter()
                    .find_map(|alias| self.values.get(*alias).cloned())
            })
    }
}

fn validate_secret_file(path: &Path) -> Result<(), DemoError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        DemoError::Config(format!(
            "failed to inspect local config {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(DemoError::Config(format!(
            "local config {} must be a regular file, not a symlink",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(DemoError::Config(format!(
                "local config {} is accessible by group or others; run chmod 600 {}",
                path.display(),
                path.display()
            )));
        }
    }
    Ok(())
}

fn parse_dotenv(content: &str) -> Result<HashMap<String, String>, DemoError> {
    let mut values = HashMap::new();
    for (index, raw_line) in content.lines().enumerate() {
        let mut line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("export ") {
            line = rest.trim_start();
        }
        let (raw_key, raw_value) = line.split_once('=').ok_or_else(|| {
            DemoError::Config(format!("local .env line {} must use NAME=value", index + 1))
        })?;
        let key = raw_key.trim();
        if key.is_empty()
            || !key.bytes().enumerate().all(|(position, byte)| {
                byte == b'_'
                    || byte.is_ascii_alphanumeric() && (position > 0 || !byte.is_ascii_digit())
            })
        {
            return Err(DemoError::Config(format!(
                "local .env line {} has an invalid variable name",
                index + 1
            )));
        }
        let value = unquote(raw_value.trim()).ok_or_else(|| {
            DemoError::Config(format!(
                "local .env line {} has unmatched quotes",
                index + 1
            ))
        })?;
        if values.insert(key.to_owned(), value).is_some() {
            return Err(DemoError::Config(format!(
                "local .env defines {key} more than once"
            )));
        }
    }
    Ok(values)
}

fn unquote(value: &str) -> Option<String> {
    match (value.chars().next(), value.chars().last()) {
        (Some(first @ ('"' | '\'')), Some(last)) if first == last && value.len() >= 2 => {
            Some(value[1..value.len() - 1].to_owned())
        }
        (Some('"' | '\''), _) | (_, Some('"' | '\'')) => None,
        _ => Some(value.to_owned()),
    }
}

fn parse_bounded_u64(
    local: &LocalConfig,
    name: &'static str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, DemoError> {
    let value = match local.value(name, &[]) {
        Some(raw) => raw
            .parse::<u64>()
            .map_err(|_| DemoError::Config(format!("{name} must be an integer")))?,
        None => default,
    };
    if !(minimum..=maximum).contains(&value) {
        return Err(DemoError::Config(format!(
            "{name} must be between {minimum} and {maximum}"
        )));
    }
    Ok(value)
}

fn parse_bounded_u32(
    local: &LocalConfig,
    name: &'static str,
    default: u32,
    minimum: u32,
    maximum: u32,
) -> Result<u32, DemoError> {
    let value = parse_bounded_u64(
        local,
        name,
        u64::from(default),
        u64::from(minimum),
        u64::from(maximum),
    )?;
    u32::try_from(value).map_err(|_| DemoError::Config(format!("{name} is too large")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_parser_accepts_current_aliases_without_evaluating_shell() {
        let values = parse_dotenv(
            "apikey='test-key'\nbase_url=https://api.kimi.com/coding/v1\nmodel_name=k3\n",
        )
        .unwrap();
        assert_eq!(values.get("apikey").map(String::as_str), Some("test-key"));
        assert_eq!(
            values.get("base_url").map(String::as_str),
            Some("https://api.kimi.com/coding/v1")
        );
        assert_eq!(values.get("model_name").map(String::as_str), Some("k3"));
    }

    #[test]
    fn dotenv_parser_rejects_duplicate_or_executable_lines() {
        assert!(parse_dotenv("apikey=first\napikey=second\n").is_err());
        assert!(parse_dotenv("$(touch /tmp/not-allowed)\n").is_err());
    }
}
