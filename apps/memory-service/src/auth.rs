use std::collections::BTreeSet;
use std::env;

use axum::http::HeaderMap;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use hmac::{Hmac, Mac};
use memory_postgres::DemoAuthority;
use serde::Deserialize;
use sha2::Sha256;
use thiserror::Error;

pub const MEMORY_READ: &str = "memory:read";
pub const MEMORY_WRITE: &str = "memory:write";
pub const MEMORY_CORRECT: &str = "memory:correct";
pub const MEMORY_REVIEW: &str = "memory:review";
pub const MEMORY_FORGET: &str = "memory:forget";
pub const MEMORY_EXPORT: &str = "memory:export";
pub const MEMORY_ERASE: &str = "memory:erase";
pub const MEMORY_ADMIN: &str = "memory:admin";
pub const MEMORY_OUTCOME: &str = "memory:outcome";
pub const MEMORY_EXTRACT: &str = "memory:extract";

const MAX_TOKEN_BYTES: usize = 8 * 1024;
const MAX_TOKEN_LIFETIME_SECONDS: i64 = 60 * 60;
const CLOCK_SKEW_SECONDS: i64 = 30;
pub const TRUSTED_USER_ID_HEADER: &str = "x-user-id";
pub const TRUSTED_TENANT_ID_HEADER: &str = "x-tenant-id";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct Authenticator {
    mode: AuthenticationMode,
}

#[derive(Clone)]
enum AuthenticationMode {
    Demo {
        authority: DemoAuthority,
    },
    TrustedUserId {
        tenant_ids: Option<BTreeSet<String>>,
    },
    JwtHs256 {
        secret: Vec<u8>,
        issuer: String,
        audience: String,
        tenant_ids: BTreeSet<String>,
    },
}

#[derive(Debug, Clone)]
pub struct AuthenticatedPrincipal {
    pub authority: DemoAuthority,
    permissions: BTreeSet<String>,
}

impl AuthenticatedPrincipal {
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions.contains("memory:*") || self.permissions.contains(permission)
    }
}

#[derive(Debug, Error)]
pub enum AuthenticationError {
    #[error("authentication configuration is invalid: {0}")]
    Configuration(String),
    #[error("authentication is required")]
    Missing,
    #[error("authentication token is invalid")]
    Invalid,
    #[error("X-User-Id must contain 1 to 256 visible characters")]
    InvalidUserId,
    #[error("X-Tenant-Id must identify an allowed tenant")]
    InvalidTenantId,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JwtHeader {
    alg: String,
    typ: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JwtClaims {
    iss: String,
    aud: Audience,
    sub: String,
    tenant_id: String,
    user_id: String,
    #[serde(default)]
    tenant_scope: bool,
    #[serde(default)]
    project_ids: Vec<String>,
    #[serde(default)]
    agent_ids: Vec<String>,
    permissions: Vec<String>,
    iat: i64,
    nbf: Option<i64>,
    exp: i64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(value) => value == expected,
            Self::Many(values) => values.iter().any(|value| value == expected),
        }
    }
}

impl Authenticator {
    pub fn from_env(demo_authority: DemoAuthority) -> Result<Self, AuthenticationError> {
        let mode = env::var("MEMORY_AUTH_MODE").unwrap_or_else(|_| "demo".into());
        let dynamic_tenant_value = env::var("MEMORY_ALLOW_DYNAMIC_TENANT").ok();
        let allow_dynamic_tenant = parse_boolean_env(
            "MEMORY_ALLOW_DYNAMIC_TENANT",
            dynamic_tenant_value.as_deref(),
            false,
        )?;
        if allow_dynamic_tenant && mode != "trusted_user_id" {
            return Err(AuthenticationError::Configuration(
                "MEMORY_ALLOW_DYNAMIC_TENANT may only be enabled with MEMORY_AUTH_MODE=trusted_user_id"
                    .into(),
            ));
        }
        match mode.as_str() {
            "demo" => Ok(Self::demo(demo_authority)),
            "trusted_user_id" => {
                if allow_dynamic_tenant {
                    Ok(Self::trusted_dynamic_tenant())
                } else {
                    let tenant_ids = parse_tenant_ids(&required_env("MEMORY_TENANT_IDS")?)?;
                    Self::trusted_user_id(tenant_ids)
                }
            }
            "jwt_hs256" => {
                let secret = required_env("MEMORY_JWT_HS256_SECRET")?;
                let issuer = required_env("MEMORY_JWT_ISSUER")?;
                let audience = required_env("MEMORY_JWT_AUDIENCE")?;
                let tenant_ids = parse_tenant_ids(&required_env("MEMORY_TENANT_IDS")?)?;
                Self::jwt_hs256(secret.into_bytes(), issuer, audience, tenant_ids)
            }
            _ => Err(AuthenticationError::Configuration(
                "MEMORY_AUTH_MODE must be demo, trusted_user_id, or jwt_hs256".into(),
            )),
        }
    }

