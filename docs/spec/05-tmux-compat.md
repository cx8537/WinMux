# 05 — tmux Compatibility Matrix

> What tmux features WinMux supports, by phase. The authoritative
> answer to "does WinMux do X?"

WinMux aims for the **core tmux experience**, not byte-for-byte
compatibility. This doc is honest about what's in and what's out.

Symbols:

- ✅ Supported
- 🟡 Partial / planned phase
- ❌ Not supported (with reason)
- 🔒 Supported but requires explicit opt-in (security)

Phase = milestone at which it becomes available. See
[`00-overview.md`](00-overview.md) for milestone definitions.

---

## Sessions

| Feature | Phase | Notes |
| --- | --- | --- |
| `new-session` | M1 | `-s name`, `-d` (detached), `-c cwd`, `-- shell args` |
| `kill-session` | M1 | `-t target` |
| `rename-session` | M1 | |
| `attach-session` | M1 | `-t target`, `-d` (detach others) |
| `detach-client` | M1 | |
| `list-sessions` / `ls` | M1 | |
| `switch-client` | M2 | |
| `has-session` | M2 | Exit-code based |
| `lock-session` | ❌ | tmux lock is rarely used; skipped |
| Session groups | ❌ | Too complex for marginal value |

---

## Windows

| Feature | Phase | Notes |
| --- | --- | --- |
| `new-window` / `neww` | M1 | `-t target`, `-n name`, `-c cwd` |
| `kill-window` | M1 | |
| `rename-window` | M1 | |
| `select-window` / `selectw` | M1 | `-t target`, `-n` (next), `-p` (previous) |
| `next-window` / `previous-window` | M1 | |
| `list-windows` | M1 | |
| `move-window` | M2 | Reorder within session |
| `link-window` | ❌ | Cross-session window links; rarely used |
| `swap-window` | M2 | |
| `renumber-windows` | M2 | |
| Activity / silence monitoring | M2 (activity), M4 (silence) | |
| Bell monitoring | M2 | OSC + audible flag |

---

## Panes

| Feature | Phase | Notes |
| --- | --- | --- |
| `split-window` | M1 | `-h` (horizontal split = left/right), `-v` (vertical split = top/bottom), `-p N` (percentage) |
| `kill-pane` | M1 | |
| `select-pane` | M1 | `-L/-R/-U/-D` directional |
| `resize-pane` | M1 | `-L/-R/-U/-D N`, `-Z` (zoom toggle) |
| `swap-pane` | M2 | |
| `rotate-window` | M2 | `prefix + o` |
| `break-pane` | M2 | Promote pane to its own window |
| `join-pane` | M2 | Inverse of break-pane |
| `select-layout` (presets) | M2 | even-h, even-v, main-h, main-v, tiled |
| `display-panes` | M2 | Numeric pane picker |
| `pipe-pane` | M4 | Redirect pane output to a command |
| Pane synchronization (`synchronize-panes`) | M4 | Send input to all panes |

---

## Copy Mode

| Feature | Phase | Notes |
| --- | --- | --- |
| Enter copy mode (`prefix + [`) | M2 | |
| vi keybindings | M2 | hjkl, w/b, $/^, /?, n/N, v, y |
| emacs keybindings | M2 | Ctrl+f/b, Ctrl+a/e, Ctrl+s, M-w |
| Copy to clipboard | M2 | OS clipboard via Tauri |
| Paste (`prefix + ]`) | M2 | Bracketed paste |
| Buffer list (`prefix + #`) | M3 | tmux's named buffers |
| Rectangular selection | M3 | |
| Search in scrollback | M2 | |

WinMux clipboard integration uses the OS clipboard as the source of
truth. The tmux internal buffer system is supported (M3) but most
users won't need it.

---

## Configuration

### `.tmux.conf` directives

#### Phase A (M1)

| Directive | Phase | Notes |
| --- | --- | --- |
| `set` / `set-option` | A | `-g` (global), `-s` (server) |
| `setw` / `set-window-option` | A | |
| `bind` / `bind-key` | A | `-r` (repeat) |
| `unbind` / `unbind-key` | A | |
| `display-message` | A | Status bar message |
| `source-file` | A | Recursion depth 5 |
| Comments (`#`) | A | |

#### Phase B (M2)

| Directive | Phase | Notes |
| --- | --- | --- |
| `bind -n` | B | Root table binding |
| `bind -T table` | B | Custom tables |
| Command chaining (`;` and `\;`) | B | |
| `display-message -p` | B | Print to stdout for CLI |
| Variable references (`#{...}` in some contexts) | B | Subset |

#### Phase C (M4, opt-in only)

| Directive | Phase | Notes |
| --- | --- | --- |
| `run-shell` | C 🔒 | Runs an external command; security risk |
| `if-shell` | C 🔒 | Conditional based on shell command exit |
| `hook` | C 🔒 | Event-triggered commands |

Phase C directives require `allow_arbitrary_commands = true` in
`winmux.toml` **and** a one-time GUI modal confirmation. See
[`../nonfunctional/security.md`](../nonfunctional/security.md).

### Common settings

