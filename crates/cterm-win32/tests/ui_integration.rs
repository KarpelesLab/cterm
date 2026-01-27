//! Windows UI integration tests for cterm
//!
//! These tests launch the actual cterm application, send input, take screenshots,
//! and verify behavior through logs and visual output.

#![cfg(windows)]

mod harness;

use harness::TestHarness;
use std::time::Duration;

/// Test that the application starts successfully and creates a window
#[test]
fn test_app_starts_and_creates_window() {
    let harness = TestHarness::launch().expect("Failed to launch cterm");

    // Wait for window to be created
    std::thread::sleep(Duration::from_secs(2));

    // Find the window
    let hwnd = harness.find_window().expect("Window not found");
    assert!(!hwnd.is_null(), "Window handle should not be null");

    // Take a screenshot for reference
    if let Err(e) = harness.take_screenshot("startup") {
        eprintln!("Screenshot failed (non-fatal): {}", e);
    }

    // Get and print logs
    let logs = harness.get_logs();
    println!("=== Application Logs ===");
    for log in &logs {
        println!("{}", log);
    }
    println!("========================");

    // Verify no errors in startup
    let has_startup_error = logs.iter().any(|l| {
        l.contains("ERROR") && (l.contains("Application error") || l.contains("Failed to"))
    });
    assert!(!has_startup_error, "Startup should not have errors");

    // Verify window was created
    let has_window_log = logs
        .iter()
        .any(|l| l.contains("Starting cterm") || l.contains("Windows native UI"));
    assert!(has_window_log, "Should have startup log");
}

/// Test that keyboard input is processed correctly
#[test]
fn test_keyboard_input() {
    let harness = TestHarness::launch().expect("Failed to launch cterm");
    std::thread::sleep(Duration::from_secs(2));

    let hwnd = harness.find_window().expect("Window not found");

    // Focus the window
    harness.focus_window(hwnd);
    std::thread::sleep(Duration::from_millis(500));

    // Type some text
    harness.send_text("echo hello\r");
    std::thread::sleep(Duration::from_secs(1));

    // Take screenshot after typing
    if let Err(e) = harness.take_screenshot("after_input") {
        eprintln!("Screenshot failed (non-fatal): {}", e);
    }

    // Get logs
    let logs = harness.get_logs();
    println!("=== Input Test Logs ===");
    for log in &logs {
        println!("{}", log);
    }
}

/// Test new tab creation with Ctrl+T
#[test]
fn test_new_tab_shortcut() {
    let harness = TestHarness::launch().expect("Failed to launch cterm");
    std::thread::sleep(Duration::from_secs(2));

    let hwnd = harness.find_window().expect("Window not found");
    harness.focus_window(hwnd);
    std::thread::sleep(Duration::from_millis(500));

    // Send Ctrl+T to create new tab
    harness.send_key_combo(&[harness::VK_CONTROL, 'T' as u16]);
    std::thread::sleep(Duration::from_secs(1));

    // Take screenshot showing tabs
    if let Err(e) = harness.take_screenshot("new_tab") {
        eprintln!("Screenshot failed (non-fatal): {}", e);
    }

    let logs = harness.get_logs();
    println!("=== New Tab Test Logs ===");
    for log in &logs {
        println!("{}", log);
    }
}

/// Test window resize
#[test]
fn test_window_resize() {
    let harness = TestHarness::launch().expect("Failed to launch cterm");
    std::thread::sleep(Duration::from_secs(2));

    let hwnd = harness.find_window().expect("Window not found");

    // Resize window
    harness.resize_window(hwnd, 1024, 768);
    std::thread::sleep(Duration::from_millis(500));

    // Take screenshot at new size
    if let Err(e) = harness.take_screenshot("resized") {
        eprintln!("Screenshot failed (non-fatal): {}", e);
    }

    let logs = harness.get_logs();
    println!("=== Resize Test Logs ===");
    for log in &logs {
        println!("{}", log);
    }
}
