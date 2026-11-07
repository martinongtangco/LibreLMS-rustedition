//! Shared test helpers for the host integration tests.

pub struct AppConfig;

impl AppConfig {
    /// A valid config bound to a free test port (fail-fast verified elsewhere).
    pub fn valid(port: u16) -> shared::config::Config {
        shared::config::Config {
            database_url: "mssql://test@test/LibreLms".to_string(),
            valkey_url: "redis://test".to_string(),
            port,
            log_level: "info".to_string(),
            session_ttl_secs: 3_600,
            session_max_ttl_secs: 72_000,
        }
    }
}

pub async fn wait_health_ok(port: u16) -> bool {
    for _ in 0..200 {
        if let Some((code, _)) = get_with(port, "/healthz", &[]).await {
            if code == 200 {
                return true;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    false
}

pub async fn get_with(port: u16, path: &str, headers: &[(&str, &str)]) -> Option<(u16, String)> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).await.ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.ok()?;
    let text = String::from_utf8_lossy(&buf).to_string();
    let head = text.lines().next()?.to_string();
    let code: u16 = head.split_whitespace().nth(1)?.parse().ok()?;
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    Some((code, body))
}