    pub fn demo(authority: DemoAuthority) -> Self {
        Self {
            mode: AuthenticationMode::Demo { authority },
        }
    }

    pub fn jwt_hs256(
        secret: Vec<u8>,
        issuer: String,
        audience: String,
        tenant_ids: BTreeSet<String>,
    ) -> Result<Self, AuthenticationError> {
        if secret.len() < 32 {
            return Err(AuthenticationError::Configuration(
                "MEMORY_JWT_HS256_SECRET must contain at least 32 bytes".into(),
            ));
        }
        validate_identifier("MEMORY_JWT_ISSUER", &issuer)?;
        validate_identifier("MEMORY_JWT_AUDIENCE", &audience)?;
        validate_tenant_ids(&tenant_ids)?;
        Ok(Self {
            mode: AuthenticationMode::JwtHs256 {
                secret,
                issuer,
                audience,
                tenant_ids,
            },
        })
    }

    pub fn trusted_user_id(tenant_ids: BTreeSet<String>) -> Result<Self, AuthenticationError> {
        validate_tenant_ids(&tenant_ids)?;
        Ok(Self {
            mode: AuthenticationMode::TrustedUserId {
                tenant_ids: Some(tenant_ids),
            },
        })
    }

    pub fn trusted_dynamic_tenant() -> Self {
        Self {
            mode: AuthenticationMode::TrustedUserId { tenant_ids: None },
        }
    }

    pub fn is_demo(&self) -> bool {
        matches!(self.mode, AuthenticationMode::Demo { .. })
    }

    pub fn mode_name(&self) -> &'static str {
        match &self.mode {
            AuthenticationMode::Demo { .. } => "demo",
            AuthenticationMode::TrustedUserId { tenant_ids: None } => {
                "trusted_user_id_dynamic_tenant"
            }
            AuthenticationMode::TrustedUserId {
                tenant_ids: Some(_),
            } => "trusted_user_id",
            AuthenticationMode::JwtHs256 { .. } => "jwt_hs256",
        }
    }

    pub fn allows_dynamic_tenants(&self) -> bool {
        matches!(
            self.mode,
            AuthenticationMode::TrustedUserId { tenant_ids: None }
        )
    }

    pub fn tenant_ids(&self) -> Vec<String> {
        match &self.mode {
            AuthenticationMode::Demo { authority } => vec![authority.tenant_id.clone()],
            AuthenticationMode::TrustedUserId {
                tenant_ids: Some(tenant_ids),
            } => tenant_ids.iter().cloned().collect(),
            AuthenticationMode::TrustedUserId { tenant_ids: None } => Vec::new(),
            AuthenticationMode::JwtHs256 { tenant_ids, .. } => tenant_ids.iter().cloned().collect(),
        }
    }

    pub fn authenticate(
        &self,
        headers: &HeaderMap,
    ) -> Result<AuthenticatedPrincipal, AuthenticationError> {
        match &self.mode {
            AuthenticationMode::Demo { authority } => Ok(AuthenticatedPrincipal {
                authority: authority.clone(),
                permissions: BTreeSet::from(["memory:*".into()]),
            }),
            AuthenticationMode::TrustedUserId { tenant_ids } => {
                authenticate_trusted_user_id(headers, tenant_ids)
            }
            AuthenticationMode::JwtHs256 {
                secret,
                issuer,
                audience,
                tenant_ids,
            } => authenticate_hs256(
                headers,
                secret,
                issuer,
                audience,
                tenant_ids,
                Utc::now().timestamp(),
            ),
        }
    }
}

