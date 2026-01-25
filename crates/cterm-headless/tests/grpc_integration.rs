//! gRPC integration tests for ctermd
//!
//! These tests spawn a ctermd server and test the gRPC API.

use std::process::{Child, Command};
use std::time::Duration;

use cterm_headless::proto::terminal_service_client::TerminalServiceClient;
use cterm_headless::proto::*;
use tonic::transport::Channel;

/// Helper to find an available port
fn find_available_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Helper to spawn ctermd server
struct CtermdServer {
    child: Child,
    port: u16,
}

impl CtermdServer {
    fn spawn() -> Self {
        let port = find_available_port();

        // Find the ctermd binary - workspace root is 2 levels up from this crate
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        let debug_path = if cfg!(windows) {
            workspace_root.join("target/debug/ctermd.exe")
        } else {
            workspace_root.join("target/debug/ctermd")
        };

        let release_path = if cfg!(windows) {
            workspace_root.join("target/release/ctermd.exe")
        } else {
            workspace_root.join("target/release/ctermd")
        };

        let ctermd_path = if debug_path.exists() {
            debug_path
        } else if release_path.exists() {
            release_path
        } else {
            panic!(
                "ctermd binary not found. Tried:\n  {}\n  {}\nPlease build with: cargo build -p cterm-headless",
                debug_path.display(),
                release_path.display()
            );
        };

        let child = Command::new(&ctermd_path)
            .args(["--tcp", "--port", &port.to_string(), "--bind", "127.0.0.1"])
            .spawn()
            .unwrap_or_else(|e| {
                panic!("Failed to spawn ctermd at {}: {}", ctermd_path.display(), e)
            });

        // Give the server time to start
        std::thread::sleep(Duration::from_millis(500));

        Self { child, port }
    }

    fn address(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for CtermdServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Connect to the gRPC server
async fn connect(addr: &str) -> TerminalServiceClient<Channel> {
    // Retry connection a few times
    for i in 0..10 {
        match TerminalServiceClient::connect(addr.to_string()).await {
            Ok(client) => return client,
            Err(_) if i < 9 => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) => panic!("Failed to connect to ctermd: {}", e),
        }
    }
    unreachable!()
}

#[tokio::test]
async fn test_create_and_list_sessions() {
    let server = CtermdServer::spawn();
    let mut client = connect(&server.address()).await;

    // Initially no sessions
    let response = client
        .list_sessions(ListSessionsRequest {})
        .await
        .expect("list_sessions failed");
    assert!(
        response.get_ref().sessions.is_empty(),
        "Expected no sessions initially"
    );

    // Create a session
    let create_response = client
        .create_session(CreateSessionRequest {
            cols: 80,
            rows: 24,
            shell: None,
            args: vec![],
            cwd: None,
            env: Default::default(),
            term: None,
        })
        .await
        .expect("create_session failed");

    let session_id = create_response.get_ref().session_id.clone();
    assert!(!session_id.is_empty(), "Session ID should not be empty");
    assert_eq!(create_response.get_ref().cols, 80);
    assert_eq!(create_response.get_ref().rows, 24);

    // List sessions should now show one
    let response = client
        .list_sessions(ListSessionsRequest {})
        .await
        .expect("list_sessions failed");
    assert_eq!(response.get_ref().sessions.len(), 1);
    assert_eq!(response.get_ref().sessions[0].session_id, session_id);

    // Destroy the session
    let destroy_response = client
        .destroy_session(DestroySessionRequest {
            session_id: session_id.clone(),
            signal: None,
        })
        .await
        .expect("destroy_session failed");
    assert!(destroy_response.get_ref().success);

    // List should be empty again
    let response = client
        .list_sessions(ListSessionsRequest {})
        .await
        .expect("list_sessions failed");
    assert!(response.get_ref().sessions.is_empty());
}

#[tokio::test]
async fn test_get_session() {
    let server = CtermdServer::spawn();
    let mut client = connect(&server.address()).await;

    // Create a session
    let create_response = client
        .create_session(CreateSessionRequest {
            cols: 100,
            rows: 30,
            shell: None,
            args: vec![],
            cwd: None,
            env: Default::default(),
            term: None,
        })
        .await
        .expect("create_session failed");

    let session_id = create_response.get_ref().session_id.clone();

    // Get session info
    let response = client
        .get_session(GetSessionRequest {
            session_id: session_id.clone(),
        })
        .await
        .expect("get_session failed");

    let session = response.get_ref().session.as_ref().unwrap();
    assert_eq!(session.session_id, session_id);
    assert_eq!(session.cols, 100);
    assert_eq!(session.rows, 30);
    assert!(session.running);

    // Cleanup
    let _ = client
        .destroy_session(DestroySessionRequest {
            session_id,
            signal: None,
        })
        .await;
}

// This test uses `cat` to echo input back, which requires Unix-like behavior.
// On Windows, cmd.exe's `more` doesn't work the same way.
#[cfg(unix)]
#[tokio::test]
async fn test_write_input_and_get_screen() {
    let server = CtermdServer::spawn();
    let mut client = connect(&server.address()).await;

    // Create a session with cat command to echo input
    let create_response = client
        .create_session(CreateSessionRequest {
            cols: 80,
            rows: 24,
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "cat".to_string()],
            cwd: None,
            env: Default::default(),
            term: Some("xterm".to_string()),
        })
        .await
        .expect("create_session failed");

    let session_id = create_response.get_ref().session_id.clone();

