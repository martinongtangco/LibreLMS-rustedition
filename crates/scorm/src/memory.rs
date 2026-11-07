//! Deterministic doubles (spec 005, T004).
//!
//! `InMemoryScormRepo` is the deterministic double (all SCORM logic
//! testable without containers); `InMemoryBagStore` doubles the 001-
//! reserved-key bag semantics with a `flush_for_test` seam (the FR-006
//! bag-loss scenario); `DoubleEnrollmentPort` is the fault-injecting
//! EnrollmentPort double (the N1 discipline is testable: an outage is
//! `Err(Unavailable)`, never an `Ok(...)`).
//!
//! The sequential duplicate check here is LOGIC-LAYER — the race proof
//! (US1.6) is the full-env test with N independent connections (plan).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::repo::{BagStore, EnrollmentPort, LaunchGate, ScormAttempt, ScormError, ScormRepo};

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Deterministic double (Principle XIII/XVII): all SCORM logic is testable
/// without containers; the full-env test proves the real engine.
#[derive(Default)]
pub struct InMemoryScormRepo {
    /// Keyed by (session, course) — the UNIQUE constraint's double analog.
    rows: Arc<Mutex<HashMap<(String, String), ScormAttempt>>>,
    ids: Arc<std::sync::atomic::AtomicU64>,
    /// Test-only fault injection (the 001/003/004 pattern).
    down: Arc<std::sync::atomic::AtomicBool>,
}

