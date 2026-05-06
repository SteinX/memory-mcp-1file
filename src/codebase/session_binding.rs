use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBinding {
    pub session_id: String,
    pub project_id: Option<String>,
    pub updated_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBindingStatus {
    pub session_id: String,
    pub project_id: Option<String>,
    pub updated_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SessionBindingStore {
    bindings: Arc<RwLock<HashMap<String, SessionBinding>>>,
    max_entries: usize,
}

impl SessionBindingStore {
    pub fn new(max_entries: usize) -> Self {
        Self {
            bindings: Arc::new(RwLock::new(HashMap::new())),
            max_entries,
        }
    }

    pub async fn bind(&self, session_id: impl Into<String>, project_id: impl Into<String>) {
        let session_id = session_id.into();
        let project_id = project_id.into();
        let mut bindings = self.bindings.write().await;
        let updated_at_unix_ms = next_timestamp_ms(&bindings);

        bindings.insert(
            session_id.clone(),
            SessionBinding {
                session_id,
                project_id: Some(project_id),
                updated_at_unix_ms,
            },
        );

        trim_oldest(&mut bindings, self.max_entries);
    }

    pub async fn unbind(&self, session_id: &str) {
        self.bindings.write().await.remove(session_id);
    }

    pub async fn binding_status(&self, session_id: &str) -> SessionBindingStatus {
        let bindings = self.bindings.read().await;
        match bindings.get(session_id) {
            Some(binding) => SessionBindingStatus {
                session_id: session_id.to_string(),
                project_id: binding.project_id.clone(),
                updated_at_unix_ms: Some(binding.updated_at_unix_ms),
            },
            None => SessionBindingStatus {
                session_id: session_id.to_string(),
                project_id: None,
                updated_at_unix_ms: None,
            },
        }
    }

    pub async fn prune_oldest(&self) {
        let mut bindings = self.bindings.write().await;
        trim_oldest(&mut bindings, self.max_entries);
    }

    pub async fn len(&self) -> usize {
        self.bindings.read().await.len()
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn next_timestamp_ms(bindings: &HashMap<String, SessionBinding>) -> u64 {
    let now = now_unix_ms();
    let latest = bindings
        .values()
        .map(|binding| binding.updated_at_unix_ms)
        .max()
        .unwrap_or(0);

    if now > latest {
        now
    } else {
        latest.saturating_add(1)
    }
}

fn trim_oldest(bindings: &mut HashMap<String, SessionBinding>, max_entries: usize) {
    if max_entries == 0 {
        bindings.clear();
        return;
    }

    while bindings.len() > max_entries {
        let oldest_session_id = bindings
            .iter()
            .min_by_key(|(session_id, binding)| (binding.updated_at_unix_ms, session_id.as_str()))
            .map(|(session_id, _)| session_id.clone());

        if let Some(session_id) = oldest_session_id {
            bindings.remove(&session_id);
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_binding_bind_rebind() {
        let store = SessionBindingStore::new(8);

        store.bind("s-alpha", "project-a").await;
        store.bind("s-alpha", "project-b").await;

        let status = store.binding_status("s-alpha").await;
        assert_eq!(status.session_id, "s-alpha");
        assert_eq!(status.project_id.as_deref(), Some("project-b"));
        assert!(status.updated_at_unix_ms.is_some());
    }

    #[tokio::test]
    async fn session_binding_unbind() {
        let store = SessionBindingStore::new(8);

        store.bind("s-alpha", "project-a").await;
        store.bind("s-beta", "project-b").await;
        store.unbind("s-alpha").await;

        let alpha_status = store.binding_status("s-alpha").await;
        let beta_status = store.binding_status("s-beta").await;

        assert_eq!(alpha_status.session_id, "s-alpha");
        assert!(alpha_status.project_id.is_none());
        assert!(alpha_status.updated_at_unix_ms.is_none());
        assert_eq!(beta_status.project_id.as_deref(), Some("project-b"));
    }

    #[tokio::test]
    async fn session_binding_bounded_prune() {
        let store = SessionBindingStore::new(2);

        store.bind("s-1", "project-a").await;
        store.bind("s-2", "project-b").await;
        store.bind("s-3", "project-c").await;

        store.prune_oldest().await;

        assert_eq!(store.len().await, 2);
        assert!(store.binding_status("s-1").await.project_id.is_none());
    }

    #[tokio::test]
    async fn session_binding_cleanup_prunes_expired() {
        let store = SessionBindingStore::new(2);

        store.bind("s-old", "project-old").await;
        store.bind("s-active-a", "project-active-a").await;
        store.bind("s-active-b", "project-active-b").await;
        store.prune_oldest().await;

        assert_eq!(store.len().await, 2);
        assert!(store.binding_status("s-old").await.project_id.is_none());
        assert_eq!(
            store
                .binding_status("s-active-a")
                .await
                .project_id
                .as_deref(),
            Some("project-active-a")
        );
        assert_eq!(
            store
                .binding_status("s-active-b")
                .await
                .project_id
                .as_deref(),
            Some("project-active-b")
        );
    }

    #[tokio::test]
    async fn session_binding_is_session_keyed() {
        let store = SessionBindingStore::new(8);

        store.bind("s-alpha", "project-a").await;
        store.bind("s-beta", "project-b").await;

        assert_eq!(
            store.binding_status("s-alpha").await.project_id.as_deref(),
            Some("project-a")
        );
        assert_eq!(
            store.binding_status("s-beta").await.project_id.as_deref(),
            Some("project-b")
        );
        assert!(store.binding_status("s-gamma").await.project_id.is_none());
    }
}
