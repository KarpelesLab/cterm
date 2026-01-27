//! Test harness for Windows UI integration tests
//!
//! Provides utilities for launching cterm, sending input, capturing screenshots,
//! and collecting logs.

#![cfg(windows)]
#![allow(dead_code)] // Some utilities are for future tests

use std::ffi::OsStr;
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::ptr;
use std::time::{Duration, Instant};

use winapi::shared::minwindef::{BOOL, DWORD, LPARAM, TRUE, UINT, WORD};
use winapi::shared::windef::{HDC, HGDIOBJ, HWND, RECT};
use winapi::um::wingdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
use winapi::um::winuser::{
    EnumWindows, FindWindowW, GetClientRect, GetDC, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible, MoveWindow, PrintWindow, ReleaseDC, SendInput, SetForegroundWindow, INPUT,
    INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, PW_CLIENTONLY,
};

/// Virtual key codes
pub const VK_CONTROL: u16 = 0x11;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_ALT: u16 = 0x12;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_TAB: u16 = 0x09;
pub const VK_ESCAPE: u16 = 0x1B;

/// Window class name for cterm
const WINDOW_CLASS: &str = "ctermWindow";

/// Test harness for launching and controlling cterm
pub struct TestHarness {
    /// Child process handle
    process: Child,
    /// Directory for storing screenshots
    screenshot_dir: PathBuf,
    /// Log file path
    log_file: PathBuf,
    /// Process ID
    pid: u32,
}

impl TestHarness {
    /// Launch cterm and create a test harness
    pub fn launch() -> io::Result<Self> {
        // Find the cterm executable
        let exe_path = find_cterm_exe()?;

        // Create test output directory
        let test_dir = std::env::temp_dir().join("cterm_test");
        std::fs::create_dir_all(&test_dir)?;

        // Create screenshot directory
        let screenshot_dir = test_dir.join("screenshots");
        std::fs::create_dir_all(&screenshot_dir)?;

        // Create log file path with unique name based on timestamp
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let log_file = test_dir.join(format!("cterm_{}.log", timestamp));

        // Launch the process with log file
        let process = Command::new(&exe_path)
            .env("RUST_LOG", "debug")
            .env("CTERM_LOG_FILE", &log_file)
            .spawn()?;

        let pid = process.id();

        Ok(Self {
            process,
            screenshot_dir,
            log_file,
            pid,
        })
    }

    /// Find the cterm window by class name
    pub fn find_window(&self) -> Option<HWND> {
        // First try FindWindowW with class name
        let class_name = to_wide_string(WINDOW_CLASS);
        let hwnd = unsafe { FindWindowW(class_name.as_ptr(), ptr::null()) };

        if !hwnd.is_null() {
            // Verify it belongs to our process
            let mut window_pid: DWORD = 0;
            unsafe { GetWindowThreadProcessId(hwnd, &mut window_pid) };
            if window_pid == self.pid {
                return Some(hwnd);
            }
        }

        // Fall back to enumeration
        self.find_window_by_enumeration()
    }

    /// Find window by enumerating all windows
    fn find_window_by_enumeration(&self) -> Option<HWND> {
        struct EnumData {
            target_pid: u32,
            result: HWND,
        }

        extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let data = unsafe { &mut *(lparam as *mut EnumData) };

            // Check if window is visible
            if unsafe { IsWindowVisible(hwnd) } == 0 {
                return TRUE;
            }

            // Get process ID
            let mut pid: DWORD = 0;
            unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };

            if pid == data.target_pid {
                // Get window title to verify it's our window
                let mut title = [0u16; 256];
                let len = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
                if len > 0 {
                    let title_str = String::from_utf16_lossy(&title[..len as usize]);
                    if title_str.contains("cterm") || title_str.is_empty() {
                        data.result = hwnd;
                        return 0; // Stop enumeration
                    }
                }
            }

