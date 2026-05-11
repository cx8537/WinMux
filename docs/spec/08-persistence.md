# 08 — Persistence

> What survives across server restarts, GUI restarts, and reboots —
> and what doesn't.

WinMux supports four levels of "I closed something, will it survive?"
The shorthand from earlier discussions:

- **Case A.** Close the GUI (tray). Sessions survive. ✓
- **Case B.** SSH connection drops. Sessions survive on the host PC. ✓
- **Case C.** Server restart on the same PC (manual or after crash).
  Sessions restored from disk: layouts, names, cwd; **shell content
  is lost** (you'd need a new shell to be spawned). ✓ with caveats.
- **Case D.** PC reboot. Same as Case C, but with one more user
  decision: "Restore N sessions?" ✓ with caveats.

This document is the canonical reference.

---

## What's Persisted

| Data | Storage | Trigger |
| --- | --- | --- |
| Session metadata (name, windows, panes, layout) | `%APPDATA%\winmux\sessions\<id>.json` | Periodic + on shutdown + on save command |
| Pane configuration (shell, cwd, env overrides) | Same JSON | Same |
| Configuration (`winmux.toml`) | `%APPDATA%\winmux\winmux.toml` | On settings change |
| `.tmux.conf` (user-written) | `%APPDATA%\winmux\winmux.conf` | User edits |
| Audit log | `%APPDATA%\winmux\audit.sqlite` | Continuous |
| Logs | `%APPDATA%\winmux\logs\<process>-<date>.log` | Continuous, rotated |
| Disk scrollback (opt-in only) | `%APPDATA%\winmux\scrollback\<id>-<w>-<p>.log` | Continuous when enabled |
| Window position and size (GUI) | `winmux.toml` | On window move/resize |
| Tray UI state (collapsed sessions panel, etc.) | `winmux.toml` | On change |
| Autostart enabled flag | `winmux.toml` AND registry | On toggle |

---

## What's Not Persisted

- **PTY input/output content** (except scrollback, opt-in).
- **In-memory scrollback** when disk mirror is off.
- **Running process state** in any meaningful sense — the shell
  process itself is owned by the OS and lives only as long as the
  server.
- **Environment variable values** beyond what's in
  `Session::env_overrides`.
- **Clipboard buffers.**

---

## Session Serialization Format

```json
{
  "schema_version": 1,
  "session": {
    "id": "ses-01HKJ4Z6PXA7G3M2F9XQ7VWERT",
    "name": "work",
    "created_at": "2026-05-11T09:32:11+09:00",
    "default_shell": {
      "path": "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
      "display_name": "pwsh",
      "args": ["-NoLogo"]
    },
    "default_cwd": "C:\\projects",
    "env_overrides": {
      "EDITOR": "code"
    },
    "persist_scrollback": false,
    "active_window": "win-...",
    "windows": [
      {
        "id": "win-...",
        "index": 0,
        "name": null,
        "active_pane": "pane-...",
        "layout": {
          "kind": "split",
          "direction": "vertical",
          "ratio": 0.5,
          "first": { "kind": "single", "id": "pane-..." },
          "second": { "kind": "single", "id": "pane-..." }
        },
        "panes": [
          {
            "id": "pane-...",
            "shell": { "path": "...", "display_name": "pwsh", "args": [] },
            "cwd": "C:\\projects\\winmux",
            "title": null
          },
          { "...": "..." }
        ]
      }
    ]
  }
}
```

A schema version field lets future versions migrate old files.
Migration is forward-only and writes a `.bak` next to the original
before transforming.

---

## When Is It Saved?

The server writes session JSON in these cases:

1. **On graceful shutdown.** All sessions written. This is the
   primary path.
2. **Periodically.** Every 5 minutes during normal operation. Tunable
   in `winmux.toml` (`session_save_interval_seconds`).
3. **On explicit `save-session` command.**
4. **On significant changes** (new pane created, layout changed,
   shell exited, session renamed) — debounced to at most one write
   per 5 seconds per session, to avoid I/O thrash.

Writes are atomic: write to `<id>.json.tmp`, fsync, rename to
`<id>.json`. Stale `.tmp` files on startup are deleted.

---

## When Is It Restored?

Sessions are **not auto-restored**. On server startup:

1. Server scans `%APPDATA%\winmux\sessions\*.json` for valid files.
2. Server logs the count: "N saved sessions available for restore."
3. Server enters its normal accept loop.
4. **When the first client connects**, the server sends a
   `RestoreOffer { count: N, sessions: [...summary...] }` message in
   the `HelloAck` (or as a follow-up event in M2+).
5. The client presents a modal:

   ```
   Restore your previous sessions?

   You had 3 sessions when WinMux last shut down:
   ▢ work (3 windows, last active 2026-05-11 09:45)
   ▢ build (1 window, last active 2026-05-11 09:32)
   ▢ docs (2 windows, last active 2026-05-10 18:10)

   [ Restore selected ]  [ Restore all ]  [ Skip ]
   ```

6. The user chooses. Choices are sent as `RestoreSessions { ids:
   [...] }`. The server spawns fresh shells for each pane in the
   selected sessions, using the persisted shell ref, cwd, and env
   overrides.
7. Unselected sessions remain on disk for a configurable retention
   period (default 7 days), then are cleaned up. The user can
   recover via the Settings → Advanced → "Recover deleted sessions"
   panel (M3).

This flow is the answer for Case D (PC reboot).

---

## What Gets Lost Across a Restart

Cases C and D both involve respawning shells. Things that don't
survive:

- **Running processes inside the pane.** If you had vim editing a
  buffer, vim's state is gone. The shell is fresh.
- **Untitled buffers in editors.** Unless the editor has its own
  recovery files (vim's swap, VS Code's hot exit), they're lost.
- **Background jobs.** A `cargo build &` running in the previous
  session is gone.
- **Shell history.** Whatever your shell persisted to `.bash_history`
  or PSReadLine history is intact; whatever was only in memory is
  not.
- **Scrollback.** Unless disk-backed scrollback was enabled.

This is documented in the user-facing modal:

> Restoring will recreate your panes' shells and working directories.
> Any commands you were running, editor buffers, or scrollback that
> wasn't saved to disk will be lost. Continue?

---

## Cleanup

### On session kill

- Session JSON deleted from `sessions/`.
- Disk scrollback (if any) deleted from `scrollback/`.
- Audit log retains the events (per retention policy).

### Periodic

- Old session JSONs (older than 7 days) are deleted on server
  startup.
- Log files older than 30 days are deleted on log rotation.
- Disk scrollback files for nonexistent sessions are cleaned up on
  startup.

### On uninstall

The installer does not delete `%APPDATA%\winmux\`. The user must
delete it manually to remove all data. This is documented in the
installer and the README.

---

## SQLite (Audit Log)

- File: `%APPDATA%\winmux\audit.sqlite`.
- WAL mode, normal synchronous.
- Indexed on timestamp and event type.
- Schema migrations via embedded SQL in `crates/winmux-server/src/audit.rs`.
- Retention: configurable (default 90 days, max 365). Cleanup runs
  at startup and once per day.

Schema (M1):

```sql
CREATE TABLE events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           TEXT NOT NULL,            -- ISO 8601 UTC
    event_type   TEXT NOT NULL,            -- e.g. 'session_created'
    session_id   TEXT,
    window_id    TEXT,
    pane_id      TEXT,
    payload      TEXT                       -- JSON, never contains content
);

