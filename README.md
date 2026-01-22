# cterm

A high-performance, customizable terminal emulator built in pure Rust. Features native UI on macOS (AppKit/CoreGraphics) and GTK4 on Linux, with a modular architecture and optimizations for running AI coding assistants like Claude Code.

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
- **Sixel Graphics**: Inline image display with DEC Sixel protocol support
- **iTerm2 Graphics**: Inline images via OSC 1337 protocol (PNG, JPEG, GIF)
- **iTerm2 File Transfer**: Receive files via OSC 1337 with streaming support for large files
- **DRCS Fonts**: Soft font support via DECDLD for custom character sets

### System Integration
- **Native PTY**: Cross-platform PTY implementation (Unix openpty, Windows ConPTY ready)
- **Crash Recovery**: Automatic recovery from crashes - a watchdog process preserves terminal sessions and restores them after unexpected termination (macOS)
- **Seamless Upgrades**: Update cterm without losing terminal sessions (Unix)
- **Auto-Update**: Built-in update checker with GitHub releases integration

## Installation

### Pre-built Binaries

Download the latest release from the [GitHub Releases](https://github.com/KarpelesLab/cterm/releases) page.

### Building from Source

#### Prerequisites

- Rust 1.70 or later

**Linux only** - GTK4 development libraries:

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

**macOS:**
No additional dependencies required - uses native AppKit/CoreGraphics.

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

## Terminal Compatibility

### Supported DEC Private Modes (DECSET/DECRST)

| Mode | Name | Description |
|------|------|-------------|
| 1 | DECCKM | Application cursor keys |
| 6 | DECOM | Origin mode (cursor addressing relative to scroll region) |
| 7 | DECAWM | Auto-wrap mode |
| 9 | X10 Mouse | X10 mouse reporting (button press only) |
| 25 | DECTCEM | Show/hide cursor |
| 80 | DECSDM | Sixel display mode (scrolling control) |
| 1000 | — | Normal mouse tracking (button press/release) |
| 1002 | — | Button-event mouse tracking (press/release/motion with button) |
| 1003 | — | Any-event mouse tracking (all motion events) |
| 1004 | — | Focus event reporting |
| 1006 | — | SGR extended mouse coordinates |
| 1047 | — | Alternate screen buffer |
| 1048 | — | Save/restore cursor |
| 1049 | — | Alternate screen buffer with cursor save/restore |
| 2004 | — | Bracketed paste mode |

### Supported ANSI Modes (SM/RM)

| Mode | Name | Description |
|------|------|-------------|
| 4 | IRM | Insert mode |
| 20 | LNM | Line feed/new line mode |

### Supported OSC Sequences

| OSC | Description |
|-----|-------------|
| 0 | Set window title and icon name |
| 1 | Set icon name |
| 2 | Set window title |
| 8 | Hyperlinks |
| 10 | Query/set foreground color |
| 11 | Query/set background color |
| 12 | Query/set cursor color |
| 52 | Clipboard operations |
| 1337 | iTerm2 inline images and file transfer |

### Sixel Graphics

cterm supports DEC Sixel graphics for inline image display:
- Full color palette support (up to 256 colors)
- RGB and HLS color definitions
- DECSDM mode for controlling image placement and scrolling
- Images scroll with terminal content
- Grid cells under images are cleared (xterm-compatible behavior)

Test with:
```bash
# Using ImageMagick
convert image.png -resize 200x200 sixel:-

# Using libsixel
img2sixel image.png
```

### iTerm2 Graphics Protocol (OSC 1337)

cterm supports iTerm2's inline image protocol for displaying PNG, JPEG, and GIF images:
- Inline image display with `inline=1`
- File transfer with `inline=0` (shows notification bar with Save/Save As/Discard)
- Streaming file transfer support for large files (spills to disk when >1MB)
- Configurable width/height in pixels, cells, or percentages
- Aspect ratio preservation

Test with:
```bash
# Using imgcat (from iTerm2 utilities)
imgcat image.png

# Manual test (inline image)
printf '\033]1337;File=inline=1:'$(base64 < image.png)'\a'

# Manual test (file transfer)
printf '\033]1337;File=name='$(echo -n "test.bin" | base64)':'$(base64 < file.bin)'\a'
```

### DRCS (Soft Fonts)

cterm supports DECDLD (DEC Download) for custom character sets:
- Define custom glyphs via escape sequences
- Multiple font sizes supported
- Designate fonts to G0/G1 character sets

## Architecture

```
cterm/
├── crates/
│   ├── cterm-core/     # Core terminal emulation (parser, screen, PTY)
│   ├── cterm-ui/       # UI abstraction traits
│   ├── cterm-app/      # Application logic (config, sessions, upgrades, crash recovery)
│   ├── cterm-gtk/      # GTK4 UI implementation (Linux)
│   └── cterm-cocoa/    # Native macOS UI using AppKit/CoreGraphics
└── docs/               # Documentation
```

The modular architecture enables:
- **cterm-core**: Pure Rust terminal emulation, reusable in other projects
- **cterm-ui**: UI-agnostic traits for toolkit abstraction
- **cterm-app**: Shared application logic between UI implementations
- **cterm-gtk**: GTK4-specific rendering and widgets (Linux, cross-platform)
- **cterm-cocoa**: Native macOS implementation using AppKit and CoreGraphics

## Built-in Themes

- Default Dark
- Default Light
- Tokyo Night
- Dracula
- Nord

Custom themes can be added as TOML files in the `themes/` configuration subdirectory.

## Roadmap

- [x] Text selection and copy/paste
- [x] Crash recovery (macOS)
- [x] Sixel graphics support
- [x] iTerm2 graphics protocol (OSC 1337)
- [x] DRCS soft font support
- [ ] Split panes
- [ ] Qt backend
- [ ] Session save/restore across restarts
- [ ] Plugin system

## License

MIT License

## Contributing

Contributions are welcome! Please open an issue or pull request on GitHub.
