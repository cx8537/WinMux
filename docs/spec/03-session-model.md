# 03 — Session Model

> Sessions, windows, panes, and the shells underneath.

WinMux's session model follows tmux's three-level hierarchy. The shape
is familiar; the implementation is Windows-native.

---

## Hierarchy

```
Session                 ── a named workspace
├─ Window               ── a tab; full-screen group of panes
│  ├─ Pane              ── one terminal (one PTY, one VirtualTerm)
│  ├─ Pane
│  └─ Pane
├─ Window
└─ Window
```

- A session has 1..N windows.
- A window has 1..N panes, arranged in a tree layout (see Layouts).
- A pane has exactly one PTY and one shell.

---

## Identifiers

| ID | Format | Notes |
| --- | --- | --- |
| `SessionId` | `ses-<ULID>` | Stable for session lifetime |
| `WindowId` | `win-<ULID>` | Unique within a session |
| `PaneId` | `pane-<ULID>` | Globally unique |
| Session name | user string | Display label; not an ID |
| Window index | `u8`, starts at 0 | tmux-style `session:0`, `session:1` |
| Pane index | `u8`, starts at 0 | tmux-style `session:0.0` |

tmux-style target strings (`work:0.1` = session `work`, window 0,
pane 1) are accepted by the CLI and the `Command` IPC message; the
server resolves them to IDs.

---

## Session

```rust
pub struct Session {
    pub id: SessionId,
    pub name: String,                  // user-facing label
    pub created_at: DateTime<Utc>,
    pub windows: Vec<Window>,
    pub active_window: WindowId,
    pub default_shell: ShellRef,       // shell for new windows/panes
    pub default_cwd: PathBuf,
    pub env_overrides: BTreeMap<String, String>,
    pub persist_scrollback: bool,      // opt-in disk mirror flag
}
```

A session is a named workspace. The user can have many sessions; the
GUI shows them in a session list.

### Creation

`NewSession` IPC message → server creates a `Session`, creates one
default `Window` with one `Pane`, spawns the configured shell, replies
`Attached`.

### Renaming

`Command { tmux: "rename-session" }` updates `session.name`. Emits
`SessionRenamed`.

### Killing

`KillSession` shuts down every pane (and child shell) in every window
of the session. The session is removed from the registry. Persisted
state is deleted from `%APPDATA%\winmux\sessions\<id>.json`.

---

## Window

```rust
pub struct Window {
    pub id: WindowId,
    pub session_id: SessionId,
    pub index: u8,                     // tmux-style index
    pub name: Option<String>,          // user override; otherwise pane title
    pub layout: PaneLayout,
    pub panes: Vec<Pane>,
    pub active_pane: PaneId,
    pub flags: WindowFlags,
}

bitflags! {
    pub struct WindowFlags: u8 {
        const BELL_ALERT     = 1 << 0; // any pane rang BEL
        const ACTIVITY_ALERT = 1 << 1; // any pane produced output since last view
        const SILENCE_ALERT  = 1 << 2; // explicit silence-monitor (M4)
    }
}
```

A window is a full-screen group of panes. From the user's perspective,
this is "a tab."

### Index

Indices start at 0 and increment as windows are created. When window
0 is deleted and a new window is created, the new window gets the
next available index (not reused), unless `renumber-windows` is set.

### Layouts

```rust
pub enum PaneLayout {
    Single(PaneId),
    Split {
        direction: SplitDirection,
        ratio: f32,                    // 0.0..=1.0, position of the divider
        first: Box<PaneLayout>,
        second: Box<PaneLayout>,
    },
}

pub enum SplitDirection { Horizontal, Vertical }
```

Recursive binary tree. Every leaf is a pane.

- `Horizontal` = top/bottom split.
- `Vertical` = left/right split.

(Naming intentionally matches tmux's `split-window -h` and `-v`,
which set the *direction of the dividing line*: `-h` splits
horizontally meaning left/right. To avoid confusion, the CLI accepts
both tmux flag names and our internal names.)

When a pane is killed, its sibling absorbs the freed area.

### Preset layouts

tmux's preset layouts are supported in Phase B:

- `even-horizontal`
- `even-vertical`
- `main-horizontal`
- `main-vertical`
- `tiled`

---

## Pane

```rust
pub struct Pane {
    pub id: PaneId,
    pub window_id: WindowId,
    pub size: PaneSize,                // rows, cols, pixel size hint
    pub shell: ShellRef,
    pub cwd: PathBuf,
    pub pty: Pty,                      // owns ConPTY + Job
    pub vterm: VirtualTerm,
    pub scrollback: Scrollback,
    pub title: Option<String>,         // from OSC 0/2
    pub state: PaneState,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub exit_code: Option<i32>,
}

pub enum PaneState { Alive, Dead, Zoomed }

pub struct PaneSize { pub rows: u16, pub cols: u16 }
```

