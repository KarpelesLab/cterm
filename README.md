# cterm

A high-performance, customizable terminal emulator built in pure Rust with GTK4, designed with modularity to support alternative UI backends. Features optimized for running AI coding assistants like Claude Code.

## Features

- **High Performance**: Custom terminal emulator with efficient screen buffer management
- **True Color Support**: Full 24-bit color support with 256-color palette
- **Unicode Support**: Proper handling of wide characters and Unicode
- **Tabs**: Multiple tabs with keyboard shortcuts, custom colors, and naming
- **Sticky Tabs**: Persistent tab configurations that survive restarts (great for Claude sessions)
- **Hyperlinks**: Clickable URLs with OSC 8 support
- **Customizable Themes**: Built-in themes (Tokyo Night, Dracula, Nord) plus custom themes via TOML
- **Keyboard Shortcuts**: Configurable shortcuts for all actions
- **Cross-Platform**: Designed with modular UI layer (currently GTK4, Qt support planned)

## Building

### Prerequisites

- Rust 1.70 or later
- GTK4 development libraries

On Debian/Ubuntu:
```bash
sudo apt install libgtk-4-dev
```

On Fedora:
```bash
sudo dnf install gtk4-devel
```

On Arch Linux:
```bash
sudo pacman -S gtk4
```

On macOS (with Homebrew):
```bash
brew install gtk4
```

### Build

```bash
# Development build
cargo build

# Release build (optimized)
cargo build --release

# Run
cargo run --release
```

The binary will be at `target/release/cterm`.

## Configuration

Configuration files are stored in:
- Linux: `~/.config/cterm/`
- macOS: `~/Library/Application Support/com.cterm.terminal/`
- Windows: `%APPDATA%\cterm\`

### Main Config (`config.toml`)

```toml
[general]
default_shell = "/bin/bash"
scrollback_lines = 10000
confirm_close_with_running = true

[appearance]
theme = "Tokyo Night"
font_family = "JetBrains Mono"
font_size = 12
cursor_style = "block"
cursor_blink = true

[tabs]
show_tab_bar = "always"
tab_bar_position = "top"

[shortcuts]
new_tab = "Ctrl+Shift+T"
close_tab = "Ctrl+Shift+W"
next_tab = "Ctrl+Tab"
prev_tab = "Ctrl+Shift+Tab"
copy = "Ctrl+Shift+C"
paste = "Ctrl+Shift+V"
```

### Sticky Tabs (`sticky_tabs.toml`)

Configure persistent tabs for Claude and other tools:

```toml
[[tabs]]
name = "Claude"
command = "claude"
color = "#7c3aed"
keep_open = true

[[tabs]]
name = "Claude (Continue)"
command = "claude"
args = ["-c"]
color = "#7c3aed"
keep_open = true
```

## Keyboard Shortcuts

| Action | Default Shortcut |
|--------|-----------------|
| New Tab | Ctrl+Shift+T |
| Close Tab | Ctrl+Shift+W |
| Next Tab | Ctrl+Tab |
| Previous Tab | Ctrl+Shift+Tab |
| Tab 1-9 | Ctrl+1-9 |
| Copy | Ctrl+Shift+C |
| Paste | Ctrl+Shift+V |
| Zoom In | Ctrl++ |
| Zoom Out | Ctrl+- |
| Reset Zoom | Ctrl+0 |
| Scroll Up | Shift+PageUp |
| Scroll Down | Shift+PageDown |

## Architecture

```
cterm/
├── crates/
│   ├── cterm-core/     # Core terminal emulation (parser, screen, PTY)
│   ├── cterm-ui/       # UI abstraction traits
│   ├── cterm-app/      # Application logic (config, sessions, shortcuts)
│   └── cterm-gtk/      # GTK4 UI implementation
└── config/             # Default configuration files
```

The modular architecture allows:
- **cterm-core**: Pure Rust terminal emulation, reusable in other projects
- **cterm-ui**: UI-agnostic traits, enabling Qt or other toolkit support
- **cterm-app**: Shared application logic between UI implementations
- **cterm-gtk**: GTK4-specific rendering and widgets

## Built-in Themes

- Default Dark
- Default Light
- Tokyo Night
- Dracula
- Nord

## Roadmap

- [ ] Selection and copy/paste
- [ ] Search in terminal output
- [ ] Split panes
- [ ] GPU-accelerated rendering option
- [ ] Qt backend
- [ ] Native macOS backend
- [ ] Windows ConPTY support
- [ ] Sixel graphics support
- [ ] Session save/restore
- [ ] Plugin system

## License

MIT License

## Contributing

Contributions are welcome! Please open an issue or pull request on GitHub.
