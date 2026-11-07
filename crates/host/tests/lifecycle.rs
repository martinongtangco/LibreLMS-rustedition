//! US1 (P1) — lifecycle gates (spec US1, all proven by test).
//! Deterministic doubles per Principle XIII/XVII.

use std::sync::Arc;
use std::time::{Duration, Instant};

use shared::session::MemorySessionStore;
use shared::storage::MemoryStorage;

/// Grab a free port (bind 0, read, drop — small accepted race in tests).
async fn free_port() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

fn cfg(port: u16) -> shared::Config {
    shared::Config::from_map(&[
        ("DATABASE_URL", Some("mssql://ignored (test double)")),
        ("VALKEY_URL", Some("redis://ignored (test double)")),
        ("PORT", Some(port.to_string().as_str())),
    ])
    .expect("valid")
}

/// Minimal HTTP/1.1 GET via tokio (no extra http client — Principle XVIII).
async fn get(url: &str, path: &str) -> Option<(u16, String)> {
    let addr = url.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(addr).await.ok()?;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let req = format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 65536 {
            break;
        }
    }
    let raw = String::from_utf8_lossy(&buf).to_string();
    let status = raw
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())?;
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Some((status, body))
}

async fn get_healthz(port: u16) -> Option<(u16, serde_json::Value)> {
    let (code, body) = get(&format!("127.0.0.1:{port}"), "/healthz").await?;
    let json = serde_json::from_str::<serde_json::Value>(&body).ok()?;
    Some((code, json))
}

async fn wait_health_ok(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some((200, j)) = get_healthz(port).await {
            if j["status"] == "ok" {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn startup_with_valid_config_reports_ok_health() {
    let port = free_port().await;
    let storage: Arc<dyn shared::storage::Storage> = Arc::new(MemoryStorage::new());
    let store: Arc<dyn shared::session::SessionStore> = Arc::new(MemorySessionStore::new());
    let stop = Arc::new(tokio::sync::Notify::new());
    let stop2 = stop.clone();
    let handle = tokio::spawn(async move {
        host::run_with_stop(
            cfg(port),
            storage,
            store,
            std::sync::Arc::new(catalog::repo::InMemoryCourseRepo::new()),
            Some(stop2),
        )
        .await
    });
    assert!(
        wait_health_ok(port, Duration::from_secs(10)).await,
        "healthz must be ok within 10s (spec SC-001)"
    );
    let (_, j) = get_healthz(port).await.expect("second read");
    assert_eq!(j["status"], "ok");
    assert_eq!(j["process"], "alive");
    assert_eq!(j["database"], "reachable");
    assert_eq!(j["session_store"], "reachable");
    stop.notify_one();
    let res = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("shutdown within 10s")
        .expect("join");
    assert!(res.is_ok(), "clean shutdown");
}

#[tokio::test]
async fn degraded_when_session_store_down() {
    let port = free_port().await;
    let storage: Arc<dyn shared::storage::Storage> = Arc::new(MemoryStorage::new());
    let mem_store = MemorySessionStore::new();
    let store: Arc<dyn shared::session::SessionStore> = Arc::new(mem_store.clone());
    let stop = Arc::new(tokio::sync::Notify::new());
    let stop2 = stop.clone();
    let handle = tokio::spawn(async move {
        host::run_with_stop(
            cfg(port),
            storage,
            store,
            std::sync::Arc::new(catalog::repo::InMemoryCourseRepo::new()),
            Some(stop2),
        )
        .await
    });
    assert!(wait_health_ok(port, Duration::from_secs(10)).await);
    mem_store.set_down(true); // runtime outage (spec US1.4)
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (code, j) = get_healthz(port).await.expect("still serving");
    assert_eq!(code, 503, "degraded → 503");
    assert_eq!(j["status"], "degraded");
    assert_eq!(j["session_store"], "unreachable");
    stop.notify_one();
    let _ = tokio::time::timeout(Duration::from_secs(10), handle).await;
}

#[tokio::test]
async fn port_in_use_fails_fast_naming_the_port() {
    let port = free_port().await;
    let holder = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .expect("holder binds");
    let storage: Arc<dyn shared::storage::Storage> = Arc::new(MemoryStorage::new());
    let store: Arc<dyn shared::session::SessionStore> = Arc::new(MemorySessionStore::new());
    let start = Instant::now();
    let res = host::run_with_stop(
        cfg(port),
        storage,
        store,
        std::sync::Arc::new(catalog::repo::InMemoryCourseRepo::new()),
        None,
    )
    .await;
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "fail-fast < 5s (SC-002)"
    );
    let msg = format!("{}", res.expect_err("must fail"));
    assert!(msg.contains(&port.to_string()), "names the port: {msg}");
    drop(holder);
}
