use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex};
use tracing::{info, warn};

pub struct PendingRequest {
    pub id: u64,
    pub description: String,
    pub sender: oneshot::Sender<bool>,
}

type PendingEntry = (String, oneshot::Sender<bool>);

#[derive(Clone)]
pub struct ConfirmationManager {
    counter: Arc<AtomicU64>,
    pending: Arc<Mutex<HashMap<u64, PendingEntry>>>,
    notifier: tokio::sync::broadcast::Sender<serde_json::Value>,
}

impl ConfirmationManager {
    pub fn new() -> Self {
        let (notifier, _) = tokio::sync::broadcast::channel(32);
        Self {
            counter: Arc::new(AtomicU64::new(1)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            notifier,
        }
    }

    /// Subscribes to outgoing HUD confirmation request broadcasts.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<serde_json::Value> {
        self.notifier.subscribe()
    }

    /// Returns a clone of the outgoing HUD broadcast sender.
    pub fn notifier(&self) -> tokio::sync::broadcast::Sender<serde_json::Value> {
        self.notifier.clone()
    }

    /// Broadcasts an arbitrary HUD notification down the Named Pipe.
    pub fn notify(&self, value: serde_json::Value) {
        let _ = self.notifier.send(value);
    }

    /// Allocates a new pending confirmation request and waits for approval.
    /// Safely defaults to `false` (DENIED) on timeout or drop.
    pub async fn request_confirmation(&self, description: &str, timeout_ms: u64) -> bool {
        let id = self.counter.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        {
            let mut guard = self.pending.lock().await;
            guard.insert(id, (description.to_string(), tx));
        }

        info!(
            "Amber action [ID: {}] awaiting user confirmation: \"{}\" (timeout: {} ms)",
            id, description, timeout_ms
        );

        // Broadcast to C++ HUD via Named Pipe IPC
        let notice = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "hud.show_confirmation",
            "params": {
                "id": id,
                "command": description,
                "timeout_ms": timeout_ms
            }
        });
        let _ = self.notifier.send(notice);

        let timeout_dur = Duration::from_millis(timeout_ms);
        match tokio::time::timeout(timeout_dur, rx).await {
            Ok(Ok(approved)) => {
                info!("Confirmation [ID: {}] resolved: approved={}", id, approved);
                approved
            }
            Ok(Err(_)) => {
                warn!("Confirmation [ID: {}] channel dropped, default DENY", id);
                let mut guard = self.pending.lock().await;
                guard.remove(&id);
                false
            }
            Err(_) => {
                warn!(
                    "Confirmation [ID: {}] timed out after {} ms, safe DENY",
                    id, timeout_ms
                );
                let mut guard = self.pending.lock().await;
                guard.remove(&id);
                false
            }
        }
    }

    /// Resolves an existing pending confirmation by ID.
    /// Returns true if the ID existed and was resolved, false otherwise.
    pub async fn resolve_confirmation(&self, id: u64, approved: bool) -> bool {
        let mut guard = self.pending.lock().await;
        if let Some((desc, sender)) = guard.remove(&id) {
            info!("Resolving confirmation [ID: {}] (\"{}\") -> {}", id, desc, approved);
            let _ = sender.send(approved);
            true
        } else {
            warn!("Attempted to resolve non-existent or expired confirmation ID {}", id);
            false
        }
    }

    /// Lists currently active confirmation IDs and their descriptions.
    pub async fn list_pending(&self) -> Vec<(u64, String)> {
        let guard = self.pending.lock().await;
        guard
            .iter()
            .map(|(&id, (desc, _))| (id, desc.clone()))
            .collect()
    }
}

impl Default for ConfirmationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_confirmation_approval() {
        let mgr = ConfirmationManager::new();
        let mgr_clone = mgr.clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let pending = mgr_clone.list_pending().await;
            assert_eq!(pending.len(), 1);
            let id = pending[0].0;
            assert!(mgr_clone.resolve_confirmation(id, true).await);
        });

        let approved = mgr.request_confirmation("git commit -m 'test'", 1000).await;
        assert!(approved);
    }

    #[tokio::test]
    async fn test_confirmation_denial() {
        let mgr = ConfirmationManager::new();
        let mgr_clone = mgr.clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let pending = mgr_clone.list_pending().await;
            assert_eq!(pending.len(), 1);
            let id = pending[0].0;
            assert!(mgr_clone.resolve_confirmation(id, false).await);
        });

        let approved = mgr.request_confirmation("rm -rf target", 1000).await;
        assert!(!approved);
    }

    #[tokio::test]
    async fn test_confirmation_timeout_defaults_to_false() {
        let mgr = ConfirmationManager::new();

        // 100 ms timeout with no resolver
        let approved = mgr.request_confirmation("git push --force", 100).await;
        assert!(!approved);

        let pending = mgr.list_pending().await;
        assert!(pending.is_empty());
    }
}