### Active pane

Each window has exactly one active pane. Keyboard input goes there.
The GUI highlights it. tmux-style commands targeting "the pane"
without explicit target use this one.

### Zoom

`prefix + z` toggles a pane to full window size. While zoomed:

- The pane occupies the whole window area in the GUI.
- Other panes are hidden but their PTYs continue running.
- `PaneState` is `Zoomed`.
- Unzooming restores the layout.

### Dead panes

A pane whose shell exited becomes `Dead`. The scrollback remains
viewable. Closing the dead pane removes it from the layout.

`set -g remain-on-exit on` (tmux convention) is the default. To
auto-close, set `remain-on-exit off` in `.tmux.conf`.

---

## Shells

```rust
pub struct ShellRef {
    pub path: PathBuf,                 // canonical executable path
    pub display_name: String,          // "pwsh", "powershell", "cmd"
    pub args: Vec<String>,             // additional arguments
}
```

### Default shell selection

On first launch, the server picks a default shell in this order:

1. `winmux.toml` `default_shell` if set and the path exists.
2. `.tmux.conf` `default-shell` if set and the path exists.
3. `pwsh.exe` (PowerShell 7) on PATH.
4. `powershell.exe` (Windows PowerShell) on PATH.
5. `cmd.exe` (always available on Windows).

The chosen shell is logged at INFO.

### Per-session override

`NewSession { shell: Some(path), ... }` overrides the default for
this session only. The CLI's `winmux new-session -s name -- pwsh
-NoLogo` passes the shell and its arguments.

### WSL

WSL distributions are allowed shells. The user can configure:

```toml
default_shell = "wsl.exe"
default_shell_args = ["-d", "Ubuntu"]
```

WinMux itself does not require WSL; this is purely "you have WSL,
use a WSL shell as your terminal."

### Git Bash, MSYS2, Cygwin, Nushell, …

Anything that speaks the ConPTY contract works. WinMux does not
maintain a list. The user configures the binary path.

---

## Environment

When spawning a child shell, the server constructs the environment in
layers (later layers override earlier):

1. Server's own environment (inherited from the spawning process).
2. Static WinMux variables: `WINMUX_VERSION`, `WINMUX_SESSION_ID`,
   `WINMUX_SESSION_NAME`, `WINMUX_WINDOW_INDEX`, `WINMUX_PANE_INDEX`.
3. Session-level overrides from `Session::env_overrides`.
4. Spawn-time overrides from `NewSession.env`.

Then, if env filtering is enabled (off by default), variables matching
the configured patterns are removed.

---

## Persistence

A session's metadata is serialized to JSON on:

- Graceful server shutdown.
- On demand via `Command { tmux: "save-session" }`.
- Periodically (default 5 min) for crash resilience.

The file is at `%APPDATA%\winmux\sessions\<session-id>.json`. It
contains:

- Session name, created_at.
- Windows: index, name, layout, active pane.
- Panes: index, shell ref, cwd at last save, title.
- **Not**: PTY content, scrollback, environment values.

On next server startup, sessions are not auto-restored. The first
connecting client is asked: "Restore N saved sessions?" The user
chooses. See [`08-persistence.md`](08-persistence.md) for the full
flow.

---

## CRUD via IPC

| Operation | Message | Notes |
| --- | --- | --- |
| List sessions | `ListSessions` → `SessionList` | |
| Create session | `NewSession` → `Attached` | |
| Rename session | `Command { tmux: "rename-session" }` | |
| Kill session | `KillSession` → `Ok` | |
| Create window | `NewWindow` → `WindowCreated` | |
| Rename window | `Command { tmux: "rename-window" }` | |
| Select window | `SelectWindow` | |
| Kill window | `KillWindow` | |
| Split pane | `SplitPane` → `PaneCreated` | |
| Select pane | `SelectPane` | |
| Resize pane | `Resize` | |
| Kill pane | `KillPane` | |
| Zoom pane | `Command { tmux: "resize-pane", args: ["-Z"] }` | |
| Swap panes | `Command { tmux: "swap-pane" }` | M2 |

See [`01-ipc-protocol.md`](01-ipc-protocol.md) for the wire format.

---

## Limits

- **Sessions per server:** 64 (soft limit; warned in `ls`).
- **Windows per session:** 32.
- **Panes per window:** 16.
- **Panes per server (total):** 256.

These limits are advisory. The server logs WARN when crossed but does
not reject the operation. They exist so that pathological scripting
(`for i in {1..10000}; do winmux new-session -d`) is noisy.

---

## Related Docs

- ConPTY and VirtualTerm internals →
  [`02-pty-and-terminal.md`](02-pty-and-terminal.md)
- Key handling and prefix → [`04-key-handling.md`](04-key-handling.md)
- Persistence and serialization → [`08-persistence.md`](08-persistence.md)
- Config schema → [`09-config.md`](09-config.md)
