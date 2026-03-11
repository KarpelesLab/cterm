# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

cterm is a high-performance terminal emulator written in pure Rust. It uses native AppKit/CoreGraphics on macOS and GTK4 on Linux/Windows.

## Build Commands

```bash
cargo build                              # Debug build
cargo build --release                    # Release build (LTO enabled)
cargo run --release                      # Build and run
./run.sh                                 # Quick build + run script

cargo test --all                         # Run all tests (macOS)
cargo test --workspace --exclude cterm-cocoa  # Run tests (Linux/Windows)
cargo test -p cterm-core                 # Run tests for a specific crate

cargo fmt --all                          # Format code
cargo fmt --all -- --check               # Check formatting

cargo clippy --workspace --all-targets   # Lint (macOS)
cargo clippy --workspace --exclude cterm-cocoa --all-targets -- -D warnings  # Lint (Linux/Windows CI)
```

**Linux prerequisites**: `libgtk-4-dev libadwaita-1-dev libpango1.0-dev libcairo2-dev libglib2.0-dev`

## Architecture

```
crates/
├── cterm-core/    # Terminal emulation: VT parser, screen buffer, PTY, Sixel graphics
├── cterm-ui/      # UI abstraction traits (TerminalView, TabBar, Window)
├── cterm-app/     # Application logic: config, sessions, upgrades, daemon reconnect
├── cterm-cocoa/   # macOS native UI (AppKit, CoreGraphics)
├── cterm-gtk/     # GTK4 UI (Linux, Windows, cross-platform)
├── cterm-client/  # gRPC client for connecting to ctermd daemon
├── cterm-proto/   # Protobuf/gRPC service definitions
└── cterm-headless/# ctermd daemon: owns PTYs, sessions survive UI restarts
```

### Core Data Flow

1. `ctermd` daemon owns PTY sessions; `cterm` UI connects via gRPC
2. PTY data → daemon → gRPC stream → client `vte` parser → Screen buffer → UI renderer
3. User input → UI events → gRPC write → daemon → PTY

### Key Abstractions

- **Terminal** (`cterm-core/term.rs`): High-level API combining Screen, Parser, and PTY
- **Screen** (`cterm-core/screen.rs`): Display buffer with scrollback, selections, inline images
- **Grid** (`cterm-core/grid.rs`): Efficient cell storage with attributes and hyperlinks
- **Parser** (`cterm-core/parser.rs`): ANSI/VT sequence handling via `vte` crate
- **Config** (`cterm-app/config.rs`): TOML configuration parsing

### Platform-Specific Code

- `cterm-cocoa`: Uses `objc2-app-kit` for native macOS rendering
- `cterm-gtk`: Uses `gtk4`, `cairo-rs` for cross-platform rendering
- Conditional compilation separates platform code; cterm-cocoa is excluded on non-macOS builds

### Special Features

- **Daemon Architecture**: `ctermd` owns all PTYs; sessions survive cterm UI restarts/upgrades
- **Seamless Upgrades** (`cterm-app/upgrade/`): Save window layout to temp file, spawn new binary, reconnect to daemon sessions
- **Graphics**: Sixel (`sixel.rs`), iTerm2 OSC 1337 (`iterm2.rs`), DRCS soft fonts (`drcs.rs`)
- **Streaming Files** (`streaming_file.rs`): Large file transfers spill to disk above 1MB

## Configuration

Config locations:
- macOS: `~/Library/Application Support/com.cterm.terminal/`
- Linux: `~/.config/cterm/`
- Windows: `%APPDATA%\cterm\`

Files: `config.toml`, `sticky_tabs.toml`, `themes/*.toml`

## Workflow

### Before Committing

Always run `cargo fmt --all` before committing to ensure consistent formatting.

### Release Process

When creating a new release:

1. Update `version` in `Cargo.toml` (under `[workspace.package]`)
2. Run `cargo fmt --all`
3. Commit the version bump
4. Tag the commit: `git tag vX.Y.Z`
5. Push commit and tag: `git push && git push origin vX.Y.Z`

Tags trigger GitHub Actions to build release binaries. Never delete/recreate tags once pushed—they become releases with published artifacts.