fn authenticate_trusted_user_id(
    headers: &HeaderMap,
    tenant_ids: &Option<BTreeSet<String>>,
) -> Result<AuthenticatedPrincipal, AuthenticationError> {
    let tenant_id = canonical_header_identifier(headers, TRUSTED_TENANT_ID_HEADER)
        .ok_or(AuthenticationError::InvalidTenantId)?;
    if tenant_ids
        .as_ref()
        .is_some_and(|allowed| !allowed.contains(tenant_id))
    {
        return Err(AuthenticationError::InvalidTenantId);
    }
    let user_id = canonical_header_identifier(headers, TRUSTED_USER_ID_HEADER)
        .ok_or(AuthenticationError::InvalidUserId)?;
    Ok(AuthenticatedPrincipal {
        authority: DemoAuthority::user(tenant_id, user_id),
        permissions: BTreeSet::from([
            MEMORY_READ.into(),
            MEMORY_EXTRACT.into(),
            MEMORY_OUTCOME.into(),
        ]),
    })
}

fn canonical_header_identifier<'a>(headers: &'a HeaderMap, name: &'static str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() || value.trim() != value || validate_claim_identifier(value).is_err()
    {
        return None;
    }
    Some(value)
}

fn authenticate_hs256(
    headers: &HeaderMap,
    secret: &[u8],
    issuer: &str,
    audience: &str,
    tenant_ids: &BTreeSet<String>,
    now: i64,
) -> Result<AuthenticatedPrincipal, AuthenticationError> {
    let authorization_values = headers.get_all(axum::http::header::AUTHORIZATION);
    let mut values = authorization_values.iter();
    let value = values.next().ok_or(AuthenticationError::Missing)?;
    if values.next().is_some() {
        return Err(AuthenticationError::Invalid);
    }
    let value = value.to_str().map_err(|_| AuthenticationError::Invalid)?;
    let token = value
        .strip_prefix("Bearer ")
        .filter(|value| !value.is_empty() && !value.contains(char::is_whitespace))
        .ok_or(AuthenticationError::Invalid)?;
    if token.len() > MAX_TOKEN_BYTES {
        return Err(AuthenticationError::Invalid);
    }
    let mut parts = token.split('.');
    let header_part = parts.next().ok_or(AuthenticationError::Invalid)?;
    let claims_part = parts.next().ok_or(AuthenticationError::Invalid)?;
    let signature_part = parts.next().ok_or(AuthenticationError::Invalid)?;
    if parts.next().is_some()
        || header_part.is_empty()
        || claims_part.is_empty()
        || signature_part.is_empty()
    {
        return Err(AuthenticationError::Invalid);
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature_part)
        .map_err(|_| AuthenticationError::Invalid)?;
    let mut verifier =
        HmacSha256::new_from_slice(secret).map_err(|_| AuthenticationError::Invalid)?;
    verifier.update(format!("{header_part}.{claims_part}").as_bytes());
    verifier
        .verify_slice(&signature)
        .map_err(|_| AuthenticationError::Invalid)?;

    let header: JwtHeader = decode_json_part(header_part)?;
    if header.alg != "HS256" || header.typ != "JWT" {
        return Err(AuthenticationError::Invalid);
    }
    let claims: JwtClaims = decode_json_part(claims_part)?;
    if claims.iss != issuer
        || !claims.aud.contains(audience)
        || claims.sub != claims.user_id
        || !tenant_ids.contains(&claims.tenant_id)
        || claims.exp <= claims.iat
        || claims.exp - claims.iat > MAX_TOKEN_LIFETIME_SECONDS
        || claims.iat > now + CLOCK_SKEW_SECONDS
        || claims.exp <= now - CLOCK_SKEW_SECONDS
        || claims
            .nbf
            .is_some_and(|not_before| not_before > now + CLOCK_SKEW_SECONDS)
    {
        return Err(AuthenticationError::Invalid);
    }
    validate_claim_identifier(&claims.sub)?;
    validate_claim_identifier(&claims.tenant_id)?;
    validate_claim_identifier(&claims.user_id)?;
    if claims.permissions.is_empty()
        || claims.permissions.len() > 32
        || claims.permissions.iter().any(|permission| {
            permission.len() > 128
                || !(permission.starts_with("memory:")
                    && permission.chars().all(|value| {
                        value.is_ascii_alphanumeric() || matches!(value, ':' | '_' | '*')
                    }))
        })
    {
        return Err(AuthenticationError::Invalid);
    }
    let mut authority = DemoAuthority::user(claims.tenant_id, claims.user_id);
    authority.tenant_scope = claims.tenant_scope;
    authority.project_ids = validate_scope_ids(claims.project_ids)?;
    authority.agent_ids = validate_scope_ids(claims.agent_ids)?;
    if authority.project_ids.len()
        + authority.agent_ids.len()
        + usize::from(authority.tenant_scope)
        + 1
        > 32
    {
        return Err(AuthenticationError::Invalid);
    }
    Ok(AuthenticatedPrincipal {
        authority,
        permissions: claims.permissions.into_iter().collect(),
    })
}

