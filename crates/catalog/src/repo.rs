//! Course repository (spec 003, Module Persistence Seam).
//!
//! The trait is defined IN THIS crate (the seam decision from spec 001 CR-2);
//! `InMemoryCourseRepo` is the deterministic double, `TiberiusCourseRepo` (T017)
//! is the real engine. Scope/role rules live here, not in the routes.

use std::collections::HashMap;
use std::sync::Arc;

use contracts::Course;
use tokio::sync::Mutex;

/// Catalog failures → envelope mapping (host): 404 / 403 / 403 / 400(field) / 503.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// Unknown course id (genuinely absent — 404, plain body, no internals).
    NotFound,
    /// The course exists but belongs to another organization (403 — the
    /// no-existence-oracle rule: out-of-scope reads answer 403, not 404).
    Scope,
    /// The session's role does not allow the action (403).
    Forbidden,
    /// Invalid input; the message names the offending field (400).
    Validation {
        field: &'static str,
        message: String,
    },
    /// The system of record is unreachable (503 — never 500, never a silent
    /// empty list).
    Unavailable,
}

/// The course repository seam (in-crate per the Module Persistence Seam).
#[async_trait::async_trait]
pub trait CourseRepo: Send + Sync {
    /// List a session organization's courses, optionally filtered by a
    /// case-insensitive substring of the title. Empty results are Ok(…).
    async fn list_for(
        &self,
        org: &str,
        title_filter: Option<&str>,
    ) -> Result<Vec<Course>, CatalogError>;

    /// Get one course; `Scope` if it exists in another org, `NotFound` if absent.
    async fn get(&self, course_id: &str, session_org: &str) -> Result<Course, CatalogError>;

