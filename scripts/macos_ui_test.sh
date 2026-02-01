#!/bin/bash
# macOS UI automation test for cterm
# Launches the app, types commands, takes screenshots, and closes

set -e

CTERM_PATH="${1:-target/debug/cterm}"
OUTPUT_DIR="${2:-test_output}"

# Create output directory
mkdir -p "$OUTPUT_DIR"

LOG_FILE="$OUTPUT_DIR/test.log"

log() {
    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    echo "$timestamp - $1" | tee -a "$LOG_FILE"
}

take_screenshot() {
    local name="$1"
    local output_path="$OUTPUT_DIR/${name}.png"

    # Use screencapture to capture the screen
    # -x: no sound, -C: capture cursor
    screencapture -x "$output_path" 2>/dev/null || true

    if [ -f "$output_path" ]; then
        log "Screenshot saved: $output_path"
        return 0
    else
        log "WARNING: Failed to save screenshot: $output_path"
        return 1
    fi
}

send_keys() {
    local text="$1"
    # Use AppleScript to send keystrokes to the frontmost application
    osascript -e "tell application \"System Events\" to keystroke \"$text\""
}

send_key() {
    local key="$1"
    local modifiers="${2:-}"

    case "$key" in
        "Return"|"return"|"enter")
            osascript -e 'tell application "System Events" to keystroke return'
            ;;
        "Tab"|"tab")
            if [ "$modifiers" = "ctrl" ]; then
                osascript -e 'tell application "System Events" to keystroke tab using control down'
            elif [ "$modifiers" = "ctrl+shift" ]; then
                osascript -e 'tell application "System Events" to keystroke tab using {control down, shift down}'
            else
                osascript -e 'tell application "System Events" to keystroke tab'
            fi
            ;;
        "t")
            if [ "$modifiers" = "cmd" ]; then
                osascript -e 'tell application "System Events" to keystroke "t" using command down'
            else
                osascript -e "tell application \"System Events\" to keystroke \"$key\""
            fi
            ;;
        "w")
            if [ "$modifiers" = "cmd" ]; then
                osascript -e 'tell application "System Events" to keystroke "w" using command down'
            else
                osascript -e "tell application \"System Events\" to keystroke \"$key\""
            fi
            ;;
        "q")
            if [ "$modifiers" = "cmd" ]; then
                osascript -e 'tell application "System Events" to keystroke "q" using command down'
            else
                osascript -e "tell application \"System Events\" to keystroke \"$key\""
            fi
            ;;
        *)
            osascript -e "tell application \"System Events\" to keystroke \"$key\""
            ;;
    esac
}

log "=== cterm UI Automation Test (macOS) ==="
log "Executable: $CTERM_PATH"
log "Output: $OUTPUT_DIR"

# Check if executable exists
if [ ! -f "$CTERM_PATH" ]; then
    # Check if it's an app bundle
    if [ -d "$CTERM_PATH" ] && [ -f "$CTERM_PATH/Contents/MacOS/cterm" ]; then
        CTERM_PATH="$CTERM_PATH/Contents/MacOS/cterm"
        log "Using app bundle executable: $CTERM_PATH"
    else
        log "ERROR: cterm not found at $CTERM_PATH"
        exit 1
    fi
fi

# Set up environment
export RUST_LOG=debug
export CTERM_LOG_FILE="$OUTPUT_DIR/cterm.log"

# Start cterm in background (disable watchdog for CI testing)
log "Starting cterm..."
"$CTERM_PATH" --no-watchdog &
CTERM_PID=$!
log "Process started with PID: $CTERM_PID"

# Wait for window to appear
log "Waiting for window..."
ATTEMPTS=0
MAX_ATTEMPTS=30
WINDOW_FOUND=false

while [ $ATTEMPTS -lt $MAX_ATTEMPTS ]; do
    sleep 0.5

    # Try multiple methods to detect the window
    # Method 1: By PID (may not work without accessibility permissions)
    WINDOW_COUNT=$(osascript -e 'tell application "System Events" to count windows of (processes whose unix id is '$CTERM_PID')' 2>/dev/null) || WINDOW_COUNT=0

    # Method 2: By process name "cterm"
    if [ "$WINDOW_COUNT" -eq 0 ]; then
        WINDOW_COUNT=$(osascript -e 'tell application "System Events" to count windows of (processes whose name is "cterm")' 2>/dev/null) || WINDOW_COUNT=0
    fi

    if [ "$WINDOW_COUNT" -gt 0 ]; then
        WINDOW_FOUND=true
        break
    fi

    ATTEMPTS=$((ATTEMPTS + 1))
    if [ $((ATTEMPTS % 5)) -eq 0 ]; then
        log "  Attempt $ATTEMPTS/$MAX_ATTEMPTS..."
    fi
done

# Even if window detection failed, check if process is running and try to proceed
if [ "$WINDOW_FOUND" != "true" ]; then
    if kill -0 $CTERM_PID 2>/dev/null; then
        log "Window detection failed but process is running - trying to proceed anyway"
        # Wait a bit more for window to be ready
        sleep 2
    else
        log "ERROR: Window not found and process is not running"
        take_screenshot "error_no_window"

        # Show cterm log if exists
        if [ -f "$CTERM_LOG_FILE" ]; then
            log "cterm log contents:"
            cat "$CTERM_LOG_FILE" | while read line; do log "  $line"; done
        fi

        exit 1
    fi
else
    log "Window found"
fi

# Activate cterm window
log "Activating cterm window..."
# Try by PID first, then by name
osascript -e "tell application \"System Events\"
    try
        set frontmost of (first process whose unix id is $CTERM_PID) to true
    on error
        set frontmost of (first process whose name is \"cterm\") to true
    end try
end tell" 2>/dev/null || true
sleep 1

# Take initial screenshot
take_screenshot "01_startup"

# Type command
log "Typing 'echo hello world'..."
send_keys "echo hello world"
sleep 0.5

# Take screenshot after typing
take_screenshot "02_after_typing"

# Press Enter
log "Pressing Enter..."
send_key "Return"
sleep 1

# Take screenshot after command execution
take_screenshot "03_after_enter"

# Type another command
log "Typing 'ls -la'..."
send_keys "ls -la"
sleep 0.5
send_key "Return"
sleep 1

# Take screenshot after ls
take_screenshot "04_after_ls"

# Test Cmd+T for new tab
log "Testing Cmd+T (new tab)..."
send_key "t" "cmd"
sleep 1

# Take screenshot showing tabs
take_screenshot "05_new_tab"

# Close the window
log "Closing window..."
send_key "q" "cmd"

# Wait for process to exit
sleep 2
if kill -0 $CTERM_PID 2>/dev/null; then
    log "Process did not exit gracefully, killing..."
    kill $CTERM_PID 2>/dev/null || true
    sleep 1
    kill -9 $CTERM_PID 2>/dev/null || true
fi

# Copy cterm log
if [ -f "$CTERM_LOG_FILE" ]; then
    log ""
    log "=== cterm application log ==="
    cat "$CTERM_LOG_FILE" | while read line; do log "$line"; done
fi

log ""
log "=== Test completed ==="
log "Screenshots saved to: $OUTPUT_DIR"
ls -la "$OUTPUT_DIR"/*.png 2>/dev/null | while read line; do log "  $line"; done

exit 0
