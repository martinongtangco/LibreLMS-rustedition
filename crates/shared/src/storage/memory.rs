//! In-memory `Storage` implementation — the deterministic test double
//! (Principle XIII/XVII). Tests create their own state; nothing is shared with
//! any long-lived development database.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::{NewSession, SchemaStatus, SessionRow, Storage, StorageError};
use contracts::{Organization, Role, User};

#[derive(Debug, Default)]
struct Inner {
    schema_ready: bool,
    applied: Vec<String>,
    users: HashMap<String, User>,
    orgs: HashMap<String, Organization>,
    memberships: HashMap<(String, String), Role>,
    sessions: HashMap<String, SessionRow>,
}

/// In-memory storage. `new()` starts empty (schema not ready).
#[derive(Debug, Default, Clone)]
pub struct MemoryStorage {
    inner: std::sync::Arc<Mutex<Inner>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the development user + organization directly (test setup; the real
    /// seeding path is migration `0002_seed.sql`).
    pub fn seed_dev(&self) {
        let mut g = self.inner.lock().expect("poisoned");
        g.users.insert(
            "dev-user".to_string(),
            User {
                id: "dev-user".into(),
                email: "dev@example.com".into(),
                name: "Dev".into(),
                theme: "default".into(),
            },
        );
        g.orgs.insert(
            "dev-org".to_string(),
            Organization {
                id: "dev-org".into(),
                name: "Dev Org".into(),
            },
        );
        g.memberships
            .insert(("dev-user".to_string(), "dev-org".to_string()), Role::Admin);
    }

    /// Test-only user creation (Principle XVII: test-created data is allowed).
    pub fn create_user_for_test(&self, user: contracts::User) {
        self.inner
            .lock()
            .expect("poisoned")
            .users
            .insert(user.id.clone(), user);
    }

    /// Test-only membership upsert (Principle XVII: test-created data is allowed).
    pub fn set_org_role_for_test(&self, user_id: &str, org_id: &str, role: contracts::Role) {
        self.inner
            .lock()
            .expect("poisoned")
            .memberships
            .insert((user_id.to_string(), org_id.to_string()), role);
    }
}
/// Migration scripts (name, sql) — mirrors `migrations/*.sql` for the memory
/// implementation. The memory "apply" records the script and applies its
/// semantic effect (schema flag / seed).

#[async_trait]
impl Storage for MemoryStorage {
    async fn verify(&self) -> Result<(), StorageError> {
        Ok(())
    }

    async fn schema_status(&self) -> Result<SchemaStatus, StorageError> {
        let ready = self.inner.lock().expect("poisoned").schema_ready;
        Ok(if ready {
            SchemaStatus::Ready
        } else {
            SchemaStatus::NotReady
        })
    }

    async fn run_migrations(&self, manifest: &[(&str, &str)]) -> Result<Vec<String>, StorageError> {
        let mut g = self.inner.lock().expect("poisoned");
        let mut applied_now = Vec::new();
        for (name, _sql) in manifest {
            if g.applied.iter().any(|a| a == name) {
                continue; // idempotent (research R1, spec US2.3)
            }
            match *name {
                "0001_base" => g.schema_ready = true,
                "0002_seed" => {
                    g.users.insert(
                        "dev-user".to_string(),
                        User {
                            id: "dev-user".into(),
                            email: "dev@example.com".into(),
                            name: "Dev".into(),
                            theme: "default".into(),
                        },
                    );
                    g.orgs.insert(
                        "dev-org".to_string(),
                        Organization {
                            id: "dev-org".into(),
                            name: "Dev Org".into(),
                        },
                    );
                    g.memberships
                        .insert(("dev-user".to_string(), "dev-org".to_string()), Role::Admin);
                }
                _ => {}
            }
            g.applied.push(name.to_string());
            applied_now.push(name.to_string());
        }
        Ok(applied_now)
    }

    async fn create_session(&self, s: &NewSession) -> Result<(), StorageError> {
        let row = SessionRow {
            token_hash: s.token_hash.clone(),
            user_id: s.user_id.clone(),
            organization_id: s.organization_id.clone(),
            created_at: s.created_at,
            last_activity: s.last_activity,
            expires_at: s.expires_at,
        };
        self.inner
            .lock()
            .expect("poisoned")
            .sessions
            .insert(row.token_hash.clone(), row);
        Ok(())
    }

    async fn get_session(&self, token_hash: &str) -> Result<Option<SessionRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .expect("poisoned")
            .sessions
            .get(token_hash)
            .cloned())
    }

    async fn touch_session(
        &self,
        token_hash: &str,
        now: i64,
        new_expiry: i64,
    ) -> Result<bool, StorageError> {
        let mut g = self.inner.lock().expect("poisoned");
        match g.sessions.get_mut(token_hash) {
            Some(s) => {
                s.last_activity = now;
                s.expires_at = new_expiry;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn delete_session(&self, token_hash: &str) -> Result<(), StorageError> {
        self.inner
            .lock()
            .expect("poisoned")
            .sessions
            .remove(token_hash);
        Ok(())
    }

    async fn get_user(&self, id: &str) -> Result<Option<User>, StorageError> {
        Ok(self.inner.lock().expect("poisoned").users.get(id).cloned())
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StorageError> {
        let g = self.inner.lock().expect("poisoned");
        Ok(g.users.values().find(|u| u.email == email).cloned())
    }

    async fn org_role(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<Option<Role>, StorageError> {
        let g = self.inner.lock().expect("poisoned");
        Ok(g.memberships
            .get(&(user_id.to_string(), organization_id.to_string()))
            .copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let s = MemoryStorage::new();
        let first = s
            .run_migrations(crate::migrate::MIGRATIONS)
            .await
            .expect("migrate");
        assert_eq!(first, vec!["0001_base", "0002_seed"]);
        let second = s
            .run_migrations(crate::migrate::MIGRATIONS)
            .await
            .expect("migrate again");
        assert!(second.is_empty(), "second run must be a no-op");
        assert!(matches!(
            s.schema_status().await.expect("status"),
            SchemaStatus::Ready
        ));
        let user = s
            .get_user_by_email("dev@example.com")
            .await
            .expect("lookup")
            .expect("seeded dev user");
        assert_eq!(user.id, "dev-user");
    }

    #[tokio::test]
    async fn session_lifecycle() {
        let s = MemoryStorage::new();
        s.run_migrations(crate::migrate::MIGRATIONS).await.unwrap();
        let row = NewSession {
            token_hash: "h1".into(),
            user_id: "dev-user".into(),
            organization_id: "dev-org".into(),
            created_at: 100,
            last_activity: 100,
            expires_at: 200,
        };
        s.create_session(&row).await.unwrap();
        assert!(s.touch_session("h1", 150, 160).await.unwrap());
        let got = s.get_session("h1").await.unwrap().expect("exists");
        assert_eq!(got.last_activity, 150);
        assert!(!s.touch_session("missing", 150, 160).await.unwrap());
        s.delete_session("h1").await.unwrap();
        assert!(s.get_session("h1").await.unwrap().is_none());
    }
}
