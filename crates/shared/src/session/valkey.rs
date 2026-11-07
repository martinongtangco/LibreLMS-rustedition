//! redis-rs (tokio) `SessionStore` against Valkey (Redis-protocol, Principle VI).
//! Uses a `ConnectionManager` (multiplexed, auto-reconnecting) — one per store.

use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::Client;

use super::{bag_key, SessionStore, StoreError};

/// Valkey-backed ephemeral session store.
#[derive(Clone)]
pub struct ValkeySessionStore {
    manager: ConnectionManager,
}

impl ValkeySessionStore {
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let client = Client::open(url)
            .map_err(|e| StoreError::Unavailable(format!("valkey connect: {e}")))?;
        let manager = client
            .get_connection_manager()
            .await
            .map_err(|e| StoreError::Unavailable(format!("valkey connect: {e}")))?;
        let pong: String = redis::cmd("PING")
            .query_async(&mut manager.clone())
            .await
            .map_err(|e| StoreError::Unavailable(format!("valkey ping: {e}")))?;
        if pong != "PONG" {
            return Err(StoreError::Other("valkey ping mismatch".to_string()));
        }
        Ok(Self { manager })
    }
}

#[async_trait]
impl SessionStore for ValkeySessionStore {
    async fn verify(&self) -> Result<(), StoreError> {
        let pong: String = redis::cmd("PING")
            .query_async(&mut self.manager.clone())
            .await
            .map_err(|e| StoreError::Unavailable(format!("valkey ping: {e}")))?;
        if pong != "PONG" {
            return Err(StoreError::Other("valkey ping mismatch".to_string()));
        }
        Ok(())
    }

    async fn set_bag(
        &self,
        token_hash: &str,
        value: &str,
        ttl_secs: u64,
    ) -> Result<(), StoreError> {
        let key = bag_key(token_hash);
        let set: String = redis::cmd("SET")
            .arg(&key)
            .arg(value)
            .arg("EX")
            .arg(ttl_secs)
            .query_async(&mut self.manager.clone())
            .await
            .map_err(|e| StoreError::Unavailable(format!("valkey set: {e}")))?;
        if set != "OK" {
            return Err(StoreError::Other("valkey set returned non-OK".to_string()));
        }
        Ok(())
    }

    async fn get_bag(&self, token_hash: &str) -> Result<Option<String>, StoreError> {
        let key = bag_key(token_hash);
        let v: Option<String> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut self.manager.clone())
            .await
            .map_err(|e| StoreError::Unavailable(format!("valkey get: {e}")))?;
        Ok(v)
    }

    async fn refresh_ttl(&self, token_hash: &str, ttl_secs: u64) -> Result<(), StoreError> {
        let key = bag_key(token_hash);
        let applied: i64 = redis::cmd("EXPIRE")
            .arg(&key)
            .arg(ttl_secs)
            .query_async(&mut self.manager.clone())
            .await
            .map_err(|e| StoreError::Unavailable(format!("valkey expire: {e}")))?;
        // 0 = key absent; not an error (bag may legitimately not exist yet).
        let _ = applied;
        Ok(())
    }
}
