# 09 — Configuration

> The `winmux.toml` schema. Defaults, migration, validation.

WinMux has two configuration files:

1. **`winmux.toml`** at `%APPDATA%\winmux\winmux.toml`. WinMux-specific
   settings: GUI preferences, security toggles, paths.
2. **`winmux.conf`** at `%APPDATA%\winmux\winmux.conf`. tmux-style
   directives: `set`, `bind`, etc. Format documented in
   [`05-tmux-compat.md`](05-tmux-compat.md).

This document describes `winmux.toml`. Both files are user-editable.

---

## Loading Order

1. Server starts. Reads `%APPDATA%\winmux\winmux.toml`.
2. Validates schema version. Migrates if needed.
3. Reads `winmux.conf` (path is also configurable in TOML).
4. tmux-side options from `winmux.conf` override TOML defaults where
   they overlap (only `prefix` and `default_shell` overlap right now).
5. Server starts serving.

If `winmux.toml` is missing, the server creates it with defaults on
first start.

If `winmux.toml` is malformed: server logs ERROR, falls back to
defaults, and shows a tray notification.

---

## Schema (v1)

```toml
# Schema version. Do not edit unless migrating.
schema_version = 1

[general]
# UI language: "system", "en", or "ko".
language = "system"

# Color theme: "dark", "light", or "system".
# M1 supports "dark" only; others fall back.
theme = "dark"

# Autostart with Windows. Mirrored in HKCU\...\Run.
autostart = false


[terminal]
# Default shell. If absent or invalid, autodetect.
# default_shell = "C:\\Program Files\\PowerShell\\7\\pwsh.exe"
# default_shell_args = ["-NoLogo"]

# Default working directory for new sessions.
# default_cwd = "C:\\Users\\you\\projects"

# In-memory scrollback lines per pane.
scrollback_lines = 10000

# Font family and size.
font_family = "Cascadia Code, Consolas, D2Coding, 'Noto Sans Mono CJK KR', monospace"
font_size = 13

# Bracketed paste mode.
bracketed_paste = true

# Confirm pastes that contain newlines.
confirm_multiline_paste = false

# Mouse mode.
mouse = true


[keys]
# Path to the tmux-style config file. Default:
# config_path = "%APPDATA%/winmux/winmux.conf"

# Default prefix if not set in winmux.conf.
prefix = "C-b"

# Repeat window in ms for `bind -r`.
repeat_ms = 500


[security]
# Disk-backed scrollback by default for new sessions. Strongly
# recommended to keep this `false`. Sessions can be opted-in
# individually via the New Session modal.
default_persist_scrollback = false

# Filter env vars matching these glob patterns when spawning shells.
# Off by default.
filter_env_patterns = []

# Allow `run-shell` / `if-shell` in winmux.conf. Off by default.
# Enabling also requires a one-time GUI modal confirmation.
allow_arbitrary_commands = false

# Audit log retention days. Cap: 365.
audit_retention_days = 90


[logging]
# Log level: "error", "warn", "info", "debug", "trace".
# Overridable via WINMUX_LOG environment variable.
level = "info"

# Log directory. Default:
# directory = "%APPDATA%/winmux/logs"


[server]
# How often (seconds) to write session metadata to disk.
session_save_interval_seconds = 300

# How long (seconds) to wait for a child shell to exit on shutdown
# before TerminateProcess.
shutdown_grace_seconds = 5


[gui]
# Last window position and size. Updated automatically.
window_x = 100
window_y = 100
window_width = 1280
window_height = 800

# Sessions panel collapsed?
sessions_panel_collapsed = false

# First-run toast shown? (Internal flag.)
first_run_toast_shown = false

# Autostart prompt shown? (Internal flag.)
autostart_prompt_shown = false


[updates]
# Manual update check from tray menu is always available. This toggle
# enables an additional check on tray startup.
check_on_startup = false


# Per-session defaults. Override via the New Session modal or via
# tmux `set-option`.
[defaults.session]
# When creating a new session without specifying a name.
name_format = "untitled-{n}"
```

---

## Field Reference

### `[general]`

| Field | Type | Default | Range / values |
| --- | --- | --- | --- |
| `language` | string | `"system"` | `"system"`, `"en"`, `"ko"` |
| `theme` | string | `"dark"` | `"dark"`, `"light"`, `"system"` |
| `autostart` | bool | `false` | |

