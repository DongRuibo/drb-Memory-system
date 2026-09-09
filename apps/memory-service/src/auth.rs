use std::collections::BTreeSet;
use std::env;

use axum::http::HeaderMap;
use memory_postgres::DemoAuthority;
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

pub const TRUSTED_USER_ID_HEADER: &str = "x-user-id";
pub const TRUSTED_TENANT_ID_HEADER: &str = "x-tenant-id";

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
    #[error("X-User-Id must contain 1 to 256 visible characters")]
    InvalidUserId,
    #[error("X-Tenant-Id must identify an allowed tenant")]
    InvalidTenantId,
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
            _ => Err(AuthenticationError::Configuration(
                "MEMORY_AUTH_MODE must be demo or trusted_user_id".into(),
            )),
        }
    }

    pub fn demo(authority: DemoAuthority) -> Self {
        Self {
            mode: AuthenticationMode::Demo { authority },
        }
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
    if values.next().is_some()
        || value.trim() != value
        || value.is_empty()
        || value.len() > 256
        || value.chars().any(char::is_control)
    {
        return None;
    }
    Some(value)
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
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
    fn tenant_configuration_rejects_empty_or_duplicate_entries() {
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
