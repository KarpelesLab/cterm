# Configuration

cterm uses TOML configuration files stored in platform-specific locations:

- **Linux**: `~/.config/cterm/`
- **macOS**: `~/Library/Application Support/com.cterm.terminal/`
- **Windows**: `%APPDATA%\cterm\`

## Main Configuration (`config.toml`)

The main configuration file controls general behavior, appearance, and keyboard shortcuts.

### General Settings

```toml
[general]
# Shell to use (defaults to $SHELL or /bin/sh)
default_shell = "/bin/bash"

# Arguments to pass to the shell
shell_args = []

# Initial working directory (optional)
working_directory = "/home/user/projects"

# Number of lines to keep in scrollback buffer
scrollback_lines = 10000

# Ask for confirmation when closing with running processes
confirm_close_with_running = true

# Environment variables to set
[general.env]
EDITOR = "vim"
TERM = "xterm-256color"
```

### Appearance Settings

```toml
[appearance]
# Theme name (built-in or custom)
theme = "Tokyo Night"

[appearance.font]
# Font family (monospace font recommended)
family = "JetBrains Mono"

# Font size in points
size = 12

[appearance.cursor]
# Cursor style: "block", "underline", or "bar"
style = "block"

# Whether the cursor should blink
blink = true
```

### Tab Settings

```toml
[tabs]
# When to show the tab bar: "always", "multiple", or "never"
show_tab_bar = "always"

# Tab bar position: "top" or "bottom"
tab_bar_position = "top"
```

### Keyboard Shortcuts

```toml
[shortcuts]
# Tab management
new_tab = "Ctrl+Shift+T"
close_tab = "Ctrl+Shift+W"
next_tab = "Ctrl+Tab"
prev_tab = "Ctrl+Shift+Tab"

# Clipboard
copy = "Ctrl+Shift+C"
paste = "Ctrl+Shift+V"

# Zoom
zoom_in = "Ctrl+plus"
zoom_out = "Ctrl+minus"
zoom_reset = "Ctrl+0"

# Navigation
scroll_up = "Shift+Page_Up"
scroll_down = "Shift+Page_Down"

# Find
find = "Ctrl+Shift+F"
```

## Sticky Tabs (`sticky_tabs.toml`)

Sticky tabs are persistent tab configurations that appear in the File menu and can be quickly opened. They're ideal for frequently-used commands or AI coding assistants.

```toml
[[tabs]]
# Display name in the menu
name = "Claude"

# Command to run
command = "claude"

# Optional: Arguments to pass
args = []

# Optional: Tab color (hex color code)
color = "#7c3aed"

# Optional: Keep tab open after command exits
keep_open = true

# Optional: Working directory
cwd = "/home/user/projects"

[[tabs]]
name = "Claude (Continue)"
command = "claude"
args = ["-c"]
color = "#7c3aed"
keep_open = true

[[tabs]]
name = "Python REPL"
command = "python3"
color = "#3572A5"
keep_open = false
```

## Custom Themes (`themes/`)

Custom themes are TOML files placed in the `themes/` subdirectory of the configuration folder.

Example theme file (`themes/my-theme.toml`):

```toml
name = "My Custom Theme"

[colors]
# Standard colors (0-7)
black = "#000000"
red = "#ff0000"
green = "#00ff00"
yellow = "#ffff00"
blue = "#0000ff"
magenta = "#ff00ff"
cyan = "#00ffff"
white = "#ffffff"

# Bright colors (8-15)
bright_black = "#808080"
bright_red = "#ff8080"
bright_green = "#80ff80"
bright_yellow = "#ffff80"
bright_blue = "#8080ff"
bright_magenta = "#ff80ff"
bright_cyan = "#80ffff"
bright_white = "#ffffff"

# Special colors
background = "#1a1b26"
foreground = "#c0caf5"
cursor = "#c0caf5"
selection_background = "#33467c"
selection_foreground = "#c0caf5"
```

## Built-in Themes

cterm includes several built-in themes:

- **Default Dark** - A simple dark theme
- **Default Light** - A simple light theme
- **Tokyo Night** - Popular dark theme with purple accents
- **Dracula** - Classic dark theme with vibrant colors
- **Nord** - Arctic, north-bluish color palette

Select themes through the menu: **View → Theme → [Theme Name]**

## Keyboard Shortcut Format

Shortcuts are specified as a combination of modifiers and a key:

**Modifiers:**
- `Ctrl` - Control key
- `Shift` - Shift key
- `Alt` - Alt/Option key
- `Super` - Super/Windows/Command key

**Keys:**
- Letters: `A` through `Z`
- Numbers: `0` through `9`
- Function keys: `F1` through `F12`
- Special keys: `Tab`, `Return`, `Escape`, `Space`, `BackSpace`, `Delete`, `Insert`, `Home`, `End`, `Page_Up`, `Page_Down`
- Symbols: `plus`, `minus`, `equal`, `bracketleft`, `bracketright`, etc.

**Examples:**
```toml
new_tab = "Ctrl+Shift+T"
zoom_in = "Ctrl+plus"
find = "Ctrl+Shift+F"
```
