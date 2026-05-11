# 06 — CLI

> `winmux.exe`: the single-shot command-line client.

The CLI is scriptable from PowerShell, `cmd`, or any shell. It does
one thing per invocation, prints to stdout/stderr, and exits.

The CLI never holds long-lived state. It connects to
`\\.\pipe\winmux-{user}`, sends a request, prints the response,
disconnects.

---

## Invocation

```
winmux <COMMAND> [ARGS...]
```

Global flags (before the command):

| Flag | Meaning |
| --- | --- |
| `-V`, `--version` | Print version and exit |
| `-h`, `--help` | Top-level help |
| `--quiet`, `-q` | Suppress non-essential output |
| `--json` | Output in JSON (for scripting) |
| `--no-color` | Disable ANSI colors (auto on non-TTY) |

---

## Commands

### `ls` — list sessions

```
winmux ls [--all]
```

Output (default):

```
work    3 windows  (attached)
build   1 window
docs    2 windows
```

With `--json`:

```json
[
  {"id": "ses-...", "name": "work", "windows": 3, "attached": true},
  {"id": "ses-...", "name": "build", "windows": 1, "attached": false},
  {"id": "ses-...", "name": "docs", "windows": 2, "attached": false}
]
```

Exit codes:

- `0` — at least one session listed
- `2` — server not running (and not auto-startable for read-only)

### `new-session` — create a session

```
winmux new-session [-s NAME] [-d] [-c DIR] [-- SHELL [SHELL_ARGS]]
```

| Flag | Meaning |
| --- | --- |
| `-s NAME`, `--session NAME` | Session name (generated if omitted) |
| `-d`, `--detached` | Do not attach this client |
| `-c DIR`, `--cwd DIR` | Initial working directory |
| `--shell PATH` | Shell to spawn (defaults to config) |
| `--` SHELL | Inline shell + args |

Examples:

```
winmux new-session -s work
winmux new-session -s build -d
winmux new-session -s wsl -- wsl.exe -d Ubuntu
winmux new-session -d -c C:\projects\sidabari -- pwsh -NoLogo
```

### `attach` — attach to a session

```
winmux attach [-t TARGET] [-d]
```

`attach` opens the tray's main window with the target session
selected. If the tray isn't running, it is started.

The CLI version of `attach` is for one-off use from a terminal — it
forwards I/O to the parent terminal until the user detaches. This is
M3-and-later behavior; in M1/M2 `attach` from CLI just opens the tray
window.

| Flag | Meaning |
| --- | --- |
| `-t TARGET`, `--target` | Target session (`name` or `ses-...`) |
| `-d`, `--detach-others` | Detach other clients on this session |

### `detach` — detach current client

```
winmux detach
```

Detaches the CLI's session attachment. Only meaningful in M3+ CLI
attach mode.

### `kill-session` — kill a session

```
winmux kill-session -t TARGET
```

Kills every pane in every window of the session, then removes the
session.

### `kill-window` — kill a window

```
winmux kill-window -t TARGET
```

`TARGET` is `session:window` (e.g., `work:1`).

### `kill-pane` — kill a pane

```
winmux kill-pane -t TARGET
```

`TARGET` is `session:window.pane` (e.g., `work:0.1`).

### `send-keys` — send keystrokes

```
winmux send-keys [-t TARGET] KEYS...
```

`KEYS` are tmux-style key names: literal text, or special names like
`Enter`, `Tab`, `Up`, `Down`, `C-c`, `M-x`. Multiple arguments are
sent in order.

```
winmux send-keys -t work:0.0 "echo hello" Enter
winmux send-keys -t build:0.0 "make test" Enter
```

### `list-windows` — list windows in a session

```
winmux list-windows [-t TARGET]
```

```
0: zsh* (1 panes)
1: vim   (1 panes)
2: htop  (2 panes)
```

`*` marks the active window.

### `list-panes` — list panes in a window

```
winmux list-panes [-t TARGET]
```

```
0: pwsh   (40x120) active
1: pwsh   (40x60)
2: vim    (40x60)
```

### `split-window` — split a pane

```
winmux split-window [-t TARGET] [-h|-v] [-p PERCENT] [-c DIR]
```

| Flag | Meaning |
| --- | --- |
| `-h` | Horizontal split (left/right; same as tmux) |
| `-v` | Vertical split (top/bottom; same as tmux; default) |
| `-p N` | Percentage size of the new pane |
| `-c DIR` | Initial cwd |

### `select-pane` — focus a pane

```
winmux select-pane -t TARGET
winmux select-pane -L     # current window, pane to the left
```

### `resize-pane` — resize a pane

```
winmux resize-pane [-t TARGET] -L|-R|-U|-D N
winmux resize-pane [-t TARGET] -Z     # zoom toggle
```

### `capture-pane` — read pane content

```
winmux capture-pane [-t TARGET] [-p] [-S START] [-E END]
```

