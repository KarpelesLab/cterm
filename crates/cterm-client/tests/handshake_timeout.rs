//! Regression test: a wedged daemon (socket exists and accepts connections, but never
//! answers the handshake) must make the client fail with `DaemonUnresponsive` within the
//! timeout instead of hanging forever. This is the failure that prevented `cterm` from
//! starting when `ctermd` deadlocked.

#![cfg(unix)]

use std::time::{Duration, Instant};

use cterm_client::{ClientError, DaemonConnection};

#[tokio::test]
async fn connect_to_wedged_daemon_times_out() {
    // Bind a Unix socket but never call `accept()` — exactly like a daemon whose
    // accept loop has stalled. The kernel completes `connect()` into the backlog, so
    // the client's transport connect succeeds at the socket level but the HTTP/2
    // handshake never completes.
    let dir = std::env::temp_dir().join(format!("cterm-test-wedged-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let sock = dir.join("ctermd.sock");
    let _listener = tokio::net::UnixListener::bind(&sock).expect("bind unix socket");

    let start = Instant::now();
    // auto_start = false: we must NOT spawn a real daemon, just observe the timeout.
    let result = DaemonConnection::connect_unix(&sock, false).await;
    let elapsed = start.elapsed();

    std::fs::remove_dir_all(&dir).ok();

    match result {
        Err(ClientError::DaemonUnresponsive(_)) => {}
        Err(e) => panic!("expected DaemonUnresponsive, got error {e:?}"),
        Ok(_) => panic!("expected DaemonUnresponsive, but connect unexpectedly succeeded"),
    }

    // Must fail promptly (connect timeout is 3s, handshake 5s) — never hang.
    assert!(
        elapsed < Duration::from_secs(15),
        "connect took too long: {elapsed:?}"
    );
}
