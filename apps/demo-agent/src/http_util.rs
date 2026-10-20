use std::net::IpAddr;

use reqwest::{Response, Url};
use serde_json::Value;

use crate::error::DemoError;

const MAX_REMOTE_ERROR_BYTES: usize = 4_096;

pub fn normalize_chat_base_url(raw: &str) -> Result<String, DemoError> {
    let url = parse_clean_url(raw, "Kimi")?;
    let is_official = url.scheme() == "https"
        && url.host_str() == Some("api.kimi.com")
        && url.path().trim_end_matches('/') == "/coding/v1";
    let is_deepseek = url.scheme() == "https"
        && url.host_str() == Some("api.deepseek.com")
        && matches!(url.path().trim_end_matches('/'), "" | "/v1");
    let is_local_test = is_loopback(&url) && matches!(url.scheme(), "http" | "https");
    if !is_official && !is_deepseek && !is_local_test {
        return Err(DemoError::Config(
            "chat base URL must be the official Kimi or DeepSeek endpoint; only loopback overrides are allowed for tests"
                .into(),
        ));
    }
    Ok(raw.trim_end_matches('/').to_owned())
}

fn parse_clean_url(raw: &str, service: &'static str) -> Result<Url, DemoError> {
    let url = Url::parse(raw)
        .map_err(|error| DemoError::Config(format!("{service} base URL is invalid: {error}")))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(DemoError::Config(format!(
            "{service} base URL must not contain credentials, query, or fragment"
        )));
    }
    Ok(url)
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

pub async fn require_success(
    mut response: Response,
    service: &'static str,
) -> Result<Response, DemoError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let header_request_id = ["x-request-id", "request-id"]
        .iter()
        .find_map(|name| response.headers().get(*name))
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let mut body = Vec::new();
    while body.len() < MAX_REMOTE_ERROR_BYTES {
        let chunk = response
            .chunk()
            .await
            .map_err(|source| DemoError::Transport { service, source })?;
        let Some(chunk) = chunk else {
            break;
        };
        let remaining = MAX_REMOTE_ERROR_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    let value = serde_json::from_slice::<Value>(&body).ok();
    let code = value
        .as_ref()
        .and_then(|json| json.get("code").or_else(|| json.pointer("/error/code")))
        .map(value_to_short_string)
        .unwrap_or_else(|| "REMOTE_REJECTED".into());
    let request_id = header_request_id
        .or_else(|| {
            value
                .as_ref()
                .and_then(|json| json.get("request_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "unavailable".into());

    Err(DemoError::Rejected {
        service,
        status,
        code,
        request_id,
    })
}

fn value_to_short_string(value: &Value) -> String {
    let raw = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => "REMOTE_REJECTED".into(),
    };
    raw.chars().take(128).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kimi_url_is_pinned_to_the_official_endpoint() {
        assert!(
            normalize_chat_base_url("https://api.kimi.com/coding/v1").is_ok(),
            "official URL must be accepted"
        );
        assert!(normalize_chat_base_url("https://api.kimi.com/coding/v1?redirect=evil").is_err());
        assert!(normalize_chat_base_url("https://example.com/v1").is_err());
        assert!(normalize_chat_base_url("https://api.deepseek.com").is_ok());
        assert!(normalize_chat_base_url("https://api.deepseek.com/v1/").is_ok());
        assert!(normalize_chat_base_url("http://api.deepseek.com").is_err());
        assert!(normalize_chat_base_url("https://api.deepseek.com/other").is_err());
        assert!(normalize_chat_base_url("http://127.0.0.1:9999/coding/v1").is_ok());
    }
}
