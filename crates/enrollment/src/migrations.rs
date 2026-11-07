//! Enrollment slice migration manifest (spec 001 Module Persistence Seam).
//!
//! The host concatenates this with `shared::migrate::MIGRATIONS` and every
//! other module's manifest (name-ordered) and passes the result to the
//! manifest-generic runner (spec 001 CR-2).

/// (name, sql) in application order.
pub const MIGRATIONS: &[(&str, &str)] = &[(
    "0005_enrollment",
    include_str!("../../../migrations/0005_enrollment.sql"),
)];