| Setting | Phase | Default | Notes |
| --- | --- | --- | --- |
| `prefix` | A | `C-b` | |
| `default-shell` | A | (autodetect) | See [`03-session-model.md`](03-session-model.md) |
| `default-terminal` | A | `xterm-256color` | We emulate xterm |
| `history-limit` | A | `10000` | Lines per pane |
| `escape-time` | A | `0` | We don't need wait-for-escape on Windows |
| `mouse` | A | `on` | Mouse handling |
| `status` | B | `on` | Status bar shown |
| `status-position` | B | `bottom` | `top` or `bottom` |
| `status-format` | B | (default) | Limited `#{...}` support |
| `pane-border-style` | B | | |
| `pane-active-border-style` | B | | |
| `window-status-format` | B | (default) | |
| `renumber-windows` | B | `off` | |
| `base-index` | A | `0` | tmux defaults `0`; we match |
| `pane-base-index` | A | `0` | |
| `remain-on-exit` | A | `on` | Default differs from tmux (which is `off`) |
| `clock-mode-style` | ❌ | | We don't have a clock mode in M1–M4 |
| `automatic-rename` | A | `on` | Sets window name from pane title |

### `set-environment`

`set-environment -g VAR value` is supported in Phase A for additions
to the session's environment overrides. Removal (`-r`) too.

### `set-hook`

Phase C only. See above.

---

## Commands

Common tmux commands that WinMux supports as IPC `Command` messages
and via the CLI. ✓ = supported.

| Command | M1 | M2 | M3 | M4 |
| --- | :-: | :-: | :-: | :-: |
| `new-session` | ✓ | | | |
| `kill-session` | ✓ | | | |
| `rename-session` | ✓ | | | |
| `attach-session` | ✓ | | | |
| `detach-client` | ✓ | | | |
| `list-sessions` | ✓ | | | |
| `switch-client` | | ✓ | | |
| `has-session` | | ✓ | | |
| `new-window` | ✓ | | | |
| `kill-window` | ✓ | | | |
| `rename-window` | ✓ | | | |
| `select-window` | ✓ | | | |
| `next-window` / `previous-window` | ✓ | | | |
| `list-windows` | ✓ | | | |
| `move-window` | | ✓ | | |
| `swap-window` | | ✓ | | |
| `split-window` | ✓ | | | |
| `kill-pane` | ✓ | | | |
| `select-pane` | ✓ | | | |
| `resize-pane` | ✓ | | | |
| `swap-pane` | | ✓ | | |
| `break-pane` | | ✓ | | |
| `join-pane` | | ✓ | | |
| `rotate-window` | | ✓ | | |
| `display-panes` | | ✓ | | |
| `select-layout` | | ✓ | | |
| `send-keys` | ✓ | | | |
| `copy-mode` | | ✓ | | |
| `paste-buffer` | | ✓ | | |
| `list-buffers` | | | ✓ | |
| `set-buffer` | | | ✓ | |
| `save-buffer` | | | ✓ | |
| `delete-buffer` | | | ✓ | |
| `command-prompt` | | ✓ | | |
| `display-message` | ✓ | | | |
| `bind-key` / `unbind-key` | ✓ | | | |
| `list-keys` | ✓ | | | |
| `source-file` | ✓ | | | |
| `set-option` / `set-window-option` | ✓ | | | |
| `show-options` | ✓ | | | |
| `if-shell` / `run-shell` | | | | ✓ 🔒 |
| `set-hook` / `show-hooks` | | | | ✓ 🔒 |
| `pipe-pane` | | | | ✓ |
| `synchronize-panes` | | | | ✓ |
| `clock-mode` | ❌ | | | |
| `lock-session` / `lock-client` | ❌ | | | |
| `confirm-before` | ❌ | | | tmux's prompt; we have GUI modals |
| `wait-for` | ❌ | | | Multi-session signaling; out of scope |
| Plugin system | TBD | | | TBD |

---

## Format Strings

tmux's `#{...}` format strings are extensive. WinMux supports a
subset, growing per phase.

### Phase B (M2) subset

- `#{session_name}`
- `#{session_id}`
- `#{window_index}`
- `#{window_name}`
- `#{window_flags}`
- `#{pane_index}`
- `#{pane_id}`
- `#{pane_title}`
- `#{pane_current_path}`
- `#{pane_current_command}`
- `#{host}` (machine hostname)
- `#{user}` (username)
- Literal `#[fg=color,bg=color,attr]...#[default]`

### Not supported

- Conditionals (`#{?cond,then,else}`)
- Loops (`#{S:...}`, `#{W:...}`)
- Complex expressions

If a format string uses an unsupported feature, the parser substitutes
the literal `#{...}` text and logs a warning.

---

## Plugins

tmux has a vibrant plugin ecosystem (tpm, tmux-resurrect,
tmux-continuum, tmux-yank, tmux-prefix-highlight, …). WinMux does
**not** support tpm-based plugins because most of them are shell
scripts that assume a POSIX environment.

For M4, we may design a plugin interface specifically for WinMux. No
commitment yet.

---

## Out of Scope

These tmux features will not be implemented:

- Lock mode (`lock-session`, `lock-client`).
- Network-attach: tmux uses Unix sockets; WinMux uses Named Pipes
  with strict per-user scope. Remote attach over the network is a
  separate product.
- Background `wait-for`/`run-shell` coordination across sessions.
- Configuration directives that depend on tmux's exact internal
  state model (e.g., `if -F` on internal flags).

---

## Compatibility Stance

When in doubt:

- **Match tmux's behavior** if it's well-defined and useful on
  Windows.
- **Don't match** if tmux's behavior depends on Unix semantics that
  don't translate (signals beyond SIGINT/SIGQUIT, fork/exec idioms,
  Unix socket details, ttys).
- **Don't invent** Windows-only conventions when tmux already has an
  equivalent. (Exception: Ctrl+C copy-when-selected, which is the
  Windows convention.)

For users coming from tmux: most of your muscle memory works. The
common `.tmux.conf` patterns work. Edge cases may not. Open an issue
if something you depend on is missing — the matrix above is the plan,
not the final word.
