//! US1 graceful shutdown (spec US1.4: signal → stop accepting, drain, exit ≤ 10s).
//! Driven via the test stop-trigger (equivalent to a process signal).

use std::sync::Arc;
use std::time::Duration;

use shared::session::MemorySessionStore;
use shared::storage::MemoryStorage;

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

#[tokio::test]
async fn stop_signal_drains_and_exits_within_10s() {
    let port = free_port().await;
    let storage: Arc<dyn shared::storage::Storage> = Arc::new(MemoryStorage::new());
    let store: Arc<dyn shared::session::SessionStore> = Arc::new(MemorySessionStore::new());
    let stop = Arc::new(tokio::sync::Notify::new());
    let stop2 = stop.clone();
    let handle = tokio::spawn(async move {
        host::run_with_stop(cfg(port), storage, store, Some(stop2)).await
    });

    // Wait until it is serving (so the drain path is actually exercised).
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    stop.notify_one(); // == process signal
    let res = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("must exit within 10s (spec US1.4)")
        .expect("join");
    assert!(res.is_ok(), "clean exit");
}
