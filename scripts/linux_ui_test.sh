#!/bin/bash
# Linux UI automation test for cterm
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

    # Try to capture the focused window, fall back to full screen
    if command -v import &> /dev/null; then
        # ImageMagick's import - capture root window (full screen)
        import -window root "$output_path" 2>/dev/null || true
    elif command -v scrot &> /dev/null; then
        scrot "$output_path" 2>/dev/null || true
    elif command -v gnome-screenshot &> /dev/null; then
        gnome-screenshot -f "$output_path" 2>/dev/null || true
    else
        log "WARNING: No screenshot tool available"
        return 1
    fi

    if [ -f "$output_path" ]; then
        log "Screenshot saved: $output_path"
        return 0
    else
        log "WARNING: Failed to save screenshot: $output_path"
        return 1
    fi
}

send_keys() {
    local keys="$1"
    if command -v xdotool &> /dev/null; then
        xdotool type --delay 50 "$keys"
    else
        log "ERROR: xdotool not available"
        return 1
    fi
}

send_key() {
    local key="$1"
    if command -v xdotool &> /dev/null; then
        xdotool key "$key"
    else
        log "ERROR: xdotool not available"
        return 1
    fi
}

log "=== cterm UI Automation Test (Linux) ==="
log "Executable: $CTERM_PATH"
log "Output: $OUTPUT_DIR"

# Check if executable exists
if [ ! -f "$CTERM_PATH" ]; then
    log "ERROR: cterm not found at $CTERM_PATH"
    exit 1
fi

# Check for required tools
for tool in xdotool; do
    if ! command -v $tool &> /dev/null; then
        log "ERROR: Required tool '$tool' not found"
        exit 1
    fi
done

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
WINDOW_ID=""
ATTEMPTS=0
MAX_ATTEMPTS=30

while [ -z "$WINDOW_ID" ] && [ $ATTEMPTS -lt $MAX_ATTEMPTS ]; do
    sleep 0.5
    # Try to find the cterm window
    WINDOW_ID=$(xdotool search --pid $CTERM_PID --onlyvisible 2>/dev/null | head -1) || true
    ATTEMPTS=$((ATTEMPTS + 1))
    if [ $((ATTEMPTS % 5)) -eq 0 ]; then
        log "  Attempt $ATTEMPTS/$MAX_ATTEMPTS..."
    fi
done

if [ -z "$WINDOW_ID" ]; then
    log "ERROR: Window not found after $MAX_ATTEMPTS attempts"
    take_screenshot "error_no_window"

    # Check if process is still running
    if kill -0 $CTERM_PID 2>/dev/null; then
        log "Process is still running, killing..."
        kill $CTERM_PID 2>/dev/null || true
    fi

    # Show cterm log if exists
    if [ -f "$CTERM_LOG_FILE" ]; then
        log "cterm log contents:"
        cat "$CTERM_LOG_FILE" | while read line; do log "  $line"; done
    fi

    exit 1
fi

log "Window found: $WINDOW_ID"

# Focus the window
xdotool windowactivate --sync "$WINDOW_ID" 2>/dev/null || true
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

# Test Ctrl+T for new tab
log "Testing Ctrl+T (new tab)..."
send_key "ctrl+t"
sleep 1

# Take screenshot showing tabs
take_screenshot "05_new_tab"

# Close the window
log "Closing window..."
send_key "ctrl+shift+q" || xdotool windowclose "$WINDOW_ID" 2>/dev/null || true

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