fn decode_json_part<T: for<'de> Deserialize<'de>>(encoded: &str) -> Result<T, AuthenticationError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| AuthenticationError::Invalid)?;
    if decoded.len() > MAX_TOKEN_BYTES {
        return Err(AuthenticationError::Invalid);
    }
    serde_json::from_slice(&decoded).map_err(|_| AuthenticationError::Invalid)
}

fn required_env(name: &str) -> Result<String, AuthenticationError> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AuthenticationError::Configuration(format!("{name} is required")))
}

fn parse_boolean_env(
    name: &str,
    value: Option<&str>,
    default: bool,
) -> Result<bool, AuthenticationError> {
    match value {
        None => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(_) => Err(AuthenticationError::Configuration(format!(
            "{name} must be true or false"
        ))),
    }
}

fn parse_tenant_ids(value: &str) -> Result<BTreeSet<String>, AuthenticationError> {
    let values = value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    if values.len() != value.split(',').count() {
        return Err(AuthenticationError::Configuration(
            "MEMORY_TENANT_IDS contains an empty or duplicate tenant ID".into(),
        ));
    }
    Ok(values)
}

fn validate_tenant_ids(tenant_ids: &BTreeSet<String>) -> Result<(), AuthenticationError> {
    if tenant_ids.is_empty() || tenant_ids.len() > 100 {
        return Err(AuthenticationError::Configuration(
            "MEMORY_TENANT_IDS must contain between 1 and 100 tenant IDs".into(),
        ));
    }
    for tenant_id in tenant_ids {
        validate_identifier("MEMORY_TENANT_IDS entry", tenant_id)?;
        if tenant_id.trim() != tenant_id
            || tenant_id.len() > 256
            || tenant_id.chars().any(char::is_control)
        {
            return Err(AuthenticationError::Configuration(
                "MEMORY_TENANT_IDS entries must contain 1 to 256 visible characters".into(),
            ));
        }
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str) -> Result<(), AuthenticationError> {
    if value.trim().is_empty() || value.len() > 512 {
        return Err(AuthenticationError::Configuration(format!(
            "{name} must contain 1 to 512 characters"
        )));
    }
    Ok(())
}

fn validate_claim_identifier(value: &str) -> Result<(), AuthenticationError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(AuthenticationError::Invalid);
    }
    Ok(())
}

