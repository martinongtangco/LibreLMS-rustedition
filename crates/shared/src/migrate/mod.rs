//! Ordered SQL migration manifest (research R1).
//!
//! Both `Storage` implementations apply this manifest: idempotent by design — a
//! script already recorded in `schema_migrations` is skipped. Checksums keep the
//! bookkeeping honest (spec US2.3, data-model.md).

use sha2::{Digest, Sha256};

/// (name, sql) in application order.
pub const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_base",
        include_str!("../../../../migrations/0001_base.sql"),
    ),
    (
        "0002_seed",
        include_str!("../../../../migrations/0002_seed.sql"),
    ),
];

/// SHA-256 hex of a script (the `schema_migrations.checksum` column).
pub fn checksum(sql: &str) -> String {
    let mut h = Sha256::new();
    h.update(sql.as_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_ordered_and_checksummed() {
        assert_eq!(MIGRATIONS[0].0, "0001_base");
        assert_eq!(MIGRATIONS[1].0, "0002_seed");
        let a = checksum("SELECT 1");
        let b = checksum("SELECT 1");
        let c = checksum("SELECT 2");
        assert_eq!(a.len(), 64);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
