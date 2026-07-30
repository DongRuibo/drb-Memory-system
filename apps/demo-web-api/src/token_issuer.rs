use std::env;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use hmac::{Hmac, Mac};
use memory_client::{MemoryClientConfig, MemoryHttpClient};
use serde::Serialize;
use sha2::Sha256;
use thiserror::Error;

use crate::demo_identity::DemoUserProfile;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct DemoMemoryClientFactory {
    memory_base_url: String,
    memory_timeout: Duration,
    token_issuer: DemoTokenIssuer,
}

impl DemoMemoryClientFactory {
    pub fn from_env(
        memory_base_url: String,
        memory_timeout: Duration,
        tenant_id: String,
    ) -> Result<Self, DemoTokenError> {
        Ok(Self {
            memory_base_url,
            memory_timeout,
            token_issuer: DemoTokenIssuer::from_env(tenant_id)?,
        })
    }

    pub fn client_for(
        &self,
        profile: &DemoUserProfile,
    ) -> Result<MemoryHttpClient, DemoTokenError> {
        let token = self.token_issuer.issue(profile)?;
        MemoryHttpClient::new(MemoryClientConfig::authenticated(
            &self.memory_base_url,
            self.memory_timeout,
            &token,
        ))
        .map_err(|error| DemoTokenError::MemoryClient(error.to_string()))
    }
}

#[derive(Clone)]
struct DemoTokenIssuer {
    secret: Arc<Vec<u8>>,
    issuer: String,
    audience: String,
    tenant_id: String,
}

impl DemoTokenIssuer {
    fn from_env(tenant_id: String) -> Result<Self, DemoTokenError> {
        let secret = env::var("MEMORY_JWT_HS256_SECRET")
            .map_err(|_| DemoTokenError::Missing("MEMORY_JWT_HS256_SECRET"))?;
        if secret.len() < 32 {
            return Err(DemoTokenError::Invalid(
                "MEMORY_JWT_HS256_SECRET must contain at least 32 bytes".into(),
            ));
        }
        let issuer = env::var("MEMORY_JWT_ISSUER").unwrap_or_else(|_| "lazytt-demo-web-api".into());
        let audience =
            env::var("MEMORY_JWT_AUDIENCE").unwrap_or_else(|_| "lazytt-memory-service".into());
        if issuer.trim().is_empty() || audience.trim().is_empty() || tenant_id.trim().is_empty() {
            return Err(DemoTokenError::Invalid(
                "JWT issuer, audience, and tenant must not be empty".into(),
            ));
        }
        Ok(Self {
            secret: Arc::new(secret.into_bytes()),
            issuer,
            audience,
            tenant_id,
        })
    }

    fn issue(&self, profile: &DemoUserProfile) -> Result<String, DemoTokenError> {
        let now = Utc::now().timestamp();
        let header = encode_json(&JwtHeader {
            alg: "HS256",
            typ: "JWT",
        })?;
        let claims = encode_json(&JwtClaims {
            iss: &self.issuer,
            aud: &self.audience,
            sub: &profile.user_id,
            tenant_id: &self.tenant_id,
            user_id: &profile.user_id,
            tenant_scope: false,
            project_ids: &profile.project_ids,
            agent_ids: &[],
            permissions: &["memory:*"],
            iat: now,
            nbf: now - 1,
            exp: now + 15 * 60,
        })?;
        let signing_input = format!("{header}.{claims}");
        let mut signer = HmacSha256::new_from_slice(self.secret.as_slice())
            .map_err(|_| DemoTokenError::Invalid("JWT signing key is invalid".into()))?;
        signer.update(signing_input.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(signer.finalize().into_bytes());
        Ok(format!("{signing_input}.{signature}"))
    }
}

#[derive(Serialize)]
struct JwtHeader<'a> {
    alg: &'a str,
    typ: &'a str,
}

#[derive(Serialize)]
struct JwtClaims<'a> {
    iss: &'a str,
    aud: &'a str,
    sub: &'a str,
    tenant_id: &'a str,
    user_id: &'a str,
    tenant_scope: bool,
    project_ids: &'a [String],
    agent_ids: &'a [String],
    permissions: &'a [&'a str],
    iat: i64,
    nbf: i64,
    exp: i64,
}

#[derive(Debug, Error)]
pub enum DemoTokenError {
    #[error("{0} is required for multi-user demo authority")]
    Missing(&'static str),
    #[error("demo JWT configuration is invalid: {0}")]
    Invalid(String),
    #[error("demo JWT serialization failed: {0}")]
    Serialization(String),
    #[error("memory client configuration failed: {0}")]
    MemoryClient(String),
}

fn encode_json(value: &impl Serialize) -> Result<String, DemoTokenError> {
    serde_json::to_vec(value)
        .map(|encoded| URL_SAFE_NO_PAD.encode(encoded))
        .map_err(|error| DemoTokenError::Serialization(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::Value;

    #[test]
    fn issued_token_binds_user_tenant_projects_and_permissions() {
        let issuer = DemoTokenIssuer {
            secret: Arc::new(b"0123456789abcdef0123456789abcdef".to_vec()),
            issuer: "issuer".into(),
            audience: "audience".into(),
            tenant_id: "tenant-a".into(),
        };
        let profile = crate::demo_identity::DemoUserRegistry::standard()
            .get("demo-alice")
            .unwrap();
        let token = issuer.issue(&profile).unwrap();
        let parts = token.split('.').collect::<Vec<_>>();
        assert_eq!(parts.len(), 3);
        let claims: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["sub"], "demo-alice");
        assert_eq!(claims["tenant_id"], "tenant-a");
        assert_eq!(claims["project_ids"][0], "lazytt-memory-system-demo");
        assert_eq!(claims["permissions"][0], "memory:*");
    }
}