            TRUE // Continue enumeration
        }

        let mut data = EnumData {
            target_pid: self.pid,
            result: ptr::null_mut(),
        };

        unsafe {
            EnumWindows(Some(enum_callback), &mut data as *mut _ as LPARAM);
        }

        if data.result.is_null() {
            None
        } else {
            Some(data.result)
        }
    }

    /// Wait for window to appear with timeout
    pub fn wait_for_window(&self, timeout: Duration) -> Option<HWND> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Some(hwnd) = self.find_window() {
                return Some(hwnd);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    /// Focus the window
    pub fn focus_window(&self, hwnd: HWND) {
        unsafe {
            SetForegroundWindow(hwnd);
        }
    }

    /// Resize the window
    pub fn resize_window(&self, hwnd: HWND, width: i32, height: i32) {
        unsafe {
            MoveWindow(hwnd, 100, 100, width, height, TRUE);
        }
    }

    /// Send a string of text as keyboard input
    pub fn send_text(&self, text: &str) {
        for c in text.chars() {
            if c == '\r' || c == '\n' {
                self.send_key(VK_RETURN);
            } else {
                // Convert char to virtual key code
                // For ASCII characters, the VK code is the uppercase ASCII value
                let vk = if c.is_ascii() {
                    c.to_ascii_uppercase() as u16
                } else {
                    continue; // Skip non-ASCII for now
                };
                self.send_key(vk);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Send a single key press
    pub fn send_key(&self, vk: u16) {
        let mut inputs = [create_key_input(vk, false), create_key_input(vk, true)];

        unsafe {
            SendInput(
                inputs.len() as UINT,
                inputs.as_mut_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }

    /// Send a key combination (e.g., Ctrl+T)
    pub fn send_key_combo(&self, keys: &[u16]) {
        // Press all keys
        let mut inputs: Vec<INPUT> = keys.iter().map(|&vk| create_key_input(vk, false)).collect();

        // Release all keys in reverse order
        for &vk in keys.iter().rev() {
            inputs.push(create_key_input(vk, true));
        }

        unsafe {
            SendInput(
                inputs.len() as UINT,
                inputs.as_mut_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }

    /// Take a screenshot of the window
    pub fn take_screenshot(&self, name: &str) -> io::Result<PathBuf> {
        let hwnd = self.find_window().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Window not found for screenshot")
        })?;

        let path = self.screenshot_dir.join(format!("{}.png", name));
        capture_window_to_png(hwnd, &path)?;

        println!("Screenshot saved to: {}", path.display());
        Ok(path)
    }

    /// Get application logs from the log file
    pub fn get_logs(&self) -> Vec<String> {
        if let Ok(content) = std::fs::read_to_string(&self.log_file) {
            content.lines().map(String::from).collect()
        } else {
            Vec::new()
        }
    }

    /// Get the log file path
    pub fn log_file(&self) -> &PathBuf {
        &self.log_file
    }

    /// Get the screenshot directory
    pub fn screenshot_dir(&self) -> &PathBuf {
        &self.screenshot_dir
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        // Kill the process when test is done
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Find the cterm executable
fn find_cterm_exe() -> io::Result<PathBuf> {
    // Try debug build first
    let debug_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join("debug")
        .join("cterm.exe");

    if debug_path.exists() {
        return Ok(debug_path);
    }

    // Try release build
    let release_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join("release")
        .join("cterm.exe");

    if release_path.exists() {
        return Ok(release_path);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "cterm.exe not found. Build the project first. Looked in:\n  {}\n  {}",
            debug_path.display(),
            release_path.display()
        ),
    ))
}

/// Convert a Rust string to a wide string for Windows APIs
fn to_wide_string(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Create a keyboard INPUT structure
fn create_key_input(vk: u16, key_up: bool) -> INPUT {
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.type_ = INPUT_KEYBOARD;

    let ki = unsafe { input.u.ki_mut() };
    *ki = KEYBDINPUT {
        wVk: vk,
        wScan: 0,
        dwFlags: if key_up { KEYEVENTF_KEYUP } else { 0 },
        time: 0,
        dwExtraInfo: 0,
    };

    input
}

/// Capture a window to a PNG file
fn capture_window_to_png(hwnd: HWND, path: &PathBuf) -> io::Result<()> {
    unsafe {
        // Get window dimensions
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        GetClientRect(hwnd, &mut rect);

        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        if width <= 0 || height <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid window dimensions",
            ));
        }

        // Get device context
        let hdc_window = GetDC(hwnd);
        if hdc_window.is_null() {
            return Err(io::Error::last_os_error());
        }

        // Create compatible DC and bitmap
        let hdc_mem = CreateCompatibleDC(hdc_window);
        let hbm = CreateCompatibleBitmap(hdc_window, width, height);
        let old_obj = SelectObject(hdc_mem, hbm as HGDIOBJ);

        // Copy window content to bitmap
        // Try PrintWindow first (works better with layered windows)
        let result = PrintWindow(hwnd, hdc_mem, PW_CLIENTONLY);
        if result == 0 {
            // Fall back to BitBlt
            BitBlt(hdc_mem, 0, 0, width, height, hdc_window, 0, 0, SRCCOPY);
        }

        // Get bitmap data
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as DWORD,
                biWidth: width,
                biHeight: -height, // Top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [std::mem::zeroed(); 1],
        };

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        GetDIBits(
            hdc_mem,
            hbm,
            0,
            height as UINT,
            pixels.as_mut_ptr() as *mut _,
            &mut bmi,
            DIB_RGB_COLORS,
        );

        // Cleanup GDI objects
        SelectObject(hdc_mem, old_obj);
        DeleteObject(hbm as HGDIOBJ);
        DeleteDC(hdc_mem);
        ReleaseDC(hwnd, hdc_window);

        // Convert BGRA to RGBA
        for chunk in pixels.chunks_mut(4) {
            chunk.swap(0, 2); // Swap B and R
        }

        // Save as PNG using the image crate
        let img =
            image::RgbaImage::from_raw(width as u32, height as u32, pixels).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Failed to create image buffer")
            })?;

        img.save(path)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        Ok(())
    }
}
