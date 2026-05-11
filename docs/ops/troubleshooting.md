# Troubleshooting

> Common problems and how to fix them.

If your problem isn't covered here, open an issue at
<https://github.com/cx8537/WinMux/issues> with the diagnostic
information described in [Diagnostic Information](#diagnostic-information)
at the bottom.

---

## Installation

### Windows SmartScreen blocks the installer

On first run of an unsigned installer, Windows shows:

> Windows protected your PC. Microsoft Defender SmartScreen prevented
> an unrecognized app from starting.

This is expected. Pre-1.0 builds of WinMux are not code-signed (see
[`../build/release.md`](../build/release.md)).

**Fix.** Click "More info" → "Run anyway." To gain confidence, verify
the SHA-256 hash of the installer against the value published in the
GitHub Release notes:

```powershell
$h = Get-FileHash -Algorithm SHA256 .\WinMux_0.2.0_x64-setup.exe
$h.Hash.ToLower()
```

### Installer says "WinMux is already installed"

The installer detected an existing install. Uninstall first, or run
the new installer in upgrade mode (it should prompt).

### Antivirus quarantines the installer

A few antivirus vendors flag unsigned Tauri apps. Submit the file for
re-scan, or whitelist `%LOCALAPPDATA%\Programs\WinMux\`. Report to us
which AV — we may submit the binary for review with that vendor.

---

## Server Won't Start

### Symptom: tray launches but says "server not responding"

Open `%APPDATA%\winmux\logs\` and look at the most recent
`server-<date>.log`. Common patterns:

#### `mutex already held: WinMux-Server-<hash>`

Another server instance is already running. Check Task Manager for
`winmux-server.exe`. If you see one, that's the live instance — try
connecting again. If you see multiple, kill all of them in Task
Manager and let the tray spawn a fresh one.

#### `failed to create named pipe: ERROR_PIPE_BUSY`

A pipe with the same name exists but is stuck. Reboot. (This is rare
on Windows 11; usually means a previous server died without releasing
the pipe.)

#### `permission denied: SID mismatch`

The server detected a client from a different user account trying to
connect. This shouldn't happen in normal use; if it does, check
whether you're running the tray under one Windows user and the server
under another (e.g., via `runas` or a service).

#### `failed to load config: ...`

The `winmux.toml` is malformed. Open it from
`%APPDATA%\winmux\winmux.toml`. If you can fix the syntax (likely a
missing quote or bracket), do so. Otherwise, rename it to
`winmux.toml.broken` and let the server recreate defaults on next
start.

### Symptom: server process exists but crashes immediately

Look for `%APPDATA%\winmux\logs\crash-server-<timestamp>.log`. This
is written by the panic hook. The first 20 lines usually show what
went wrong.

If the crash is reproducible, attach the crash log to a GitHub issue.

---

## Tray Issues

### Tray icon doesn't appear

- Right-click the taskbar → "Taskbar settings" → "Other system tray
  icons." Find `WinMux` and toggle it on. Windows often hides new
  tray icons by default.
- If WinMux is not in the list at all, the tray process isn't
  running. Launch `winmux-tray.exe` from the Start menu.

### Tray icon shows yellow exclamation

The server hasn't responded to a ping in 90+ seconds. Click the icon
for details. Options:

- **Restart server** — kills the existing server and starts a fresh
  one. Running PTYs die (they're owned by the dying server). Sessions
  may be restorable from disk on next start.
- **Wait** — sometimes the server is doing slow I/O (large disk
  scrollback writes); it will catch up.

### Main window opens off-screen

You disconnected the monitor that hosted the window. Right-click the
tray → "Open main window." The window should recenter on the primary
monitor automatically; if not, edit `%APPDATA%\winmux\winmux.toml`
under `[gui]` to clear `window_x` and `window_y`.

---

## Shell Spawn Fails

### `failed to spawn: The system cannot find the file specified`

The configured shell path doesn't exist or isn't executable. Open
Settings → Terminal → "Default shell" and pick from the autodetected
list, or browse to a valid `pwsh.exe` / `powershell.exe` / `cmd.exe`.

### Shell starts but immediately exits

Look at the pane's exit code (visible in the pane title bar when a
shell has exited). Common causes:

- **`-NoLogo` flag issue with old PowerShell:** try removing custom
  shell args.
- **`profile.ps1` crashes:** start with `pwsh -NoProfile` to confirm.
- **Working directory doesn't exist:** check `[terminal].default_cwd`
  in `winmux.toml`.

### WSL shell shows garbled output

WSL distributions sometimes don't agree with WinMux's terminal type.
Try setting `TERM=xterm-256color` in your shell's startup file (e.g.,
`~/.bashrc`). If symptoms persist (boxes for borders, wrong colors),
report the WSL distribution name and version.

---

## IME and CJK Input

### Korean / Japanese composition produces garbled characters

WinMux relies on the WebView2's composition events. If you see
half-composed characters going to the shell:

- Make sure you're on the latest Windows 11 (older builds had
  WebView2 composition bugs).
- Make sure the shell's input encoding is UTF-8:
  - PowerShell: `[Console]::InputEncoding =
    [System.Text.Encoding]::UTF8`
  - cmd: `chcp 65001`

### Prefix key triggers during composition

This is intentional: prefix is detected before IME at `keydown`. To
type the prefix character literally (e.g., to send `Ctrl+B` to the
shell), press the prefix twice (`Ctrl+B Ctrl+B`).

### CJK characters render as boxes

The font doesn't contain Korean / Japanese / Chinese glyphs. Open
Settings → Terminal → Font and ensure the font stack includes a CJK
fallback. The default is:

```
Cascadia Code, Consolas, D2Coding, 'Noto Sans Mono CJK KR', monospace
```

If none of those are installed, install one. **D2Coding** is a free
Korean developer favorite; **Noto Sans Mono CJK KR** is Google's
free option.

---

## Performance

### Typing feels laggy

- Check the tray icon for the yellow "server unresponsive" overlay.
  If present, the server is bogged down. Restart it.
- Check CPU usage of `winmux-tray.exe`. If consistently high, the
  WebView is rendering too much — try reducing scrollback line limit
  in Settings → Terminal.
- Run with `WINMUX_LOG=info` and check the server log for repeated
  WARN lines about slow IPC or slow snapshots.

### High RAM usage in the tray

Tauri WebView baseline is ~150 MiB. Each xterm.js terminal with a
large scrollback adds memory. If you have many sessions with
`scrollback_lines = 100000`, expect ~500 MiB. Reduce
`scrollback_lines` if this is a concern.

### Build (`cargo build`) crashes the tray

Massive build output can fill broadcast queues. Symptoms: the tray
slows down briefly, then "client disconnected: slow" appears in the
server log, and the GUI shows "session disconnected — reattach?"

WinMux is designed to handle this: the slow client is disconnected,
other clients (and the running build itself) continue. Just reattach.
If you see this frequently, increase the broadcast queue size in
`winmux.toml` (when implemented; M2+).

---

## Sessions Lost After Upgrade

Sessions persist as JSON in `%APPDATA%\winmux\sessions\`. If the
schema changed between versions, WinMux migrates them on startup.
If migration fails:

- A backup is written next to the original
  (`<id>.json.bak-vN-<timestamp>`).
- The session may be skipped on the "Restore previous sessions?"
  prompt.
- Check `%APPDATA%\winmux\logs\` for migration WARN/ERROR entries.

If the migration was incomplete, you can hand-edit the backup file
to match the new schema; the schema is documented in
[`../spec/08-persistence.md`](../spec/08-persistence.md).

---

## Reading the Logs

### Where logs live

```
%APPDATA%\winmux\logs\
  server-2026-05-11.log
  server-2026-05-10.log
  tray-2026-05-11.log
  tray-2026-05-10.log
  cli-2026-05-11.log
  crash-server-2026-05-11T09-32-11.log
```

### Tail a log in real time

```powershell
Get-Content "$env:APPDATA\winmux\logs\server-$(Get-Date -Format yyyy-MM-dd).log" -Wait
```

### Find recent errors

```powershell
Get-Content "$env:APPDATA\winmux\logs\server-*.log" |
    Select-String "ERROR|WARN" |
    Select-Object -Last 50
```

### Increase log level for the next session

```powershell
$env:WINMUX_LOG = "debug"
winmux start-server
```

Or set permanently in `winmux.toml`:

```toml
[logging]
level = "debug"
```

Set back to `"info"` when done — `debug` logs are voluminous.

---

## `.tmux.conf` Issues

### A directive does nothing

WinMux supports a subset of tmux directives, phased by milestone. See
[`../spec/05-tmux-compat.md`](../spec/05-tmux-compat.md) for the
matrix. If the directive is unsupported, a WARN appears in the
server log when the config is loaded.

### `run-shell` / `if-shell` is ignored

These require explicit opt-in for security reasons. Enable in
`winmux.toml`:

```toml
[security]
allow_arbitrary_commands = true
```

You will also see a GUI confirmation modal the first time after
enabling. See
[`../nonfunctional/security.md`](../nonfunctional/security.md).

### Config changes don't take effect

`winmux.conf` is reloaded on:

- `prefix + r` if bound.
- `winmux source-file` from the CLI.
- Settings → Keys → "Reload config" button.

`winmux.toml` is **not** hot-reloaded; restart the server.

---

## Network and Updates

### "Check for updates" hangs or fails

This is the only outbound network call WinMux makes. It contacts the
GitHub Releases API. Possible causes:

- **No internet.**
- **Corporate proxy.** WinMux does not have a proxy configuration UI
  in M1–M3. As a workaround, set `HTTPS_PROXY` / `HTTP_PROXY`
  environment variables before launching the tray.
- **GitHub rate limit.** Try again later.

There is no other "update" mechanism. WinMux never downloads anything
on its own.

---

## Reporting an Issue

If none of the above fixes your issue:

1. Generate a diagnostic bundle: Settings → Advanced → "Export
   diagnostic bundle." Saves a zip to your chosen location.
2. Review the zip; remove anything you don't want to share.
3. Open an issue at
   <https://github.com/cx8537/WinMux/issues> with:
   - WinMux version (`winmux --version`).
   - Windows version (`winver`).
   - Steps to reproduce.
   - The diagnostic zip (or the relevant log excerpts inline).

For security issues, see [SECURITY.md](../../SECURITY.md) — **do not
file as a public issue.**

---

## Diagnostic Information

Useful values to include in bug reports:

```powershell
# WinMux version
winmux --version

# Windows version
[System.Environment]::OSVersion
winver

# Running WinMux processes
Get-Process winmux-* -ErrorAction SilentlyContinue |
    Select-Object Name, Id, StartTime, CPU, WorkingSet64

# Server pipe status
Test-Path "\\.\pipe\winmux-*"  # PowerShell can't enumerate pipes;
# alternative: use Sysinternals' pipelist.exe

# Disk usage
Get-ChildItem "$env:APPDATA\winmux" -Recurse -File |
    Measure-Object -Property Length -Sum |
    ForEach-Object { "{0:N2} MB" -f ($_.Sum / 1MB) }
```

---

## Related Docs

- Manual verification before release →
  [`manual-test-checklist.md`](manual-test-checklist.md)
- Logging behavior → [`../nonfunctional/logging.md`](../nonfunctional/logging.md)
- Known issues → [`../known-issues.md`](../known-issues.md)
