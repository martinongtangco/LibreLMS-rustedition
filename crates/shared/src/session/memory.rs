//! In-memory `SessionStore` — deterministic test double (Principle XIII/XVII).

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::{bag_key, SessionStore, StoreError};

#[derive(Debug, Clone)]
struct Entry {
    value: String,
    /// Absolute expiry (unix seconds).
    expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct MemorySessionStore {
    inner: std::sync::Arc<Mutex<Inner>>,
}

#[derive(Debug)]
struct Inner {
    entries: HashMap<String, Entry>,
    /// Test knob: simulate the store being down (spec US1.4 → 503 path).
    down: bool,
    /// Wall clock injected by tests (Principle XVII: no dependence on
    /// ambient state).
    now: i64,
}

impl Default for MemorySessionStore {
    fn default() -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(Inner {
                entries: HashMap::new(),
                down: false,
                now: 0,
            })),
        }
    }
}

impl MemorySessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulate the store becoming unreachable (spec US1.4).
    pub fn set_down(&self, down: bool) {
        self.inner.lock().expect("poisoned").down = down;
    }

    /// Advance the test clock.
    pub fn set_now(&self, now: i64) {
        self.inner.lock().expect("poisoned").now = now;
    }
}

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn verify(&self) -> Result<(), StoreError> {
        let g = self.inner.lock().expect("poisoned");
        if g.down {
            return Err(StoreError::Unavailable("simulated outage".to_string()));
        }
        Ok(())
    }

    async fn set_bag(
        &self,
        token_hash: &str,
        value: &str,
        ttl_secs: u64,
    ) -> Result<(), StoreError> {
        let mut g = self.inner.lock().expect("poisoned");
        if g.down {
            return Err(StoreError::Unavailable("simulated outage".to_string()));
        }
        let now = g.now;
        let key = bag_key(token_hash);
        g.entries.insert(
            key,
            Entry {
                value: value.to_string(),
                expires_at: now + ttl_secs as i64,
            },
        );
        Ok(())
    }

    async fn get_bag(&self, token_hash: &str) -> Result<Option<String>, StoreError> {
        let g = self.inner.lock().expect("poisoned");
        if g.down {
            return Err(StoreError::Unavailable("simulated outage".to_string()));
        }
        let key = bag_key(token_hash);
        match g.entries.get(&key) {
            Some(e) if e.expires_at > g.now => Ok(Some(e.value.clone())),
            Some(_) => Ok(None), // expired
            None => Ok(None),
        }
    }

    async fn refresh_ttl(&self, token_hash: &str, ttl_secs: u64) -> Result<(), StoreError> {
        let mut g = self.inner.lock().expect("poisoned");
        if g.down {
            return Err(StoreError::Unavailable("simulated outage".to_string()));
        }
        let now = g.now;
        let key = bag_key(token_hash);
        if let Some(e) = g.entries.get_mut(&key) {
            e.expires_at = now + ttl_secs as i64;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bag_ttl_and_down_behavior() {
        let s = MemorySessionStore::new();
        s.set_bag("h1", "{}", 100).await.unwrap();
        assert_eq!(s.get_bag("h1").await.unwrap().as_deref(), Some("{}"));
        s.set_now(150);
        assert_eq!(s.get_bag("h1").await.unwrap(), None, "expired after TTL");
        s.set_now(100);
        s.refresh_ttl("h1", 100).await.unwrap();
        s.set_now(150);
        assert_eq!(
            s.get_bag("h1").await.unwrap().as_deref(),
            Some("{}"),
            "refreshed"
        );
        s.set_down(true);
        assert!(
            s.get_bag("h1").await.is_err(),
            "down → Unavailable (503 path)"
        );
        assert!(s.verify().await.is_err());
    }
}
