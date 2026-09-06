use std::path::Path;
use std::time::{Duration, Instant};

use cterm_client::DaemonConnection;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let sock = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/run/user/1000/cterm/ctermd.sock".to_string());
    let path = Path::new(&sock);

    eprintln!("probe: connecting + handshake to {sock} (5s timeout)...");
    let t = Instant::now();
    let conn = match tokio::time::timeout(
        Duration::from_secs(5),
        DaemonConnection::connect_unix(path, false),
    )
    .await
    {
        Err(_) => {
            eprintln!(
                "probe: HANDSHAKE TIMED OUT after {:?} -- daemon accepted TCP but never answered Handshake RPC",
                t.elapsed()
            );
            std::process::exit(2);
        }
        Ok(Err(e)) => {
            eprintln!(
                "probe: handshake returned error after {:?}: {e}",
                t.elapsed()
            );
            std::process::exit(3);
        }
        Ok(Ok(c)) => {
            eprintln!("probe: handshake OK in {:?}", t.elapsed());
            c
        }
    };

    let info = conn.info();
    eprintln!(
        "probe: daemon_version={} hostname={} is_local={}",
        info.daemon_version, info.hostname, info.is_local
    );

    eprintln!("probe: list_sessions (5s timeout)...");
    let t = Instant::now();
    match tokio::time::timeout(Duration::from_secs(5), conn.list_sessions()).await {
        Err(_) => {
            eprintln!(
                "probe: LIST_SESSIONS TIMED OUT after {:?} -- handshake works but ListSessions is blocked",
                t.elapsed()
            );
            std::process::exit(4);
        }
        Ok(Err(e)) => eprintln!("probe: list_sessions error after {:?}: {e}", t.elapsed()),
        Ok(Ok(list)) => {
            eprintln!(
                "probe: list_sessions OK in {:?}, {} session(s):",
                t.elapsed(),
                list.len()
            );
            for s in &list {
                eprintln!("  - {s:?}");
            }
        }
    }
}