CREATE INDEX idx_events_ts ON events(ts);
CREATE INDEX idx_events_type ON events(event_type);
```

`payload` is structured per `event_type` and validated on insert.

---

## Configuration File Migration

`winmux.toml` carries a `schema_version` field. On startup, if the
file's version is older than the current code:

1. Load the file.
2. Apply each version migration in sequence.
3. Write a backup at `winmux.toml.bak-v<old>-<timestamp>`.
4. Write the new file.
5. Log the migration at INFO.

If migration fails, the original file is left intact and a warning is
shown in the GUI: "Settings could not be migrated; using defaults.
Open the file in Settings → Advanced → 'Edit raw config' to recover."

---

## Disk Space

WinMux is conservative with disk usage:

- Logs: ≤ 2 GiB total before forced rotation.
- Audit log: typically small (KBs/day for normal use), bounded by
  retention.
- Disk scrollback: per-pane cap (default 100 MiB), per-session
  total cap (default 1 GiB), with rotation.
- Session JSONs: KBs each.

On low disk space (free space < 100 MiB on the `%APPDATA%` volume),
the server stops persisting new state (logs, scrollback, sessions)
and surfaces a notification. Running shells are not affected.

---

## Recovery Procedures

If session JSONs become corrupt or unparseable, the server:

1. Logs ERROR with the file path and the parse error.
2. Moves the file to `sessions/.broken/<id>-<ts>.json`.
3. Continues normally.

The user can recover by editing the file manually or accepting the
loss.

---

## Related Docs

- Session/window/pane model → [`03-session-model.md`](03-session-model.md)
- Configuration file format → [`09-config.md`](09-config.md)
- Security implications of persistence →
  [`../nonfunctional/security.md`](../nonfunctional/security.md)