    // Give the shell time to start
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Write some input
    let write_response = client
        .write_input(WriteInputRequest {
            session_id: session_id.clone(),
            data: b"hello\n".to_vec(),
        })
        .await
        .expect("write_input failed");

    assert!(write_response.get_ref().bytes_written > 0);

    // Give it time to process
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Get screen text
    let screen_response = client
        .get_screen_text(GetScreenTextRequest {
            session_id: session_id.clone(),
            include_scrollback: false,
            start_row: None,
            end_row: None,
        })
        .await
        .expect("get_screen_text failed");

    // The output should contain "hello" somewhere
    let screen_text = screen_response.get_ref().lines.join("\n");
    assert!(
        screen_text.contains("hello"),
        "Screen should contain 'hello', got: {:?}",
        screen_text
    );

    // Cleanup
    let _ = client
        .destroy_session(DestroySessionRequest {
            session_id,
            signal: None,
        })
        .await;
}

#[tokio::test]
async fn test_resize() {
    let server = CtermdServer::spawn();
    let mut client = connect(&server.address()).await;

    // Create a session
    let create_response = client
        .create_session(CreateSessionRequest {
            cols: 80,
            rows: 24,
            shell: None,
            args: vec![],
            cwd: None,
            env: Default::default(),
            term: None,
        })
        .await
        .expect("create_session failed");

    let session_id = create_response.get_ref().session_id.clone();

    // Resize
    let resize_response = client
        .resize(ResizeRequest {
            session_id: session_id.clone(),
            cols: 120,
            rows: 40,
        })
        .await
        .expect("resize failed");

    assert!(resize_response.get_ref().success);

    // Verify new size
    let response = client
        .get_session(GetSessionRequest {
            session_id: session_id.clone(),
        })
        .await
        .expect("get_session failed");

    let session = response.get_ref().session.as_ref().unwrap();
    assert_eq!(session.cols, 120);
    assert_eq!(session.rows, 40);

    // Cleanup
    let _ = client
        .destroy_session(DestroySessionRequest {
            session_id,
            signal: None,
        })
        .await;
}

#[tokio::test]
async fn test_get_cursor() {
    let server = CtermdServer::spawn();
    let mut client = connect(&server.address()).await;

    // Create a session
    let create_response = client
        .create_session(CreateSessionRequest {
            cols: 80,
            rows: 24,
            shell: None,
            args: vec![],
            cwd: None,
            env: Default::default(),
            term: None,
        })
        .await
        .expect("create_session failed");

    let session_id = create_response.get_ref().session_id.clone();

    // Give shell time to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Get cursor position
    let cursor_response = client
        .get_cursor(GetCursorRequest {
            session_id: session_id.clone(),
        })
        .await
        .expect("get_cursor failed");

    let cursor = cursor_response.get_ref().cursor.as_ref().unwrap();
    // Cursor should be visible
    assert!(cursor.visible);

    // Cleanup
    let _ = client
        .destroy_session(DestroySessionRequest {
            session_id,
            signal: None,
        })
        .await;
}

#[tokio::test]
async fn test_get_screen_full() {
    let server = CtermdServer::spawn();
    let mut client = connect(&server.address()).await;

    // Create a session
    let create_response = client
        .create_session(CreateSessionRequest {
            cols: 80,
            rows: 24,
            shell: None,
            args: vec![],
            cwd: None,
            env: Default::default(),
            term: None,
        })
        .await
        .expect("create_session failed");

    let session_id = create_response.get_ref().session_id.clone();

    // Give shell time to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Get full screen state
    let screen_response = client
        .get_screen(GetScreenRequest {
            session_id: session_id.clone(),
            include_scrollback: false,
        })
        .await
        .expect("get_screen failed");

    let screen = screen_response.get_ref();
    assert_eq!(screen.cols, 80);
    assert_eq!(screen.rows, 24);
    assert!(screen.cursor.is_some());
    // Should have 24 visible rows
    assert_eq!(screen.visible_rows.len(), 24);

    // Cleanup
    let _ = client
        .destroy_session(DestroySessionRequest {
            session_id,
            signal: None,
        })
        .await;
}

#[tokio::test]
async fn test_multiple_sessions() {
    let server = CtermdServer::spawn();
    let mut client = connect(&server.address()).await;

    // Create multiple sessions
    let mut session_ids = Vec::new();
    for _ in 0..3 {
        let response = client
            .create_session(CreateSessionRequest {
                cols: 80,
                rows: 24,
                shell: None,
                args: vec![],
                cwd: None,
                env: Default::default(),
                term: None,
            })
            .await
            .expect("create_session failed");
        session_ids.push(response.get_ref().session_id.clone());
    }

    // List should show 3 sessions
    let response = client
        .list_sessions(ListSessionsRequest {})
        .await
        .expect("list_sessions failed");
    assert_eq!(response.get_ref().sessions.len(), 3);

    // Destroy all
    for session_id in session_ids {
        let _ = client
            .destroy_session(DestroySessionRequest {
                session_id,
                signal: None,
            })
            .await;
    }

    // Should be empty
    let response = client
        .list_sessions(ListSessionsRequest {})
        .await
        .expect("list_sessions failed");
    assert!(response.get_ref().sessions.is_empty());
}

#[tokio::test]
async fn test_invalid_session_id() {
    let server = CtermdServer::spawn();
    let mut client = connect(&server.address()).await;

    // Try to get a non-existent session
    let result = client
        .get_session(GetSessionRequest {
            session_id: "nonexistent-session-id".to_string(),
        })
        .await;

    assert!(result.is_err(), "Should fail for invalid session ID");
}
