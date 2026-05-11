# 00 — Overview

> The architecture of WinMux in one document. Read this before any
> non-trivial work.

---

## What WinMux Is

WinMux is a Windows-native terminal multiplexer. It lets a user:

- Run multiple shell sessions in parallel.
- Detach from those sessions and reattach later, even after closing
  the GUI.
- Split a single visual area into multiple panes.
- Drive the whole thing from the keyboard, with tmux-style prefix key
  bindings.

It is built for Windows 11 first. macOS and Linux are out of scope
(code may compile but is not tested).

It is built for a single primary user on a single PC. Multi-user
isolation is required (Named Pipe ACLs and per-user `%APPDATA%`), but
the feature set is not designed for many simultaneous attachers.

It is not a tmux clone. It implements the core experience and the
common bindings, not the entire surface area of a 25-year-old
project.

---

## Three Processes

WinMux runs as three separate executables:

```
                    User
                     │
       ┌─────────────┼─────────────────────┐
       ▼             ▼                     ▼
winmux-tray.exe   winmux.exe         (third-party,
 ── tray icon       ── CLI              e.g. PowerShell
 ── main window     ── single-shot      from a script)
                    
       │             │                     │
       └─── Named Pipe: \\.\pipe\winmux-{user} ───┐
                                                  ▼
                                       winmux-server.exe
                                       ── owns ConPTY handles
                                       ── owns child shell processes
                                       ── owns virtual terminal state
                                       ── owns scrollback
                                       ── owns audit log
                                       ── runs the IPC server
                                                  │
                                                  │ ConPTY (per pane)
                                                  ▼
                                       PowerShell / cmd / pwsh / ...
```

### Why three processes

- **The GUI can die, the work can't.** If `winmux-tray.exe` crashes,
  the shells keep running. Restart the tray, reattach, continue.
- **The CLI is single-shot.** `winmux ls` shouldn't pay the cost of
  spinning up a WebView. It connects to the server, prints, exits.
- **The server has one job.** No GUI dependencies, no global UI
  state. It manages PTYs and serves clients.

### Who owns what

| Responsibility | server | tray | cli |
| --- | :---: | :---: | :---: |
| `CreatePseudoConsole`, HPCON ownership | ✓ | | |
| Child shell processes (in a Job Object) | ✓ | | |
| `alacritty_terminal` instances (virtual terminal state) | ✓ | | |
| Scrollback buffers (memory and disk) | ✓ | | |
| Named Pipe server | ✓ | | |
| `.tmux.conf` parsing | ✓ | | |
| Audit log SQLite database | ✓ | | |
| `HKCU\...\Run` autostart registration | ✓ | | |
| Single-instance enforcement (Named Mutex) | ✓ | | |
| Tauri WebView, xterm.js rendering | | ✓ | |
| Prefix key state machine | | ✓ | |
| `Ctrl+C`-as-copy when selection exists | | ✓ | |
| Tray icon and main window lifecycle | | ✓ | |
| Single-instance enforcement (Tauri plugin) | | ✓ | |
| Persisted window position and size | | ✓ | |
| Single-shot command execution then exit | | | ✓ |
| Command-line argument parsing (`clap`) | | | ✓ |

Anything not on this list is in `winmux-protocol`, the shared crate
with IPC message types and version constants.

---

## Lifecycles

### Server

1. **Start.** Triggered by either the tray, the CLI, or autostart.
   Spawned with `DETACHED_PROCESS | CREATE_NO_WINDOW` so it survives
   the spawning process.
2. **Single-instance check.** Tries to acquire a Named Mutex named
   `Local\WinMux-Server-{user-sha8}`. If it can't, another server is
   already running and this instance exits silently with status 0.
3. **Pipe setup.** Creates `\\.\pipe\winmux-{user}` with an explicit
   security descriptor (current user SID full access, everyone else
   denied) and the `FILE_FLAG_FIRST_PIPE_INSTANCE` flag.
4. **Load config.** Reads `%APPDATA%\winmux\winmux.toml`. If missing
   or invalid, falls back to defaults and logs a warning.
5. **Load `.tmux.conf`.** Path from config; default
   `%APPDATA%\winmux\winmux.conf`.
6. **Restore.** If `%APPDATA%\winmux\sessions\*.json` exists, present
   a "restore previous sessions?" decision to the first connecting
   client. (Server does not auto-restore; user choice required.)
7. **Serve.** Accept clients until shutdown.

### Server shutdown

Initiated by:

- Tray menu "Quit WinMux" → sends `Shutdown` message.
- Windows session ending (`WM_QUERYENDSESSION` / `WM_ENDSESSION`).
- Crash (panic). Best-effort cleanup via the panic hook.

Sequence:

1. Stop accepting new client connections.
2. Send `ServerBye` to all attached clients.
3. Serialize each session's metadata (not output content) to
   `%APPDATA%\winmux\sessions\<session-id>.json`.