### `[terminal]`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `default_shell` | string (path) | autodetected | Absolute path |
| `default_shell_args` | array of strings | `[]` | |
| `default_cwd` | string (path) | user home | |
| `scrollback_lines` | int | `10000` | 100..1_000_000 |
| `font_family` | string | (CJK chain) | CSS font stack |
| `font_size` | int | `13` | 6..72 |
| `bracketed_paste` | bool | `true` | |
| `confirm_multiline_paste` | bool | `false` | |
| `mouse` | bool | `true` | |

### `[keys]`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `config_path` | string (path) | `%APPDATA%/winmux/winmux.conf` | |
| `prefix` | string | `"C-b"` | tmux key syntax |
| `repeat_ms` | int | `500` | 100..2000 |

### `[security]`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `default_persist_scrollback` | bool | `false` | |
| `filter_env_patterns` | array of strings | `[]` | Glob, case-insensitive |
| `allow_arbitrary_commands` | bool | `false` | Requires GUI confirmation in addition |
| `audit_retention_days` | int | `90` | 1..365 |

### `[logging]`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `level` | string | `"info"` | `"error"`, `"warn"`, `"info"`, `"debug"`, `"trace"` |
| `directory` | string (path) | `%APPDATA%/winmux/logs` | |

### `[server]`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `session_save_interval_seconds` | int | `300` | 60..3600 |
| `shutdown_grace_seconds` | int | `5` | 1..30 |

### `[gui]`

GUI internal state. Edited by the app; manual edits are tolerated but
may be overwritten.

### `[updates]`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `check_on_startup` | bool | `false` | |

### `[defaults.session]`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `name_format` | string | `"untitled-{n}"` | `{n}` is incrementing integer |

---

## Validation

On load:

- Unknown top-level keys: WARN, retained on disk (in case the user
  wants them for a future version).
- Unknown nested keys within known tables: WARN.
- Out-of-range numbers: clamp to range, WARN.
- Invalid enum values: fall back to default, WARN.
- Missing required tables: filled with defaults silently.

Parser uses `toml` crate + `serde` deserialization with custom
validators where needed.

---

## Migration

`schema_version` is bumped on incompatible changes. When the code sees
an old version:

1. Loads the file with a "permissive" deserializer that accepts the
   old layout.
2. Applies a migration function (`migrate_v1_to_v2`, etc.).
3. Writes a backup: `winmux.toml.bak-v1-2026-05-11T09-32-11`.
4. Writes the new file.
5. Logs the migration at INFO.

Migration logic lives in `crates/winmux-server/src/config/migration.rs`,
one function per version bump.

Forward-only. We do not support downgrading.

---

## Environment Variable Overrides

Some settings can be overridden via env vars (useful for testing):

| Env var | Overrides |
| --- | --- |
| `WINMUX_LOG` | `[logging].level` |
| `WINMUX_CONFIG` | path to `winmux.toml` |
| `WINMUX_CONF` | path to `winmux.conf` |
| `WINMUX_NO_AUTOSTART` | forces `autostart = false` for this run |

`WINMUX_LOG` follows the `tracing-subscriber::EnvFilter` syntax
(`info`, `winmux_server=debug,winmux_protocol=warn`, etc.).

---

## Hot Reload

`winmux.toml` is **not** hot-reloaded. Changes take effect on next
server start.

`winmux.conf` is reloaded on:

- `prefix + r` if bound.
- `winmux source-file` from the CLI.
- Settings → Keys → "Reload config" button.

We do not watch the file for changes — too easy to trigger on partial
saves during editing.

---

## Editing

The Settings UI exposes a subset of `winmux.toml` (the common
fields). Advanced fields are edited directly in the file. Settings →
Advanced has an "Open `winmux.toml` in default editor" button.

Manual edits while the server is running are not picked up (no hot
reload).

---

## Related Docs

- Persisted state and session JSONs → [`08-persistence.md`](08-persistence.md)
- `.tmux.conf` directives (winmux.conf) → [`05-tmux-compat.md`](05-tmux-compat.md)
- Logging behavior → [`../nonfunctional/logging.md`](../nonfunctional/logging.md)
- Security toggles → [`../nonfunctional/security.md`](../nonfunctional/security.md)