| Flag | Meaning |
| --- | --- |
| `-p`, `--print` | Print to stdout |
| `-S START` | Start line (negative = relative to top of scrollback) |
| `-E END` | End line (negative = relative to bottom) |

Useful for grepping pane output from PowerShell:

```powershell
winmux capture-pane -t work:0.0 -p | Select-String "ERROR"
```

### `display-message` — print a message to a client

```
winmux display-message [-t TARGET] "Build finished"
```

Sends a transient message that the tray shows as a toast / status bar
flash.

### `source-file` — reload a config file

```
winmux source-file PATH
```

### `show-options` — print effective options

```
winmux show-options [-g]
```

### `bind-key` / `unbind-key`

```
winmux bind-key KEY COMMAND...
winmux unbind-key KEY
```

### `command-prompt` (M2)

```
winmux command-prompt "rename-window %%"
```

### `kill-server` — shut down the server

```
winmux kill-server
```

Sends `Shutdown` to the server. The server serializes sessions and
exits. Equivalent to "Quit WinMux" in the tray.

### `start-server` — start the server explicitly

```
winmux start-server
```

Useful for testing. Most commands auto-start the server if needed.

### `version`

```
winmux version
```

Prints CLI version, server version (if running), and protocol
version.

```
winmux 0.1.0
server 0.1.0 (running, pid 12345)
protocol v1
```

---

## Target Syntax

WinMux's CLI accepts tmux-style targets:

| Form | Means |
| --- | --- |
| `name` | The session named `name` |
| `name:` | (Same; trailing `:` allowed) |
| `name:0` | Window 0 of session `name` |
| `name:0.1` | Pane 1 of window 0 of session `name` |
| `name:windowname` | Window with that display name |
| `name:.1` | (Default window of `name`, pane 1) |
| `ses-01H...` | By session ID |
| `pane-01H...` | By pane ID directly |

If no target is given, the command applies to the "current" target —
the session the calling client is attached to (or the most recently
attached one for a fresh CLI invocation).

If no session is determinable, the command errors.

---

## Exit Codes

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | General error (with message on stderr) |
| `2` | Server not running and could not start |
| `3` | Target not found (session, window, or pane) |
| `4` | Protocol mismatch (incompatible server version) |
| `5` | Permission denied (SID mismatch — should not occur in normal use) |
| `64` | Usage error (invalid arguments; matches `EX_USAGE`) |

---

## Output Format

### Default (human)

Tab-aligned, colorized when stdout is a TTY. ANSI colors disabled
when:

- Output is redirected (not a TTY).
- `--no-color` is passed.
- `NO_COLOR` environment variable is set.
- `CLICOLOR=0` is set.

### `--json`

Single JSON value to stdout. No prefix, no suffix. For scripting:

```powershell
$sessions = winmux ls --json | ConvertFrom-Json
$sessions | Where-Object { $_.attached }
```

When `--json` is set and an error occurs, the error is also emitted
as JSON to **stdout**:

```json
{"error": {"code": "SESSION_NOT_FOUND", "message": "no such session: work"}}
```

The exit code reflects the error.

---

## Server Auto-Start

The CLI auto-starts the server for commands that need it:

- Always-auto-start: `new-session`, `attach`, `send-keys`, anything
  that modifies state.
- Read-only commands (`ls`, `list-windows`, `list-panes`,
  `show-options`, `version`): error if the server isn't running, do
  not start it.

Auto-start procedure:

1. Try to connect to the pipe. If `ERROR_FILE_NOT_FOUND`, no server.
2. Spawn `winmux-server.exe` from the same install directory with
   `DETACHED_PROCESS | CREATE_NO_WINDOW`.
3. Poll the pipe with backoff (100 ms, 300 ms, 1 s, 3 s). Give up at
   3 s and report "server failed to start."
4. On success, proceed with the original command.

---

## Help

`winmux --help` prints top-level help. `winmux COMMAND --help` prints
command-specific help with examples. All help text is in English.

---

## PowerShell Tips

Some patterns that work well with `winmux`:

```powershell
# Run a build in the background and watch from your editor session
winmux new-session -s build -d -c C:\projects\winmux -- pwsh -NoLogo
winmux send-keys -t build:0.0 "cargo build --release" Enter

# Tail the build's pane from a script
while ($true) {
    winmux capture-pane -t build:0.0 -p | Select-Object -Last 5
    Start-Sleep 2
}

# Kill all sessions matching a pattern
winmux ls --json | ConvertFrom-Json | Where-Object {
    $_.name -like "scratch-*"
} | ForEach-Object { winmux kill-session -t $_.name }
```

---

## Related Docs

- IPC protocol → [`01-ipc-protocol.md`](01-ipc-protocol.md)
- Tray and GUI counterparts → [`07-tray-and-gui.md`](07-tray-and-gui.md)
- tmux command compatibility matrix → [`05-tmux-compat.md`](05-tmux-compat.md)