impl InMemoryScormRepo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only fault injection for the 503 path.
    pub fn set_unavailable_for_test(&self, down: bool) {
        self.down.store(down, std::sync::atomic::Ordering::SeqCst);
    }

    fn check(&self) -> Result<(), ScormError> {
        if self.down.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(ScormError::Unavailable);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl ScormRepo for InMemoryScormRepo {
    async fn find_by_session_course(
        &self,
        session_id: &str,
        course_id: &str,
    ) -> Result<Option<ScormAttempt>, ScormError> {
        self.check()?;
        let g = self.rows.lock().await;
        Ok(g.get(&(session_id.to_string(), course_id.to_string()))
            .cloned())
    }

    async fn create(&self, attempt: &ScormAttempt) -> Result<(), ScormError> {
        self.check()?;
        let mut g = self.rows.lock().await;
        let key = (attempt.session_id.clone(), attempt.course_id.clone());
        // LOGIC-LAYER duplicate check (the engine's UNIQUE constraint is the
        // real-arbiter analog — plan US1.6).
        if g.contains_key(&key) {
            return Err(ScormError::AttemptExists);
        }
        let mut row = attempt.clone();
        if row.id.is_empty() {
            let n = self.ids.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            row.id = format!("sc-{n}");
        }
        row.created_at = now_secs();
        g.insert(key, row);
        Ok(())
    }

    async fn update_committed(
        &self,
        id: &str,
        state: &str,
        suspend_data: &str,
        score_raw: Option<i32>,
        session_time: i64,
    ) -> Result<(), ScormError> {
        self.check()?;
        let mut g = self.rows.lock().await;
        let row = g
            .values_mut()
            .find(|r| r.id == id)
            .ok_or(ScormError::NotFound)?;
        row.state = state.to_string();
        row.suspend_data = suspend_data.to_string();
        row.score_raw = score_raw;
        row.session_time = session_time;
        Ok(())
    }

    async fn finalize(
        &self,
        id: &str,
        state: &str,
        suspend_data: &str,
        score_raw: Option<i32>,
        session_time: i64,
    ) -> Result<(), ScormError> {
        self.check()?;
        let mut g = self.rows.lock().await;
        let row = g
            .values_mut()
            .find(|r| r.id == id)
            .ok_or(ScormError::NotFound)?;
        // The finished_at belt (plan): set exactly once — a second finalize
        // is `AttemptFinal`, never a re-record (first-wins; the 004 engine
        // owns the completion record).
        if row.finished_at.is_some() {
            return Err(ScormError::AttemptFinal);
        }
        row.state = state.to_string();
        row.suspend_data = suspend_data.to_string();
        row.score_raw = score_raw;
        row.session_time = session_time;
        row.finished_at = Some(now_secs());
        Ok(())
    }
}

/// The BagStore double (doubles the 001-reserved-key semantics — the key
/// itself never appears here; `flush_for_test` is the FR-006 bag-loss seam).
#[derive(Default)]
pub struct InMemoryBagStore {
    bags: Arc<Mutex<HashMap<String, String>>>,
    down: Arc<std::sync::atomic::AtomicBool>,
}

impl InMemoryBagStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only: simulate a session-store flush (the bag is lost; the
    /// committed state in the repo survives — FR-006 rehydration).
    pub fn flush_for_test(&self) {
        // The Arc<Mutex> is re-locked by the trait methods; this seam
        // bypasses the lock with the same interior API.
        if let Ok(mut g) = self.bags.try_lock() {
            g.clear();
        }
    }

    /// Test-only fault injection for the 503 path.
    pub fn set_unavailable_for_test(&self, down: bool) {
        self.down.store(down, std::sync::atomic::Ordering::SeqCst);
    }

    fn check(&self) -> Result<(), ScormError> {
        if self.down.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(ScormError::Unavailable);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl BagStore for InMemoryBagStore {
    async fn get_bag(&self, token_hash: &str) -> Result<Option<String>, ScormError> {
        self.check()?;
        let g = self.bags.lock().await;
        Ok(g.get(token_hash).cloned())
    }

    async fn set_bag(&self, token_hash: &str, bag: &str, _ttl_secs: u64) -> Result<(), ScormError> {
        self.check()?;
        let mut g = self.bags.lock().await;
        g.insert(token_hash.to_string(), bag.to_string());
        Ok(())
    }
}

/// The (learner, course, org) key for the double's maps.
type PortKey = (String, String, String);

/// The EnrollmentPort double (fault-injecting — the N1 discipline is
/// testable: outage → `Err(Unavailable)`, never `Ok(NotEnrolled)`-as-404/
/// 409; `record_completion` counts calls and is an idempotent no-op after
/// the first — the 004 first-wins double analog).
#[derive(Default)]
pub struct DoubleEnrollmentPort {
    gates: Arc<Mutex<HashMap<PortKey, LaunchGate>>>,
    /// (learner, course, org) → recorded (first-wins double analog).
    completions: Arc<Mutex<HashMap<PortKey, u64>>>,
    down: Arc<std::sync::atomic::AtomicBool>,
}

impl DoubleEnrollmentPort {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only seeding (bypasses nothing — the fixture IS the data).
    pub async fn seed(&self, learner_id: &str, course_id: &str, org_id: &str, gate: LaunchGate) {
        self.gates.lock().await.insert(
            (
                learner_id.to_string(),
                course_id.to_string(),
                org_id.to_string(),
            ),
            gate,
        );
    }

    /// Test-only fault injection for the 503 path.
    pub fn set_unavailable_for_test(&self, down: bool) {
        self.down.store(down, std::sync::atomic::Ordering::SeqCst);
    }

    /// Test-only: how many times `record_completion` was invoked.
    pub async fn completion_calls(&self, learner_id: &str, course_id: &str, org_id: &str) -> u64 {
        self.completions
            .lock()
            .await
            .get(&(
                learner_id.to_string(),
                course_id.to_string(),
                org_id.to_string(),
            ))
            .copied()
            .unwrap_or(0)
    }

    fn check(&self) -> Result<(), ScormError> {
        if self.down.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(ScormError::Unavailable);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl EnrollmentPort for DoubleEnrollmentPort {
    async fn enrollment_status(
        &self,
        learner_id: &str,
        course_id: &str,
        org_id: &str,
    ) -> Result<LaunchGate, ScormError> {
        // Outage discipline (004 N1 parity): outage → Err, never Ok(...).
        self.check()?;
        Ok(self
            .gates
            .lock()
            .await
            .get(&(
                learner_id.to_string(),
                course_id.to_string(),
                org_id.to_string(),
            ))
            .copied()
            .unwrap_or(LaunchGate::NotEnrolled))
    }

    async fn record_completion(
        &self,
        learner_id: &str,
        course_id: &str,
        org_id: &str,
    ) -> Result<(), ScormError> {
        self.check()?;
        // First-wins double analog: the 004 engine's `dbo.completions` PK
        // makes a second call an idempotent no-op — never an error, never a
        // second timestamp.
        let mut g = self.completions.lock().await;
        let entry = g
            .entry((
                learner_id.to_string(),
                course_id.to_string(),
                org_id.to_string(),
            ))
            .or_insert(0);
        *entry += 1;
        Ok(())
    }
}