    /// Admin: create a course in the session's organization (validation: title
    /// non-empty ≤ 200 chars; description ≤ 5,000).
    async fn create(
        &self,
        session_org: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<Course, CatalogError>;

    /// Admin: update title and/or description of a course of the session's org.
    async fn update(
        &self,
        course_id: &str,
        session_org: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<Course, CatalogError>;

    /// Admin: delete a course of the session's org.
    async fn delete(&self, course_id: &str, session_org: &str) -> Result<(), CatalogError>;
}

/// Deterministic double (Principle VIII/XIII): all catalog logic is testable
/// without containers; the full-env test (T017) proves the real engine.
#[derive(Default)]
pub struct InMemoryCourseRepo {
    courses: Arc<Mutex<HashMap<String, Course>>>,
    /// Test-only fault injection (the 001 MemorySessionStore pattern): lets the
    /// deterministic suite exercise the 503 path without a real outage.
    down: Arc<std::sync::atomic::AtomicBool>,
}

impl InMemoryCourseRepo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only seeding (bypasses validation by design — fixtures are trusted).
    pub async fn create_for_test(&self, course: Course) -> Result<(), CatalogError> {
        self.courses.lock().await.insert(course.id.clone(), course);
        Ok(())
    }

    /// Test-only fault injection for the 503 path (spec 003 FR-008).
    pub fn set_unavailable_for_test(&self, down: bool) {
        self.down.store(down, std::sync::atomic::Ordering::SeqCst);
    }

    fn is_down(&self) -> bool {
        self.down.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Validation rules (SC-005 boundary battery).
const MAX_TITLE_LEN: usize = 200;
const MAX_DESCRIPTION_LEN: usize = 5_000;

pub fn validate_title(title: &str) -> Result<(), CatalogError> {
    if title.trim().is_empty() {
        return Err(CatalogError::Validation {
            field: "title",
            message: "title must be non-empty".into(),
        });
    }
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(CatalogError::Validation {
            field: "title",
            message: format!("title must be at most {MAX_TITLE_LEN} characters"),
        });
    }
    Ok(())
}

pub fn validate_description(description: &str) -> Result<(), CatalogError> {
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(CatalogError::Validation {
            field: "description",
            message: format!("description must be at most {MAX_DESCRIPTION_LEN} characters"),
        });
    }
    Ok(())
}

/// Scope rule (FR-006 / story 3.2): existing-but-foreign → Scope (403),
/// genuinely absent → NotFound (404). Never an existence oracle.
fn scope_check(course: &Course, session_org: &str) -> Result<(), CatalogError> {
    if course.organization_id != session_org {
        return Err(CatalogError::Scope);
    }
    Ok(())
}

#[async_trait::async_trait]
impl CourseRepo for InMemoryCourseRepo {
    async fn list_for(
        &self,
        org: &str,
        title_filter: Option<&str>,
    ) -> Result<Vec<Course>, CatalogError> {
        if self.is_down() {
            return Err(CatalogError::Unavailable);
        }
        let needle = title_filter.map(str::to_lowercase);
        let g = self.courses.lock().await;
        let mut out: Vec<Course> = g
            .values()
            .filter(|c| c.organization_id == org)
            .filter(|c| {
                needle
                    .as_ref()
                    .is_none_or(|n| n.is_empty() || c.title.to_lowercase().contains(n))
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    async fn get(&self, course_id: &str, session_org: &str) -> Result<Course, CatalogError> {
        if self.is_down() {
            return Err(CatalogError::Unavailable);
        }
        let g = self.courses.lock().await;
        let course = g.get(course_id).cloned().ok_or(CatalogError::NotFound)?;
        scope_check(&course, session_org)?;
        Ok(course)
    }

    async fn create(
        &self,
        session_org: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<Course, CatalogError> {
        if self.is_down() {
            return Err(CatalogError::Unavailable);
        }
        validate_title(title)?;
        if let Some(d) = description {
            validate_description(d)?;
        }
        let mut g = self.courses.lock().await;
        // Ids: deterministic counter within the double (real engine: NEWID()).
        let next = g
            .values()
            .filter(|c| c.organization_id == session_org)
            .count()
            + 1;
        let course = Course {
            id: format!("mem-{session_org}-{next}"),
            title: title.to_string(),
            description: description.unwrap_or_default().to_string(),
            organization_id: session_org.to_string(),
        };
        g.insert(course.id.clone(), course.clone());
        Ok(course)
    }

    async fn update(
        &self,
        course_id: &str,
        session_org: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<Course, CatalogError> {
        if self.is_down() {
            return Err(CatalogError::Unavailable);
        }
        let mut g = self.courses.lock().await;
        let course = g.get_mut(course_id).ok_or(CatalogError::NotFound)?;
        scope_check(course, session_org)?;
        if let Some(t) = title {
            validate_title(t)?;
            course.title = t.to_string();
        }
        if let Some(d) = description {
            validate_description(d)?;
            course.description = d.to_string();
        }
        Ok(course.clone())
    }

    async fn delete(&self, course_id: &str, session_org: &str) -> Result<(), CatalogError> {
        if self.is_down() {
            return Err(CatalogError::Unavailable);
        }
        let mut g = self.courses.lock().await;
        let course = g.get(course_id).ok_or(CatalogError::NotFound)?;
        scope_check(course, session_org)?;
        g.remove(course_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_boundaries() {
        assert!(validate_title("ok").is_ok());
        assert!(validate_title("").is_err());
        assert!(validate_title("  ").is_err());
        assert!(
            validate_title(&"x".repeat(200)).is_ok(),
            "200 is the boundary — accepted"
        );
        assert!(validate_title(&"x".repeat(201)).is_err(), "201 rejected");
    }

    #[test]
    fn description_boundaries() {
        assert!(
            validate_description(&"x".repeat(5_000)).is_ok(),
            "5,000 accepted"
        );
        assert!(
            validate_description(&"x".repeat(5_001)).is_err(),
            "5,001 rejected"
        );
    }

    #[test]
    fn scope_check_maps_foreign_to_scope_and_absent_is_not_found() {
        let c = Course {
            id: "1".into(),
            title: "t".into(),
            description: String::new(),
            organization_id: "org-b".into(),
        };
        assert_eq!(scope_check(&c, "org-a"), Err(CatalogError::Scope));
        assert_eq!(scope_check(&c, "org-b"), Ok(()));
    }
}
