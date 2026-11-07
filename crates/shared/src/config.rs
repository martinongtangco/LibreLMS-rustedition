//! Environment configuration with fail-fast validation (spec FR-001).
//!
//! Missing or malformed required items cause startup to fail within 5 seconds with
//! a message naming the offending item — never a stack trace, never a timeout.

/// Parsed application configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub valkey_url: String,
    pub port: u16,
    pub session_ttl_secs: u64,
    pub session_max_ttl_secs: u64,
    pub log_level: String,
}

/// Error with the offending item named (spec FR-001, SC-002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// Name of the configuration item that is missing or malformed.
    pub item: &'static str,
    /// Why it failed ("missing", "empty", or a short malformation description).
    pub reason: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "configuration error: {} is {}", self.item, self.reason)
    }
}

impl std::error::Error for ConfigError {}

fn require_str(item: &'static str, val: Option<&str>) -> Result<String, ConfigError> {
    match val {
        Some(v) if !v.trim().is_empty() => Ok(v.to_string()),
        Some(_) => Err(ConfigError {
            item,
            reason: "empty".to_string(),
        }),
        None => Err(ConfigError {
            item,
            reason: "missing".to_string(),
        }),
    }
}

fn map_u64_or(
    vars: &[(&str, Option<&str>)],
    item: &'static str,
    default: u64,
) -> Result<u64, ConfigError> {
    match vars.iter().find(|(n, _)| *n == item).and_then(|(_, v)| *v) {
        Some(v) if !v.trim().is_empty() => v.parse::<u64>().map_err(|_| ConfigError {
            item,
            reason: "not a valid positive integer".to_string(),
        }),
        _ => Ok(default),
    }
}

impl Config {
    /// Parse configuration from the process environment. Fails fast naming the
    /// first missing/malformed required item. `PORT` is optional (default 5000).
    pub fn from_env() -> Result<Config, ConfigError> {
        let database_url = require_str(
            "DATABASE_URL",
            std::env::var("DATABASE_URL").ok().as_deref(),
        )?;
        let valkey_url = require_str("VALKEY_URL", std::env::var("VALKEY_URL").ok().as_deref())?;
        let port = match std::env::var("PORT") {
            Ok(v) if !v.trim().is_empty() => v.parse::<u16>().map_err(|_| ConfigError {
                item: "PORT",
                reason: "not a valid port number".to_string(),
            })?,
            _ => 5000,
        };
        let env_u64_or = |item: &'static str, default: u64| -> Result<u64, ConfigError> {
            match std::env::var(item) {
                Ok(v) if !v.trim().is_empty() => v.parse::<u64>().map_err(|_| ConfigError {
                    item,
                    reason: "not a valid positive integer".to_string(),
                }),
                _ => Ok(default),
            }
        };
        let session_ttl_secs = env_u64_or("SESSION_TTL_SECS", 28_800)?;
        let session_max_ttl_secs = env_u64_or("SESSION_MAX_TTL_SECS", 604_800)?;
        if session_max_ttl_secs < session_ttl_secs {
            return Err(ConfigError {
                item: "SESSION_MAX_TTL_SECS",
                reason: "must be >= SESSION_TTL_SECS".to_string(),
            });
        }
        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
        Ok(Config {
            database_url,
            valkey_url,
            port,
            session_ttl_secs,
            session_max_ttl_secs,
            log_level,
        })
    }

    /// Parse configuration from an explicit variable map (used by fail-fast tests
    /// — Principle XVII: tests create their own state, never depend on process env).
    pub fn from_map(vars: &[(&str, Option<&str>)]) -> Result<Config, ConfigError> {
        let get = |k: &str| -> Option<&str> {
            vars.iter()
                .find(|(name, _)| *name == k)
                .and_then(|(_, v)| *v)
        };
        let database_url = require_str("DATABASE_URL", get("DATABASE_URL"))?;
        let valkey_url = require_str("VALKEY_URL", get("VALKEY_URL"))?;
        let port = match get("PORT") {
            Some(v) if !v.trim().is_empty() => v.parse::<u16>().map_err(|_| ConfigError {
                item: "PORT",
                reason: "not a valid port number".to_string(),
            })?,
            _ => 5000,
        };
        let session_ttl_secs = map_u64_or(vars, "SESSION_TTL_SECS", 28_800)?;
        let session_max_ttl_secs = map_u64_or(vars, "SESSION_MAX_TTL_SECS", 604_800)?;
        if session_max_ttl_secs < session_ttl_secs {
            return Err(ConfigError {
                item: "SESSION_MAX_TTL_SECS",
                reason: "must be >= SESSION_TTL_SECS".to_string(),
            });
        }
        let log_level = get("LOG_LEVEL").unwrap_or("info").to_string();
        Ok(Config {
            database_url,
            valkey_url,
            port,
            session_ttl_secs,
            session_max_ttl_secs,
            log_level,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Vec<(&'static str, Option<&'static str>)> {
        vec![
            ("DATABASE_URL", Some("mssql://localhost/LibreLms")),
            ("VALKEY_URL", Some("redis://localhost:6379")),
        ]
    }

    #[test]
    fn valid_config_parses_with_defaults() {
        let cfg = Config::from_map(&base()).expect("valid");
        assert_eq!(cfg.port, 5000);
        assert_eq!(cfg.session_ttl_secs, 28_800);
        assert_eq!(cfg.session_max_ttl_secs, 604_800);
    }

    #[test]
    fn missing_database_url_is_named() {
        let vars: Vec<(&str, Option<&str>)> = vec![("VALKEY_URL", Some("redis://localhost:6379"))];
        let err = Config::from_map(&vars).unwrap_err();
        assert_eq!(err.item, "DATABASE_URL");
        assert!(err.to_string().contains("DATABASE_URL"));
    }

    #[test]
    fn missing_valkey_url_is_named() {
        let vars: Vec<(&str, Option<&str>)> =
            vec![("DATABASE_URL", Some("mssql://localhost/LibreLms"))];
        let err = Config::from_map(&vars).unwrap_err();
        assert_eq!(err.item, "VALKEY_URL");
    }

    #[test]
    fn malformed_port_is_named() {
        let mut vars = base();
        vars.push(("PORT", Some("not-a-port")));
        let err = Config::from_map(&vars).unwrap_err();
        assert_eq!(err.item, "PORT");
    }

    #[test]
    fn max_ttl_below_ttl_is_rejected_and_named() {
        let mut vars = base();
        vars.push(("SESSION_TTL_SECS", Some("100")));
        vars.push(("SESSION_MAX_TTL_SECS", Some("50")));
        let err = Config::from_map(&vars).unwrap_err();
        assert_eq!(err.item, "SESSION_MAX_TTL_SECS");
    }
}
