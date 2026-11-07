//! Shared contract shapes (Principle III: modules consume these via `contracts` only).
//!
//! Placeholder shapes with the fields later slices need; populated by their slices.
//! No dependencies beyond serde (data-model.md, T011).

use serde::{Deserialize, Serialize};

/// A person who signs in (data-model.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
    pub theme: String,
}

/// The scope boundary for data and authorization (data-model.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
}

/// Role within an organization (data-model.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
}

/// Membership of a user in an organization (data-model.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Membership {
    pub user_id: String,
    pub organization_id: String,
    pub role: Role,
}

/// A course in the catalog — populated by the Catalog slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Course {
    pub id: String,
    pub title: String,
    pub description: String,
    pub organization_id: String,
}

/// Enrollment state — populated by the Enrollment slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Enrollment {
    pub id: String,
    pub user_id: String,
    pub course_id: String,
    pub organization_id: String,
    pub status: EnrollmentStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnrollmentStatus {
    Active,
    Completed,
}

/// Per-user completion record — populated by the Enrollment slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Completion {
    pub enrollment_id: String,
    pub completed_at: i64,
}

/// SCORM attempt status — the `bag` of the ephemeral session store maps to this
/// (populated by the SCORM slice; data-model.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScormAttemptStatus {
    pub session_id: String,
    pub course_id: String,
    /// SCORM 1.2 `lesson_status` (not started / incomplete / completed / passed /
    /// failed) — the simplified 1.2 scope from the constitution.
    pub state: String,
    pub suspend_data: Option<String>,
    pub score_raw: Option<f64>,
    /// `session_time` as `HH:MM:SS` per SCORM 1.2.
    pub session_time: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_roundtrip_through_json() {
        let u = User {
            id: "u1".into(),
            email: "a@b.c".into(),
            name: "A".into(),
            theme: "default".into(),
        };
        let s = serde_json::to_string(&u).expect("serialize");
        let back: User = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(u, back);

        let a = ScormAttemptStatus {
            session_id: "s1".into(),
            course_id: "c1".into(),
            state: "completed".into(),
            suspend_data: Some("{}".into()),
            score_raw: Some(95.0),
            session_time: Some("00:05:00".into()),
        };
        let s = serde_json::to_string(&a).expect("serialize");
        let back: ScormAttemptStatus = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(a, back);
        assert_eq!(
            Role::Admin,
            serde_json::from_str("\"admin\"").expect("role")
        );
    }
}
