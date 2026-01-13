# cterm

A high-performance, customizable terminal emulator built in pure Rust with GTK4. Features a modular architecture designed to support alternative UI backends, with optimizations for running AI coding assistants like Claude Code.

## Features

### Terminal Emulation
- **High Performance**: Custom VT100/ANSI terminal emulator with efficient screen buffer management
- **True Color Support**: Full 24-bit RGB color with 256-color palette fallback
- **Unicode Support**: Proper handling of wide characters, combining characters, and emoji
- **Scrollback Buffer**: Configurable scrollback with efficient memory usage
- **Find in Scrollback**: Search through terminal history with regex support

### User Interface
- **Tabs**: Multiple terminal tabs with keyboard shortcuts
- **Tab Customization**: Custom colors and names for tabs
- **Sticky Tabs**: Persistent tab configurations for frequently-used commands (great for Claude sessions)
- **Themes**: Built-in themes (Tokyo Night, Dracula, Nord, and more) plus custom TOML themes
- **Keyboard Shortcuts**: Fully configurable shortcuts for all actions
- **Zoom**: Adjustable font size with Ctrl+/Ctrl-

### Terminal Features
- **Hyperlinks**: Clickable URLs with OSC 8 support
- **Clipboard**: OSC 52 clipboard integration for remote copy/paste
- **Color Queries**: OSC 10/11 color query support for theme-aware applications
- **Alternate Screen**: Full alternate screen buffer support (for vim, less, etc.)

### System Integration
- **Native PTY**: Cross-platform PTY implementation (Unix openpty, Windows ConPTY ready)
- **Seamless Upgrades**: Update cterm without losing terminal sessions (Unix)
- **Auto-Update**: Built-in update checker with GitHub releases integration

## Installation

### Pre-built Binaries

Download the latest release from the [GitHub Releases](https://github.com/KarpelesLab/cterm/releases) page.

### Building from Source

#### Prerequisites

- Rust 1.70 or later
- GTK4 development libraries

**Debian/Ubuntu:**
```bash
sudo apt install libgtk-4-dev
```

**Fedora:**
```bash
sudo dnf install gtk4-devel
```

**Arch Linux:**
```bash
sudo pacman -S gtk4
```

**macOS (Homebrew):**
```bash
brew install gtk4
```

#### Build

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

Configuration files are stored in platform-specific locations:
- **Linux**: `~/.config/cterm/`
- **macOS**: `~/Library/Application Support/com.cterm.terminal/`
- **Windows**: `%APPDATA%\cterm\`

See [docs/configuration.md](docs/configuration.md) for detailed configuration options.

## Keyboard Shortcuts

| Action | Default Shortcut |
|--------|------------------|
| New Tab | Ctrl+Shift+T |
| Close Tab | Ctrl+Shift+W |
| Next Tab | Ctrl+Tab |
| Previous Tab | Ctrl+Shift+Tab |
| Switch to Tab 1-9 | Ctrl+1-9 |
| Copy | Ctrl+Shift+C |
| Paste | Ctrl+Shift+V |
| Find | Ctrl+Shift+F |
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
│   ├── cterm-app/      # Application logic (config, sessions, upgrades)
│   └── cterm-gtk/      # GTK4 UI implementation
└── docs/               # Documentation
```

The modular architecture enables:
- **cterm-core**: Pure Rust terminal emulation, reusable in other projects
- **cterm-ui**: UI-agnostic traits for toolkit abstraction
- **cterm-app**: Shared application logic between UI implementations
- **cterm-gtk**: GTK4-specific rendering and widgets

## Built-in Themes

- Default Dark
- Default Light
- Tokyo Night
- Dracula
- Nord

Custom themes can be added as TOML files in the `themes/` configuration subdirectory.

## Roadmap

- [ ] Text selection and improved copy/paste
- [ ] Split panes
- [ ] GPU-accelerated rendering
- [ ] Qt backend
- [ ] Sixel/iTerm2 graphics support
- [ ] Session save/restore across restarts
- [ ] Plugin system

## License

MIT License

## Contributing

Contributions are welcome! Please open an issue or pull request on GitHub.