4. For each child shell process:
   - Send `CTRL_BREAK_EVENT` to the process group.
   - Wait up to 5 seconds.
   - If still alive, `TerminateProcess`.
   - The Job Object on the server guarantees descendants die when the
     server itself exits.
5. Close all Named Pipe handles. Flush logs. Drop everything.
6. Release the Named Mutex.
7. Exit.

### Tray

1. **Start.** Either the user double-clicks the installed shortcut or
   the autostart entry runs `winmux-tray.exe` on logon.
2. **Single-instance check.** Tauri's `single-instance` plugin. If
   another tray is running, foreground it and exit.
3. **Server discovery.** Tries to connect to
   `\\.\pipe\winmux-{user}`. If the connect fails because the pipe
   does not exist, the tray spawns `winmux-server.exe` with
   `DETACHED_PROCESS | CREATE_NO_WINDOW` and retries with backoff
   (100 ms, 300 ms, 1 s, 3 s, give up).
4. **Tray icon.** Always shown. Main window is *not* shown
   automatically.
5. **First-launch toast.** Exactly once (recorded in config): "WinMux
   is running. Click the tray icon to open it."
6. **Main window on demand.** Tray icon double-click, "Open main
   window" menu item, or balloon click.

### Tray shutdown

- "Hide window" / `X` button → window hidden, tray still running,
  server untouched. A one-time toast tells the user this.
- "Quit WinMux" tray menu → sends `Shutdown` to server (server
  performs its own shutdown), then tray exits.
- User logout / system shutdown → tray sends `ClientBye`, then exits;
  Windows session-ending kicks the server through its own path.

### CLI

The CLI does one thing per invocation and exits.

```
winmux <subcommand> [args...]
```

1. Parse arguments with `clap`.
2. Connect to `\\.\pipe\winmux-{user}`. If the server isn't running:
   - `winmux ls` and read-only commands → exit with error "no server
     running" (status 2).
   - `winmux attach`, `winmux new-session`, and other write
     commands → start the server, wait up to 3 s for the pipe, then
     proceed.
3. Send the request, receive the response, print result, exit.
4. CLI never holds long-lived state.

---

## Milestones

| Milestone | What's working at the end of it |
| --- | --- |
| **M0 — PoC** | One server, one client (tray), one ConPTY. PowerShell spawns. Bytes flow both ways. Detach and reattach show the previous screen. Job Object cleanup verified. |
| **M1 — MVP** | Multiple sessions, multiple windows per session, multiple panes per window. Core prefix bindings (`c`, `d`, `n`, `p`, `%`, `"`, arrow keys, `x`, `z`, `0–9`). Basic `.tmux.conf` (Phase A). Panel layout in the GUI. |
| **M2 — Compatibility** | `winmux.exe` CLI with `ls`, `attach`, `new-session`, `kill-session`, `send-keys`, `list-windows`, `list-panes`. Copy mode (vi and emacs). Phase B `.tmux.conf`. Disk-backed scrollback. |
| **M3 — Persistence** | Session serialization and restore across server restart and reboot. Autostart toggle in tray. Toast / first-launch UX polish. Manual test checklist passes. |
| **M4 — Advanced** | Multiple clients on one session (pair programming). Hooks. `run-shell` / `if-shell` behind explicit opt-in. Possibly a plugin interface — TBD. |

These are the only commitments. Anything inside an M doesn't ship
until everything in the M is done.

---

## Build Layout

```
WinMux/
├── Cargo.toml              # workspace root
├── crates/
│   ├── winmux-protocol/    # IPC types, version constants
│   ├── winmux-server/      # background daemon
│   ├── winmux-tray/        # Tauri shell, tray icon, GUI bridge
│   └── winmux-cli/         # CLI tool
├── src/                    # React frontend, xterm.js, prefix logic
├── src-tauri/              # Tauri config and Rust glue for the tray
└── docs/                   # this directory
```

The frontend in `src/` is owned by `winmux-tray`. The server has no
frontend code.

---

## Critical Boundaries (don't cross)

- **Server depends on `winmux-protocol`** and nothing else from the
  workspace. Especially not on Tauri.
- **Tray depends on `winmux-protocol`** and on Tauri. Not on
  `portable-pty` or `alacritty_terminal`.
- **CLI depends on `winmux-protocol`** and on `clap`. Nothing else.
- **`winmux-protocol`** depends on `serde` and `thiserror`. Nothing
  else.

If a change would require a different dependency graph, the change is
wrong — discuss before writing code.

---

## Related Docs

- IPC details → [`01-ipc-protocol.md`](01-ipc-protocol.md)
- ConPTY and virtual terminal → [`02-pty-and-terminal.md`](02-pty-and-terminal.md)
- Session / window / pane model → [`03-session-model.md`](03-session-model.md)
- Key handling → [`04-key-handling.md`](04-key-handling.md)
- tmux compatibility matrix → [`05-tmux-compat.md`](05-tmux-compat.md)
- Major decisions → [`../decisions.md`](../decisions.md)
- Known issues → [`../known-issues.md`](../known-issues.md)
