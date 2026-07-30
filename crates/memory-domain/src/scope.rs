use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Tenant,
    User,
    Project,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryScope {
    tenant_id: String,
    kind: ScopeKind,
    scope_id: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScopeError {
    #[error("tenant_id must not be empty")]
    EmptyTenant,
    #[error("tenant scope must not have a scope_id")]
    TenantScopeHasId,
    #[error("non-tenant scope requires a scope_id")]
    MissingScopeId,
    #[error("scope_id must not be empty")]
    EmptyScopeId,
}

impl MemoryScope {
    pub fn new(
        tenant_id: impl Into<String>,
        kind: ScopeKind,
        scope_id: Option<String>,
    ) -> Result<Self, ScopeError> {
        let tenant_id = tenant_id.into();
        if tenant_id.trim().is_empty() {
            return Err(ScopeError::EmptyTenant);
        }

        match (kind, scope_id.as_deref()) {
            (ScopeKind::Tenant, Some(_)) => return Err(ScopeError::TenantScopeHasId),
            (ScopeKind::Tenant, None) => {}
            (_, None) => return Err(ScopeError::MissingScopeId),
            (_, Some(value)) if value.trim().is_empty() => return Err(ScopeError::EmptyScopeId),
            (_, Some(_)) => {}
        }

        Ok(Self {
            tenant_id,
            kind,
            scope_id,
        })
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn kind(&self) -> ScopeKind {
        self.kind
    }

    pub fn scope_id(&self) -> Option<&str> {
        self.scope_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_scope_rejects_scope_id() {
        assert_eq!(
            MemoryScope::new("tenant-1", ScopeKind::Tenant, Some("unexpected".into())),
            Err(ScopeError::TenantScopeHasId)
        );
    }

    #[test]
    fn project_scope_requires_scope_id() {
        assert_eq!(
            MemoryScope::new("tenant-1", ScopeKind::Project, None),
            Err(ScopeError::MissingScopeId)
        );
    }

    #[test]
    fn typed_scope_is_valid() {
        let scope = MemoryScope::new("tenant-1", ScopeKind::User, Some("user-7".into()))
            .expect("valid user scope");

        assert_eq!(scope.tenant_id(), "tenant-1");
        assert_eq!(scope.kind(), ScopeKind::User);
        assert_eq!(scope.scope_id(), Some("user-7"));
    }
}
