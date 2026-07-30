use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue, header};
use chrono::{DateTime, Utc};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

pub const DEMO_VIEWER_COOKIE: &str = "lazytt_demo_viewer";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DemoUserProfile {
    pub user_id: String,
    pub display_name: String,
    pub role_label: String,
    pub avatar_initial: String,
    pub project_ids: Vec<String>,
    pub primary_project_id: String,
    pub primary_project_label: String,
    pub can_reset_project: bool,
}

#[derive(Clone)]
pub struct DemoUserRegistry {
    users: Arc<Vec<DemoUserProfile>>,
    by_id: Arc<HashMap<String, DemoUserProfile>>,
    default_user_id: String,
}

impl DemoUserRegistry {
    pub fn standard() -> Self {
        let users = vec![
            DemoUserProfile {
                user_id: "demo-alice".into(),
                display_name: "Alice".into(),
                role_label: "Agent 开发工程师".into(),
                avatar_initial: "A".into(),
                project_ids: vec!["lazytt-memory-system-demo".into()],
                primary_project_id: "lazytt-memory-system-demo".into(),
                primary_project_label: "LazyTT 项目".into(),
                can_reset_project: true,
            },
            DemoUserProfile {
                user_id: "demo-bob".into(),
                display_name: "Bob".into(),
                role_label: "平台工程师".into(),
                avatar_initial: "B".into(),
                project_ids: vec!["lazytt-memory-system-demo".into()],
                primary_project_id: "lazytt-memory-system-demo".into(),
                primary_project_label: "LazyTT 项目".into(),
                can_reset_project: false,
            },
            DemoUserProfile {
                user_id: "demo-carol".into(),
                display_name: "Carol".into(),
                role_label: "外部协作者".into(),
                avatar_initial: "C".into(),
                project_ids: vec!["external-sandbox-demo".into()],
                primary_project_id: "external-sandbox-demo".into(),
                primary_project_label: "外部沙盒项目".into(),
                can_reset_project: false,
            },
        ];
        let by_id = users
            .iter()
            .cloned()
            .map(|user| (user.user_id.clone(), user))
            .collect();
        Self {
            users: Arc::new(users),
            by_id: Arc::new(by_id),
            default_user_id: "demo-alice".into(),
        }
    }

    pub fn users(&self) -> &[DemoUserProfile] {
        self.users.as_slice()
    }

    pub fn get(&self, user_id: &str) -> Option<DemoUserProfile> {
        self.by_id.get(user_id).cloned()
    }

    pub fn default_user(&self) -> DemoUserProfile {
        self.get(&self.default_user_id)
            .expect("standard demo registry must contain its default user")
    }
}

#[derive(Debug, Clone)]
struct ViewerSession {
    user_id: String,
    revision: u64,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ResolvedDemoIdentity {
    pub viewer_session_id: Uuid,
    pub revision: u64,
    pub profile: DemoUserProfile,
}

#[derive(Clone)]
pub struct DemoViewerSessions {
    sessions: Arc<Mutex<HashMap<Uuid, ViewerSession>>>,
    lifetime: Duration,
    secure_cookie: bool,
}

impl DemoViewerSessions {
    pub fn new(secure_cookie: bool) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            lifetime: Duration::from_secs(8 * 60 * 60),
            secure_cookie,
        }
    }

    pub fn resolve(
        &self,
        headers: &HeaderMap,
        registry: &DemoUserRegistry,
    ) -> (ResolvedDemoIdentity, Option<HeaderValue>) {
        let now = Utc::now();
        let requested_id =
            cookie_value(headers, DEMO_VIEWER_COOKIE).and_then(|value| Uuid::parse_str(value).ok());
        let mut sessions = self
            .sessions
            .lock()
            .expect("demo viewer session mutex must not be poisoned");
        sessions.retain(|_, session| session.expires_at > now);

        if let Some(session_id) = requested_id
            && let Some(session) = sessions.get_mut(&session_id)
            && let Some(profile) = registry.get(&session.user_id)
        {
            session.expires_at = expires_at(now, self.lifetime);
            return (
                ResolvedDemoIdentity {
                    viewer_session_id: session_id,
                    revision: session.revision,
                    profile,
                },
                None,
            );
        }

        let session_id = Uuid::new_v4();
        let profile = registry.default_user();
        sessions.insert(
            session_id,
            ViewerSession {
                user_id: profile.user_id.clone(),
                revision: 1,
                expires_at: expires_at(now, self.lifetime),
            },
        );
        (
            ResolvedDemoIdentity {
                viewer_session_id: session_id,
                revision: 1,
                profile,
            },
            Some(self.cookie_header(session_id)),
        )
    }

    pub fn switch(
        &self,
        viewer_session_id: Uuid,
        user_id: &str,
        registry: &DemoUserRegistry,
    ) -> Result<ResolvedDemoIdentity, DemoIdentityError> {
        let profile = registry
            .get(user_id)
            .ok_or(DemoIdentityError::UnknownUser)?;
        let mut sessions = self
            .sessions
            .lock()
            .expect("demo viewer session mutex must not be poisoned");
        let session = sessions
            .get_mut(&viewer_session_id)
            .ok_or(DemoIdentityError::ExpiredViewerSession)?;
        if session.user_id != profile.user_id {
            session.user_id = profile.user_id.clone();
            session.revision = session.revision.saturating_add(1);
        }
        session.expires_at = expires_at(Utc::now(), self.lifetime);
        Ok(ResolvedDemoIdentity {
            viewer_session_id,
            revision: session.revision,
            profile,
        })
    }

    fn cookie_header(&self, session_id: Uuid) -> HeaderValue {
        let secure = if self.secure_cookie { "; Secure" } else { "" };
        HeaderValue::from_str(&format!(
            "{DEMO_VIEWER_COOKIE}={session_id}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}{}",
            self.lifetime.as_secs(),
            secure
        ))
        .expect("generated demo viewer cookie must be a valid header value")
    }
}

#[derive(Debug, Error)]
pub enum DemoIdentityError {
    #[error("the selected demo user is not available")]
    UnknownUser,
    #[error("the browser demo session expired; reload the page and try again")]
    ExpiredViewerSession,
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|entry| {
            let (cookie_name, value) = entry.split_once('=')?;
            (cookie_name == name).then_some(value)
        })
}

fn expires_at(now: DateTime<Utc>, lifetime: Duration) -> DateTime<Utc> {
    now + chrono::Duration::seconds(lifetime.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_registry_models_personal_and_project_boundaries() {
        let registry = DemoUserRegistry::standard();
        let alice = registry.get("demo-alice").unwrap();
        let bob = registry.get("demo-bob").unwrap();
        let carol = registry.get("demo-carol").unwrap();
        assert_eq!(alice.project_ids, bob.project_ids);
        assert_ne!(alice.project_ids, carol.project_ids);
        assert!(alice.can_reset_project);
        assert!(!bob.can_reset_project);
        assert!(!carol.can_reset_project);
    }

    #[test]
    fn switching_changes_the_resolved_principal_and_revision() {
        let registry = DemoUserRegistry::standard();
        let sessions = DemoViewerSessions::new(false);
        let (initial, cookie) = sessions.resolve(&HeaderMap::new(), &registry);
        assert_eq!(initial.profile.user_id, "demo-alice");
        assert!(cookie.is_some());

        let switched = sessions
            .switch(initial.viewer_session_id, "demo-bob", &registry)
            .unwrap();
        assert_eq!(switched.profile.user_id, "demo-bob");
        assert_eq!(switched.revision, initial.revision + 1);
    }
}
