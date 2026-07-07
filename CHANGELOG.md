# Changelog

All notable changes to cterm are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project loosely follows semantic versioning. The whole workspace shares
a single version via `[workspace.package]`. Pure CI, lint, and formatting churn
is omitted for readability.

## [Unreleased]

## [0.0.18] - 2026-07-07

### Added
- Native SSH via puressh, replacing the system `ssh` binary; the self-updater
  now uses rsurl instead of reqwest (MSRV 1.88).
- Interactive SSH auth for the remote tunnel: keyboard-interactive, plus native
  host-key / password / passphrase prompt dialogs on macOS, GTK, and Windows.
- Run gRPC directly over the SSH channel — no locally forwarded socket file.
- Jump-host chains via a `>` separator (`bastion:2222>10.0.0.5`), plus SSH
  connection history in the connect dialog.
- Default `~/.ssh/id_*` identity files are loaded automatically (including
  PKCS#1/SEC1/PKCS#8 and `id_xmss`); identities are offered lazily via their
  `.pub` and only decrypted on demand.
- zlib compression (`zlib@openssh.com`) on the gRPC daemon tunnel, cutting the
  transfer for screen snapshots and scrollback.
- Mouse-event forwarding and alternate-scroll in the GTK and Windows terminals.

### Fixed
- Reconnecting a window with many tabs no longer stalls: sessions attach
  concurrently, RPCs no longer serialize on the connection mutex, and each tab
  fetches its screen snapshot only once (no redundant scrollback transfer or
  placeholder resize).
- Detect a stale `ctermd` socket by connecting rather than trusting the PID
  file, and prevent a daemon deadlock from hanging cterm startup.
- Cross-platform SSH build fixes (gate ssh-agent to Unix; Windows
  `EM_SETPASSWORDCHAR` cast).

## [0.0.17] - 2026-06-22

### Added
- Hyperlink (OSC 8) rendering, hover, and interaction across GTK, macOS, and Windows.
- Streaming input RPC with batched fallback for low-latency typing.
- Custom SSH port in remote dialogs (`user@host:port`).
- Scrollbar overlay for the terminal view (macOS + GTK).
- Bell/alert state managed through the `ctermd` daemon; alerted tabs are visually distinct.
- Serialize DRCS soft fonts and charset state across gRPC reconnection.
- Confirm close when a foreground process is running in daemon sessions; auto-close tabs when the shell exits.
- New tabs inherit daemon context from the current tab (macOS); SSH Remote attaches to all existing sessions.
- Disconnect action in the remote tab right-click menu.
- Raise the gRPC message size limit to 64 MB for large scrollback snapshots.
- Enable SSH compression (`-C`) on remote tunnels by default.

### Fixed
- Keep word/line selection stable across scrollback wrap.
- Connect to the correct daemon for remote SSH sessions; keep the SSH tunnel alive across tokio runtimes.
- GTK4 tab bar styling, close button, auto-close on shell exit, and Ctrl+PageUp/PageDown navigation.
- Persist custom tab title to the daemon from the Set Title menu action.
- Double-borrow panic when closing a tab via the close button.

### Removed
- Experimental mosh, Latch, relay, and "unixshells" integrations (prototyped during this cycle, then removed before release).

## [0.0.16] - 2026-03-15

Daemon-centric architecture: all sessions now run through `ctermd`.

### Added
- Route all terminal sessions through the `ctermd` daemon, with attach/detach semantics so sessions survive UI restarts and seamless upgrades.
- New `cterm-client` library and `cterm-proto` crate for daemon communication over gRPC.
- Daemon session reconnection, lifecycle management, and graceful SIGTERM shutdown.
- Remote host management with automatic `ctermd` install; SSH remote support over stdio/socket forwarding.
- Incremental screen updates in `StreamScreenUpdates`.
- Persist tab metadata (color, title, template) in the daemon.
- macOS daemon-backed terminal view and session menu; GTK daemon-backed terminal widget.
- "Kill Local ctermd" and relaunch-in-place debug menu items across all frontends.

### Changed
- Simplified the upgrade protocol; removed standalone crash recovery in favor of daemon-backed sessions.

### Fixed
- Preserve screen state, custom tab titles, colors, template, window frame, and active tab across daemon relaunch/upgrade.
- Smarter daemon auto-shutdown by tracking active streams; destroy sessions on tab close.
- Raise the file descriptor limit at startup; restore it for child processes.

## [0.0.15] - 2026-03-10

### Added
- macOS: render bold, italic, and dim (SGR 2) text; `bold_is_bright` option.
- GTK4: input method (IM) support for Japanese/CJK input, Ctrl+PageUp/PageDown tab switching, and libadwaita menu styling with visible keyboard shortcuts.
- Include dots in word selection (e.g. version strings).

### Fixed
- Word selection across wrapped line boundaries.
- GTK4: reset scroll to bottom on input, scrollback rendering on mouse scroll, Ctrl+Shift shortcuts, menu display, and window title on tab switch.
- Close the PTY master FD on tab close (FD leak); set `FD_CLOEXEC` on PTY master and watchdog socket FDs.
- Capture the executable path at startup for reliable relaunch.

## [0.0.14] - 2026-02-21

### Added
- Auto-scroll when dragging a selection beyond the terminal bounds.
- Show open tabs with custom names in Quick Launch.
- Expand shell variables (`~`, `$HOME`, `${VAR}`) in config paths.
- Cmd+Shift+T shortcut for Set Title.

### Fixed
- Address security-audit findings for input bounds and file safety.
- Theme selection now persists across restarts.
- Draw full-width underline/strikethrough/overline and background for wide characters on macOS.
- Preserve word/line selection anchor across drag-direction changes.

## [0.0.13] - 2026-02-13

### Added
- Set Title and Set Tab Color in the native tab context menu.

### Fixed
- Preserve custom titles, tab colors, and cwd across upgrades.
- Guard against use-after-free by checking `view_invalid` inside dispatch blocks.

## [0.0.12] - 2026-02-08

### Added
- OS-specific icon templates per platform; macOS full-canvas app icon.

### Fixed
- Spill scrollback to temp files during upgrade to avoid the 64 MB buffer limit.

## [0.0.11] - 2026-02-08

### Added
- macOS code signing and notarization in CI; `workflow_dispatch` trigger for manual builds.

### Fixed
- Find the signing identity dynamically from the keychain.

## [0.0.10] - 2026-02-07

### Added
- Configurable Tools menu with external tool shortcuts.
- File drag-and-drop support with an options dialog.
- "Next Alerted Tab" shortcut to cycle through bell-active tabs (all platforms).
- GTK tab context menu: rename and set color.

### Fixed
- Preserve word/line selection on mouseUp instead of clearing it.
- Emit the Bell event from the terminal process loop.

## [0.0.9] - 2026-02-06

### Added
- Confirm before closing tabs or quitting with running processes.
- Bell/alert notifications: macOS dock badge with count, and Windows support.
- GTK cross-platform seamless upgrade support.
- Windows Quick Open dialog and upgrade receiver.
- UI screenshot tests for Linux and macOS; window positioning menu items.

### Fixed
- Async OSC 52 clipboard query (GTK); Docker tab creation from the picker (win32).
- Preserve word/line anchor when extending a selection backwards.
- Skip the close-confirmation dialog during relaunch.

## [0.0.8] - 2026-01-30

### Added
- Command+1–9 shortcuts for tab selection.
- Platform-specific default fonts.
- Windows UI integration test infrastructure (PowerShell automation).

### Fixed
- Windows rendering not updating after PTY data; DirectWrite `E_INVALIDARG` on startup.
- Windows UI freeze and double-input bugs.
- Draw the cursor at double width for wide (CJK) characters.
- Send readline-style sequences for Option+Arrow on macOS.
- Preserve tab order during relaunch; various Quick Open input fixes.

## [0.0.7] - 2026-01-27

### Added
- Quick Open Template overlay (Cmd+G / Ctrl+Shift+G).
- New tabs inherit the working directory from the active terminal.
- Right-click tab context menu for rename and color.
- Dedicated Git Sync tab in preferences with a Sync Now button (macOS + Windows).
- Dynamically generated app icons with the version number; macOS app icon.
- `ctermd --scrollback` option; macOS auto-update installs the full app bundle.

### Changed
- Upgrade protocol now uses JSON with a versioned, backward-compatible header.

### Fixed
- Maintain scroll position when viewing scrollback history.
- Clear selection when the selected text is deleted or modified.
- Use `modes.show_cursor` for DECTCEM cursor visibility.

## [0.0.6] - 2026-01-26

### Added
- `ctermd` headless terminal daemon with a gRPC API (plus integration tests).
- Git-backed configuration sync and git remote support for tab templates.
- Open-tab-from-template feature (GTK).
- Configurable `TERM`/`COLORTERM` and focus-event support; locked background color for templates.

### Fixed
- Save crash state before relaunch to preserve buffers.
- proto3 optional support for `tonic-build`; CI build improvements.

## [0.0.5] - 2026-01-25

### Added
- Native Windows UI (`cterm-win32`) with feature parity to GTK/macOS.
- Windows seamless upgrade protocol.
- File transfer support across all platforms; GTK file transfer and Docker status display.
- macOS Check for Updates menu item.

### Changed
- Consolidate shared dialog code into `cterm-app`; consolidate PTY ownership and fix Windows DLL bundling.

### Fixed
- Numerous win32 build, Direct2D, and API-alignment fixes for the `windows` crate 0.60/0.61.
- Restore all tabs during a macOS seamless upgrade.

## [0.0.4] - 2026-01-24

### Added
- SSH remote connection support for tab templates.
- Full `devcontainer.json` support with Dockerfile building; auto-detect `devcontainer.json`.
- Tab color picker and modifier-key support; snap window resize to the character grid.
- In-app log viewer for debugging; reorganized template UI (General/Docker/Remote tabs).
- `CLAUDE.md` guidance file and `run.sh` helper.

### Fixed
- Restore window position, size, and all tabs after relaunch.

## [0.0.3] - 2026-01-24

### Added
- Graphics: Sixel, DRCS soft fonts, iTerm2 inline images (OSC 1337), and streaming file transfer for large files.
- OSC 8 hyperlink support; block/rectangular selection (Option+drag); mouse reporting; IME for Japanese/CJK input.
- Crash recovery with an FD-passing watchdog, periodic state saving, and display restoration.
- Docker configuration in the Tab Templates UI; devcontainer support.

### Fixed
- Many macOS fixes: focus/activation, scrollback scroll wheel, view resize, and several segfaults.
- Resize the tab-stops array when terminal dimensions change.

## [0.0.2] - 2026-01-21

### Added
- Native macOS UI using AppKit with CoreGraphics text rendering.
- Text selection with mouse support; tab templates with unique tabs; preferences window.
- State-preserving debug relaunch; secret debug menu; native window tabbing and keyboard shortcuts.
- Copy/paste/select-all; warn when closing a terminal with a running process.

### Changed
- Unify the binary entry point with platform-specific backends; remove redundant backend binaries.

### Fixed
- Arrow and special key handling; segfault on Command+W; focus handling after tab switch/upgrade; selection color inversion.

## [0.0.1] - 2026-01-14

Initial pre-release.

### Added
- Initial cterm terminal emulator: VT parser, screen buffer with scrollback, and a native PTY implementation.
- Menu bar (File, Edit, Terminal, Tabs, Help); clipboard paste, zoom, tab stops, and DSR.
- Bell notification indicators; tab system with Ctrl+Shift shortcuts.
- Auto-update tab/window title from the terminal; Docker terminal tabs; hidden Debug submenu.
- Seamless upgrade system for live process updates; multi-platform GitHub Actions builds.

### Changed
- Replace `portable-pty` with a unified native PTY implementation.

[Unreleased]: https://github.com/KarpelesLab/cterm/compare/v0.0.17...HEAD
[0.0.17]: https://github.com/KarpelesLab/cterm/compare/v0.0.16...v0.0.17
[0.0.16]: https://github.com/KarpelesLab/cterm/compare/v0.0.15...v0.0.16
[0.0.15]: https://github.com/KarpelesLab/cterm/compare/v0.0.14...v0.0.15
[0.0.14]: https://github.com/KarpelesLab/cterm/compare/v0.0.13...v0.0.14
[0.0.13]: https://github.com/KarpelesLab/cterm/compare/v0.0.12...v0.0.13
[0.0.12]: https://github.com/KarpelesLab/cterm/compare/v0.0.11...v0.0.12
[0.0.11]: https://github.com/KarpelesLab/cterm/compare/v0.0.10...v0.0.11
[0.0.10]: https://github.com/KarpelesLab/cterm/compare/v0.0.9...v0.0.10
[0.0.9]: https://github.com/KarpelesLab/cterm/compare/v0.0.8...v0.0.9
[0.0.8]: https://github.com/KarpelesLab/cterm/compare/v0.0.7...v0.0.8
[0.0.7]: https://github.com/KarpelesLab/cterm/compare/v0.0.6...v0.0.7
[0.0.6]: https://github.com/KarpelesLab/cterm/compare/v0.0.5...v0.0.6
[0.0.5]: https://github.com/KarpelesLab/cterm/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/KarpelesLab/cterm/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/KarpelesLab/cterm/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/KarpelesLab/cterm/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/KarpelesLab/cterm/releases/tag/v0.0.1