fn validate_scope_ids(values: Vec<String>) -> Result<BTreeSet<String>, AuthenticationError> {
    if values.len() > 100 {
        return Err(AuthenticationError::Invalid);
    }
    let value_count = values.len();
    let values = values.into_iter().collect::<BTreeSet<_>>();
    if values.len() != value_count {
        return Err(AuthenticationError::Invalid);
    }
    for value in &values {
        validate_claim_identifier(value)?;
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use serde_json::json;

    const SECRET: &[u8] = b"test-only-memory-auth-secret-with-more-than-32-bytes";

    fn token(claims: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        let signing_input = format!("{header}.{claims}");
        let mut signer = HmacSha256::new_from_slice(SECRET).unwrap();
        signer.update(signing_input.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(signer.finalize().into_bytes());
        format!("{signing_input}.{signature}")
    }

    fn headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers
    }

    fn valid_claims(now: i64) -> serde_json::Value {
        json!({
            "iss": "https://identity.internal",
            "aud": ["lazytt-memory", "another-service"],
            "sub": "alice",
            "tenant_id": "tenant-a",
            "user_id": "alice",
            "tenant_scope": true,
            "project_ids": ["project-7"],
            "agent_ids": ["agent-3"],
            "permissions": ["memory:read", "memory:write"],
            "iat": now - 10,
            "nbf": now - 10,
            "exp": now + 300
        })
    }

    #[test]
    fn valid_short_lived_token_derives_principal_and_permissions() {
        let now = 1_800_000_000;
        let principal = authenticate_hs256(
            &headers(&token(valid_claims(now))),
            SECRET,
            "https://identity.internal",
            "lazytt-memory",
            &BTreeSet::from(["tenant-a".into()]),
            now,
        )
        .unwrap();
        assert_eq!(principal.authority.tenant_id, "tenant-a");
        assert_eq!(principal.authority.user_id, "alice");
        assert!(principal.authority.tenant_scope);
        assert!(principal.authority.project_ids.contains("project-7"));
        assert!(principal.authority.agent_ids.contains("agent-3"));
        assert!(principal.has_permission(MEMORY_READ));
        assert!(!principal.has_permission(MEMORY_ERASE));
    }

    #[test]
    fn trusted_user_id_derives_user_scoped_least_privilege_principal() {
        let authenticator =
            Authenticator::trusted_user_id(BTreeSet::from(["tenant-a".into(), "tenant-b".into()]))
                .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            TRUSTED_TENANT_ID_HEADER,
            HeaderValue::from_static("tenant-b"),
        );
        headers.insert(
            TRUSTED_USER_ID_HEADER,
            HeaderValue::from_static("identity-user-42"),
        );

        let principal = authenticator.authenticate(&headers).unwrap();

        assert_eq!(principal.authority.tenant_id, "tenant-b");
        assert_eq!(principal.authority.user_id, "identity-user-42");
        assert!(!principal.authority.tenant_scope);
        assert!(principal.has_permission(MEMORY_READ));
        assert!(principal.has_permission(MEMORY_EXTRACT));
        assert!(principal.has_permission(MEMORY_OUTCOME));
        assert!(!principal.has_permission(MEMORY_WRITE));
        assert!(!principal.has_permission(MEMORY_ADMIN));
        assert!(!principal.has_permission(MEMORY_ERASE));
    }

    #[test]
    fn trusted_dynamic_tenant_accepts_any_canonical_tenant_id() {
        let authenticator = Authenticator::trusted_dynamic_tenant();
        let mut headers = HeaderMap::new();
        headers.insert(
            TRUSTED_TENANT_ID_HEADER,
            HeaderValue::from_static("wifi-test-company-c"),
        );
        headers.insert(
            TRUSTED_USER_ID_HEADER,
            HeaderValue::from_static("identity-user-42"),
        );

        let principal = authenticator.authenticate(&headers).unwrap();

        assert!(authenticator.allows_dynamic_tenants());
        assert!(authenticator.tenant_ids().is_empty());
        assert_eq!(principal.authority.tenant_id, "wifi-test-company-c");
        assert_eq!(principal.authority.user_id, "identity-user-42");
    }

    #[test]
    fn trusted_dynamic_tenant_still_rejects_noncanonical_tenant_id() {
        let authenticator = Authenticator::trusted_dynamic_tenant();
        let mut headers = HeaderMap::new();
        headers.insert(
            TRUSTED_TENANT_ID_HEADER,
            HeaderValue::from_static(" company-c "),
        );
        headers.insert(TRUSTED_USER_ID_HEADER, HeaderValue::from_static("alice"));

        assert!(matches!(
            authenticator.authenticate(&headers),
            Err(AuthenticationError::InvalidTenantId)
        ));
    }

    #[test]
    fn trusted_user_id_rejects_missing_duplicate_or_noncanonical_header() {
        let authenticator =
            Authenticator::trusted_user_id(BTreeSet::from(["tenant-a".into()])).unwrap();
        assert!(matches!(
            authenticator.authenticate(&HeaderMap::new()),
            Err(AuthenticationError::InvalidTenantId)
        ));

        let mut missing_user = HeaderMap::new();
        missing_user.insert(
            TRUSTED_TENANT_ID_HEADER,
            HeaderValue::from_static("tenant-a"),
        );
        assert!(matches!(
            authenticator.authenticate(&missing_user),
            Err(AuthenticationError::InvalidUserId)
        ));

        let mut invalid_tenant = missing_user.clone();
        invalid_tenant.insert(TRUSTED_USER_ID_HEADER, HeaderValue::from_static("alice"));
        invalid_tenant.insert(
            TRUSTED_TENANT_ID_HEADER,
            HeaderValue::from_static("tenant-not-allowed"),
        );
        assert!(matches!(
            authenticator.authenticate(&invalid_tenant),
            Err(AuthenticationError::InvalidTenantId)
        ));

        let mut duplicate_tenant = invalid_tenant.clone();
        duplicate_tenant.append(
            TRUSTED_TENANT_ID_HEADER,
            HeaderValue::from_static("tenant-a"),
        );
        assert!(matches!(
            authenticator.authenticate(&duplicate_tenant),
            Err(AuthenticationError::InvalidTenantId)
        ));

        let mut duplicate_user = missing_user.clone();
        duplicate_user.append(TRUSTED_USER_ID_HEADER, HeaderValue::from_static("alice"));
        duplicate_user.append(TRUSTED_USER_ID_HEADER, HeaderValue::from_static("bob"));
        assert!(matches!(
            authenticator.authenticate(&duplicate_user),
            Err(AuthenticationError::InvalidUserId)
        ));

        let mut invalid_user = missing_user;
        invalid_user.insert(TRUSTED_USER_ID_HEADER, HeaderValue::from_static(" alice "));
        assert!(matches!(
            authenticator.authenticate(&invalid_user),
            Err(AuthenticationError::InvalidUserId)
        ));
    }

    #[test]
    fn trusted_user_id_requires_canonical_allowed_tenants() {
        assert!(Authenticator::trusted_user_id(BTreeSet::new()).is_err());
        assert!(Authenticator::trusted_user_id(BTreeSet::from([" tenant-a".into()])).is_err());
    }

    #[test]
    fn missing_tampered_expired_and_wrong_audience_tokens_are_rejected() {
        let now = 1_800_000_000;
        assert!(matches!(
            authenticate_hs256(
                &HeaderMap::new(),
                SECRET,
                "https://identity.internal",
                "lazytt-memory",
                &BTreeSet::from(["tenant-a".into()]),
                now
            ),
            Err(AuthenticationError::Missing)
        ));

        let mut tampered = token(valid_claims(now));
        tampered.push('x');
        assert!(matches!(
            authenticate_hs256(
                &headers(&tampered),
                SECRET,
                "https://identity.internal",
                "lazytt-memory",
                &BTreeSet::from(["tenant-a".into()]),
                now
            ),
            Err(AuthenticationError::Invalid)
        ));

        let mut expired = valid_claims(now);
        expired["iat"] = json!(now - 600);
        expired["exp"] = json!(now - 60);
        assert!(
            authenticate_hs256(
                &headers(&token(expired)),
                SECRET,
                "https://identity.internal",
                "lazytt-memory",
                &BTreeSet::from(["tenant-a".into()]),
                now
            )
            .is_err()
        );
        assert!(
            authenticate_hs256(
                &headers(&token(valid_claims(now))),
                SECRET,
                "https://identity.internal",
                "wrong-audience",
                &BTreeSet::from(["tenant-a".into()]),
                now
            )
            .is_err()
        );

        let mut wrong_tenant = valid_claims(now);
        wrong_tenant["tenant_id"] = json!("tenant-not-allowed");
        assert!(
            authenticate_hs256(
                &headers(&token(wrong_tenant)),
                SECRET,
                "https://identity.internal",
                "lazytt-memory",
                &BTreeSet::from(["tenant-a".into()]),
                now
            )
            .is_err()
        );
    }

    #[test]
    fn jwt_configuration_requires_strong_secret_and_unique_tenants() {
        assert!(
            Authenticator::jwt_hs256(
                b"short".to_vec(),
                "issuer".into(),
                "audience".into(),
                BTreeSet::from(["tenant-a".into()])
            )
            .is_err()
        );
        assert!(parse_tenant_ids("tenant-a,,tenant-b").is_err());
        assert!(parse_tenant_ids("tenant-a,tenant-a").is_err());
        assert_eq!(
            parse_tenant_ids("tenant-b, tenant-a").unwrap(),
            BTreeSet::from(["tenant-a".into(), "tenant-b".into()])
        );
    }

    #[test]
    fn dynamic_tenant_flag_requires_an_explicit_boolean() {
        assert!(parse_boolean_env("FLAG", None, false).is_ok_and(|value| !value));
        assert!(parse_boolean_env("FLAG", Some("true"), false).is_ok_and(|value| value));
        assert!(parse_boolean_env("FLAG", Some("false"), true).is_ok_and(|value| !value));
        assert!(parse_boolean_env("FLAG", Some("1"), false).is_err());
    }
}
