# Windows UI automation test for cterm
# Launches the app, types commands, takes screenshots, and closes

param(
    [string]$CtermPath = "target\debug\cterm.exe",
    [string]$OutputDir = "test_output"
)

$ErrorActionPreference = "Stop"

# Create output directory
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

# Log file
$LogFile = Join-Path $OutputDir "test.log"

function Log {
    param([string]$Message)
    $timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    "$timestamp - $Message" | Tee-Object -FilePath $LogFile -Append
}

function Take-Screenshot {
    param(
        [string]$Name,
        [System.IntPtr]$Hwnd = [System.IntPtr]::Zero
    )

    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing

    $outputPath = Join-Path $OutputDir "$Name.png"

    if ($Hwnd -ne [System.IntPtr]::Zero) {
        # Get window rectangle
        $rect = New-Object RECT
        [User32]::GetWindowRect($Hwnd, [ref]$rect) | Out-Null

        $width = $rect.Right - $rect.Left
        $height = $rect.Bottom - $rect.Top

        if ($width -gt 0 -and $height -gt 0) {
            $bitmap = New-Object System.Drawing.Bitmap($width, $height)
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, [System.Drawing.Size]::new($width, $height))
            $bitmap.Save($outputPath, [System.Drawing.Imaging.ImageFormat]::Png)
            $graphics.Dispose()
            $bitmap.Dispose()
            Log "Screenshot saved: $outputPath"
            return $true
        }
    }

    # Fallback: full screen
    $screen = [System.Windows.Forms.Screen]::PrimaryScreen
    $bitmap = New-Object System.Drawing.Bitmap($screen.Bounds.Width, $screen.Bounds.Height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($screen.Bounds.Location, [System.Drawing.Point]::Empty, $screen.Bounds.Size)
    $bitmap.Save($outputPath, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
    Log "Full screen screenshot saved: $outputPath"
    return $true
}

# Add Win32 types
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Collections.Generic;

public struct RECT {
    public int Left;
    public int Top;
    public int Right;
    public int Bottom;
}

public class User32 {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern IntPtr FindWindow(string lpClassName, string lpWindowName);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, ref RECT lpRect);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll")]
    public static extern int GetWindowTextLength(IntPtr hWnd);

    public const uint WM_CLOSE = 0x0010;

    public static IntPtr FindWindowByProcessId(uint processId) {
        IntPtr result = IntPtr.Zero;
        EnumWindows(delegate(IntPtr hWnd, IntPtr lParam) {
            uint pid;
            GetWindowThreadProcessId(hWnd, out pid);
            if (pid == processId && IsWindowVisible(hWnd)) {
                int length = GetWindowTextLength(hWnd);
                if (length > 0) {
                    result = hWnd;
                    return false; // Stop enumeration
                }
            }
            return true; // Continue
        }, IntPtr.Zero);
        return result;
    }
}
"@

Log "=== cterm UI Automation Test ==="
Log "Executable: $CtermPath"

# Check if executable exists
if (-not (Test-Path $CtermPath)) {
    Log "ERROR: cterm.exe not found at $CtermPath"
    exit 1
}

# Set environment for logging
$env:RUST_LOG = "debug"
$env:CTERM_LOG_FILE = Join-Path $OutputDir "cterm.log"

Log "Starting cterm..."
$process = Start-Process -FilePath $CtermPath -PassThru

Log "Process started with PID: $($process.Id)"

# Wait for window to appear
Log "Waiting for window (PID: $($process.Id))..."
$hwnd = [System.IntPtr]::Zero
$attempts = 0
$maxAttempts = 30

while ($hwnd -eq [System.IntPtr]::Zero -and $attempts -lt $maxAttempts) {
    Start-Sleep -Milliseconds 500
    $hwnd = [User32]::FindWindowByProcessId($process.Id)
    $attempts++
    if ($attempts % 5 -eq 0) {
        Log "  Attempt $attempts/$maxAttempts..."
    }
}

if ($hwnd -eq [System.IntPtr]::Zero) {
    Log "ERROR: Window not found after $maxAttempts attempts"
    # Take screenshot of whatever is visible
    Take-Screenshot -Name "error_no_window"

    # Check if process is still running
    if (-not $process.HasExited) {
        Log "Process is still running, killing..."
        $process.Kill()
    }

    # Copy cterm log if exists
    if (Test-Path $env:CTERM_LOG_FILE) {
        Log "cterm log contents:"
        Get-Content $env:CTERM_LOG_FILE | ForEach-Object { Log "  $_" }
    }

    exit 1
}

Log "Window found: $hwnd"

# Bring window to foreground
[User32]::SetForegroundWindow($hwnd) | Out-Null
Start-Sleep -Seconds 1

# Take initial screenshot
Take-Screenshot -Name "01_startup" -Hwnd $hwnd

# Send keystrokes using SendKeys
Add-Type -AssemblyName System.Windows.Forms

Log "Typing 'echo hello world'..."
[System.Windows.Forms.SendKeys]::SendWait("echo hello world")
Start-Sleep -Milliseconds 500

# Take screenshot after typing
Take-Screenshot -Name "02_after_typing" -Hwnd $hwnd

# Press Enter
Log "Pressing Enter..."
[System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
Start-Sleep -Seconds 1

# Take screenshot after command execution
Take-Screenshot -Name "03_after_enter" -Hwnd $hwnd

# Type another command
Log "Typing 'dir'..."
[System.Windows.Forms.SendKeys]::SendWait("dir")
Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
Start-Sleep -Seconds 1

# Take screenshot after dir
Take-Screenshot -Name "04_after_dir" -Hwnd $hwnd

# Test Ctrl+T for new tab
Log "Testing Ctrl+T (new tab)..."
[System.Windows.Forms.SendKeys]::SendWait("^t")
Start-Sleep -Seconds 1

# Take screenshot showing tabs
Take-Screenshot -Name "05_new_tab" -Hwnd $hwnd

# Close the window
Log "Closing window..."
[User32]::PostMessage($hwnd, [User32]::WM_CLOSE, [System.IntPtr]::Zero, [System.IntPtr]::Zero) | Out-Null

# Wait for process to exit
$exited = $process.WaitForExit(5000)
if (-not $exited) {
    Log "Process did not exit gracefully, killing..."
    $process.Kill()
}

Log "Process exited with code: $($process.ExitCode)"

# Copy cterm log
if (Test-Path $env:CTERM_LOG_FILE) {
    Log ""
    Log "=== cterm application log ==="
    Get-Content $env:CTERM_LOG_FILE | ForEach-Object { Log $_ }
}

Log ""
Log "=== Test completed ==="
Log "Screenshots saved to: $OutputDir"
Get-ChildItem $OutputDir -Filter "*.png" | ForEach-Object { Log "  - $($_.Name)" }

exit 0
