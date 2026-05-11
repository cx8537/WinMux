# Manual Test Checklist

> Things to verify by hand before each release.

The E2E test suite catches regressions in protocol, persistence, and
core lifecycle. It does **not** catch visual glitches, IME oddities,
or platform-specific UX problems. This checklist does.

Use this for every release tag. Document any deviations in the GitHub
Release notes.

Format: each item has a checkbox. Strike through items that don't
apply to the current release (e.g., M1 doesn't have copy mode yet).

---

## Pre-flight

- [ ] Tag is correct in `package.json`, `Cargo.toml`, and
  `tauri.conf.json`.
- [ ] `CHANGELOG.md` entry exists for the version.
- [ ] CI is green on the tag commit.
- [ ] Installer artifact downloaded from CI.
- [ ] Installer SHA-256 hash recorded.
- [ ] Test machine: clean Windows 11, no prior WinMux install.

---

## Installation

- [ ] SmartScreen warning appears. Click "More info" → "Run anyway"
  works.
- [ ] Installer language picker shows English and Korean.
- [ ] Installation completes without admin prompt (per-user install).
- [ ] Installed to `%LOCALAPPDATA%\Programs\WinMux\`.
- [ ] Start menu entry created.
- [ ] No `HKLM` registry entries created.
  ```powershell
  reg query "HKLM\Software\WinMux" 2>$null # should be empty
  ```

---

## First Launch

- [ ] Launch from Start menu.
- [ ] Tray icon appears within 3 seconds.
- [ ] First-run toast appears: "WinMux is running. Click the tray
  icon to open it."
- [ ] Main window is **not** open yet.
- [ ] `winmux-server.exe` is running in Task Manager.
- [ ] `winmux-tray.exe` is running in Task Manager.

---

## Tray Icon

- [ ] Single left-click opens the main window.
- [ ] Right-click shows the context menu:
  - WinMux version line
  - Open main window
  - New session…
  - Sessions ▶ (empty submenu initially)
  - Settings…
  - About WinMux
  - Check for updates
  - Quit WinMux
- [ ] Closing main window via X hides it; tray icon remains.
- [ ] Reopening from tray icon shows the previous state.

---

## Main Window

- [ ] Window opens at a sensible default size on the primary monitor.
- [ ] Resizing the window updates `winmux.toml` `[gui]` on close.
- [ ] Moving the window updates `[gui]` on close.
- [ ] Closing and reopening restores position.
- [ ] DPI change: drag the window between monitors with different
  scaling. Terminal text re-renders cleanly.
- [ ] High contrast mode (Settings → Accessibility → Contrast themes
  → High contrast Black): UI remains legible. No invisible text.

---

## Session and Pane

- [ ] Create a new session via "+ New session" button.
- [ ] Default shell is `pwsh.exe` if installed, else `powershell.exe`,
  else `cmd.exe`. Verify in log: `session.created shell=...`.
- [ ] Type `echo hello`. Output appears.
- [ ] Split pane horizontally via `prefix + %`.
- [ ] Split pane vertically via `prefix + "`.
- [ ] Each pane has independent input/output.
- [ ] Drag the divider between panes. Resize sends update.
- [ ] `prefix + arrow` navigates between panes.
- [ ] `prefix + z` toggles zoom on the active pane.
- [ ] `prefix + x` then `y` kills the active pane.

---

## Multiple Sessions

- [ ] Create three sessions: `work`, `build`, `docs`.
- [ ] Sessions panel on left shows all three.
- [ ] Clicking a session switches to it instantly.
- [ ] Right-click on a session shows Rename and Kill options.
- [ ] Renaming works; new name appears immediately.
- [ ] Tray menu Sessions submenu lists all three.

---

## Detach and Reattach

- [ ] In a session, run `vim` (or any alt-screen program) and edit
  some text.
- [ ] Close the main window via X button. Sessions persist (tray
  shows them).
- [ ] Reopen the main window. Attach to the same session. The vim
  screen is restored exactly. **No flashing previous content.**
- [ ] Quit the tray via "Quit WinMux" tray menu. All processes exit.
- [ ] Relaunch WinMux. The "Restore previous sessions?" modal appears
  if you had sessions running at quit.
- [ ] Choosing "Skip" creates a clean state.
- [ ] Choosing "Restore all" recreates sessions with fresh shells in
  the same layout.

---

## IME (Korean — primary)

- [ ] Switch input language to Korean.
- [ ] Type `안녕하세요`. Composition shows in the cursor, then commits
  on space or punctuation.
- [ ] No garbled characters in the shell.
- [ ] Press `Ctrl+B` (prefix) during composition. Composition
  cancels; prefix activates.

---

## IME (Japanese, secondary)

- [ ] Switch to Microsoft IME for Japanese.
- [ ] Type `こんにちは` via romaji input.
- [ ] Composition behaves the same as Korean.

---

## Clipboard

- [ ] Select text in a pane with mouse.
- [ ] Press `Ctrl+C`. Text is copied to clipboard. **No SIGINT sent
  to shell.** (Run `sleep 60` first to verify it isn't interrupted.)
- [ ] With no selection, press `Ctrl+C`. The shell's running command
  is interrupted.
- [ ] `Ctrl+V` pastes from clipboard via bracketed paste.

---

## Settings

- [ ] Open Settings from tray menu.
- [ ] General tab: language switcher works. Switching to Korean
  re-renders UI in Korean immediately.
- [ ] Terminal tab: change font family. New panes use the new font.
- [ ] Terminal tab: change scrollback line limit. Setting saved.
- [ ] Keys tab: change prefix. New prefix is active immediately for
  existing panes.
- [ ] Security tab: toggle "Default disk scrollback" — confirmation
  modal appears with the warning.
- [ ] Advanced tab: "Export diagnostic bundle" creates a zip.
- [ ] Inspect zip: contains logs from last 7 days, sanitized config,
  no PTY content.

---

## Autostart

- [ ] Settings → General → Autostart toggle ON.
- [ ] Check registry:
  ```powershell
  reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v WinMux
  ```
- [ ] Reboot Windows.
- [ ] Tray launches automatically. Server starts.
- [ ] Settings → General → Autostart toggle OFF.
- [ ] Registry value removed.

---

## CLI

- [ ] Open a separate PowerShell window.
- [ ] `winmux --version` shows the correct version.
- [ ] `winmux ls` lists current sessions.
- [ ] `winmux new-session -s scratch -d` creates a detached session.
- [ ] In the GUI, the new session appears in the list.
- [ ] `winmux send-keys -t scratch:0.0 "echo X" Enter`. Output
  appears in the pane.
- [ ] `winmux capture-pane -t scratch:0.0 -p` prints "X".
- [ ] `winmux kill-session -t scratch`. Session disappears.
- [ ] `winmux ls --json | ConvertFrom-Json` works for scripting.

---

## CJK Font Fallback

- [ ] In a pane, run a command that prints Korean characters:
  ```powershell
  echo "안녕하세요"
  ```
- [ ] Characters render as glyphs, not boxes.
- [ ] Wide characters take 2 cells (cursor advances correctly).
- [ ] In Settings, change font to one without CJK support (e.g.,
  `Cascadia Code` only with fallback disabled — for testing). The
  fallback chain kicks in.

---

## Performance Smoke

- [ ] Type in a pane. Visual feedback is "instant" (under 50 ms by
  feel).
- [ ] Run a high-volume command:
  ```powershell
  1..10000 | ForEach-Object { "line $_" }
  ```
  Output streams without freezing the GUI.
- [ ] Idle for 1 minute. CPU usage of WinMux processes is < 2%
  combined.
- [ ] After 10 minutes of normal use, memory is stable (no obvious
  leak).

---

## Server Restart

- [ ] In Task Manager, end `winmux-server.exe`.
- [ ] Tray icon shows yellow exclamation within 90 seconds.
- [ ] Click "Restart server" in the tray menu.
- [ ] New server starts. Previous sessions' shells are gone (expected).
- [ ] Restore modal offers previous sessions.

---

## Crash Recovery

- [ ] Trigger a known panic case (in dev: a debug-only `cargo run -p
  winmux-server -- --panic-test`). In a release context, skip or
  simulate via Task Manager force-end.
- [ ] Crash log appears in `%APPDATA%\winmux\logs\`.
- [ ] Crash log contains: panic message, location, backtrace.
- [ ] Crash log does **not** contain: PTY content, env values.

---

## Upgrade Path

- [ ] Install previous version (if applicable).
- [ ] Create sessions, configure settings.
- [ ] Run new installer. Confirm it offers upgrade.
- [ ] After upgrade, settings preserved, sessions restorable.
- [ ] Schema migration log entry exists (if schema bumped).

---

## Uninstall

- [ ] Settings → autostart OFF (to clean registry first).
- [ ] Quit WinMux.
- [ ] Uninstall via Settings → Apps → Installed apps → WinMux.
- [ ] `%LOCALAPPDATA%\Programs\WinMux\` is removed.
- [ ] Start menu entry removed.
- [ ] `HKCU\...\Run\WinMux` removed (already off, double-check).
- [ ] **`%APPDATA%\winmux\` is preserved** (user data; documented
  behavior).
- [ ] Reinstall: existing config and sessions appear in the new
  install (because `%APPDATA%\winmux\` was kept).

---

## SmartScreen Verification

- [ ] On a different Windows 11 machine (fresh install), download
  the installer from the GitHub Release page.
- [ ] SmartScreen appears.
- [ ] Verify SHA-256:
  ```powershell
  (Get-FileHash -Algorithm SHA256 .\WinMux_<version>_x64-setup.exe).Hash.ToLower()
  ```
  Matches the hash in the Release notes.
- [ ] "More info" → "Run anyway" installs successfully.

---

## E2E Reference

The automated E2E scenarios listed in
[`../nonfunctional/testing.md`](../nonfunctional/testing.md) cover:

1. Basic lifecycle.
2. Persistence across tray restart.
3. vim screen restore.
4. Clear and reconnect.
5. High-volume output.
6. Multiple clients (M4 only).
7. Slow client isolation.
8. CLI roundtrip.
9. Protocol mismatch.
10. Graceful shutdown.

If you find a regression manually that should have been caught
automatically, **add a regression test** in the same PR that fixes
the bug.

---

## Sign-off

- [ ] All items above checked or explicitly N/A.
- [ ] Any deviations noted in the GitHub Release notes.
- [ ] Release published.

---

## Related Docs

- Release process → [`../build/release.md`](../build/release.md)
- Automated test strategy →
  [`../nonfunctional/testing.md`](../nonfunctional/testing.md)
- Troubleshooting users may hit → [`troubleshooting.md`](troubleshooting.md)
