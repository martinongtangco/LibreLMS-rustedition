//! US1 fail-fast gates (spec FR-001 / SC-002): each missing/malformed config
//! item fails fast, naming the item.

use shared::config::{Config, ConfigError};

fn missing_item(vars: &[(&str, Option<&str>)]) -> ConfigError {
    Config::from_map(vars).unwrap_err()
}

#[test]
fn missing_database_url_is_named() {
    let vars: Vec<(&str, Option<&str>)> =
        vec![("VALKEY_URL", Some("redis://x")), ("PORT", Some("1234"))];
    let err = missing_item(&vars);
    assert_eq!(err.item, "DATABASE_URL");
    assert!(err.to_string().contains("DATABASE_URL"));
}

#[test]
fn missing_valkey_url_is_named() {
    let vars: Vec<(&str, Option<&str>)> =
        vec![("DATABASE_URL", Some("mssql://x")), ("PORT", Some("1234"))];
    let err = missing_item(&vars);
    assert_eq!(err.item, "VALKEY_URL");
    assert!(err.to_string().contains("VALKEY_URL"));
}

#[test]
fn malformed_port_is_named() {
    let vars: Vec<(&str, Option<&str>)> = vec![
        ("DATABASE_URL", Some("mssql://x")),
        ("VALKEY_URL", Some("redis://x")),
        ("PORT", Some("not-a-port")),
    ];
    let err = missing_item(&vars);
    assert_eq!(err.item, "PORT");
    assert!(err.to_string().contains("PORT"));
}

#[test]
fn failfast_is_instant_not_a_timeout() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let vars: Vec<(&str, Option<&str>)> = vec![("VALKEY_URL", Some("redis://x"))];
    let _ = Config::from_map(&vars).unwrap_err();
    assert!(
        start.elapsed() < Duration::from_millis(100),
        "fail-fast, never a timeout (SC-002)"
    );
}
