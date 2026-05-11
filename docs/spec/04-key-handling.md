# 04 — Key Handling

> The prefix state machine, key tables, IME and CJK input, and the
> Ctrl+C copy-or-interrupt dance.

Key handling lives entirely in the **tray** (frontend). xterm.js
receives keystrokes through a custom key event handler; the prefix
state machine decides whether to swallow the key or pass it through.

The server never sees prefix keys. It sees the resulting actions
(IPC messages) or raw PTY input bytes.

---

## State Machine

```
                   ┌──────────────────────────────────┐
                   │                                  │
                   │                                  ▼
            ┌──────────┐  prefix             ┌─────────────────┐
            │  Idle    │ ───────────────────►│ AwaitingCommand │
            └──────────┘  (e.g., Ctrl+B)     └─────────────────┘
                ▲                                  │
                │                                  │ command key
                │                                  ▼
                │                          ┌─────────────────┐
                │                          │  Executing      │
                │                          │  (IPC dispatch) │
                │                          └─────────────────┘
                │                                  │
                │  resolved (action / unknown)     │
                └──────────────────────────────────┘
```

States:

- **Idle.** Default. All keys pass through to xterm.js (which writes
  them to the PTY via `PtyInput`).
- **AwaitingCommand.** The prefix has been pressed. The next keypress
  is interpreted as a tmux command. The UI shows a subtle indicator
  ("prefix active"). A 3-second timeout returns to Idle.
- **Executing.** Briefly, while the IPC request flies. Almost
  invisible.

### Prefix

Default: `Ctrl+B`. Configurable via `.tmux.conf` (`set -g prefix
C-x`) or `winmux.toml`.

To send a literal prefix to the shell: prefix + prefix
(`Ctrl+B Ctrl+B`).

### Repeat (`-r`)

Some bindings (resize, navigate) support repeat:

```
bind -r H resize-pane -L 5
```

After the first `prefix + H`, subsequent `H` presses (without
prefix) within the repeat window (default 500 ms) re-execute the
same binding. Any other key returns to Idle.

---

## Key Tables

A key table is a named set of bindings.

| Table | When active |
| --- | --- |
| `root` | Idle state. `bind -n` adds here. |
| `prefix` | AwaitingCommand state. `bind` (without `-T`) adds here. |
| `copy-mode-vi` | Copy mode, vi keys. |
| `copy-mode` | Copy mode, emacs keys. |
| Custom (`bind -T mytable ...`) | Activated by `switch-client -T mytable`. |

The active table is consulted on every keypress. If a binding
matches, its action runs. If not (in `prefix`), the prefix is
canceled and the key is dropped. If not (in `root`), the key passes
through to xterm.js → PTY.

---

## Custom xterm.js Key Handler

xterm.js exposes `attachCustomKeyEventHandler`. We provide a function
that:

1. Composes the key into a normalized form (modifiers + key name).
2. Looks up in the active table.
3. If matched, dispatches and returns `false` (xterm.js does not
   process this key).
4. If not matched and in `prefix` state, returns `false` (silently
   eaten).
5. If not matched and in `root` state, returns `true` (xterm.js
   handles normally).

```typescript
terminal.attachCustomKeyEventHandler((event) => {
  return keyboardManager.handle(event);
});
```

The manager (`src/lib/keyboard.ts`) owns the state machine and the
key tables. It also tracks the prefix-repeat timer.

---

## Default Bindings (Phase A)

In the `prefix` table (after pressing `Ctrl+B`):

| Key | Action |
| --- | --- |
| `c` | new window |
| `,` | rename window |
| `n` | next window |
| `p` | previous window |
| `0..9` | select window 0..9 |
| `&` | kill window |
| `%` | split pane vertically (left/right) |
| `"` | split pane horizontally (top/bottom) |
| `x` | kill pane |
| `z` | zoom pane toggle |
| `arrows` | select pane in direction |
| `o` | rotate panes |
| `space` | next preset layout (M2) |
| `d` | detach |
| `[` | enter copy mode (M2) |
| `]` | paste buffer (M2) |
| `:` | command prompt (M2) |
| `?` | list bindings |
| `Ctrl+B` | send literal prefix to shell |

User overrides apply via `.tmux.conf`.

---

## Ctrl+C: Copy or Interrupt

The Sidabari/Windows convention:

- If there is a non-empty text selection in the active pane → `Ctrl+C`
  copies the selection to the OS clipboard, clears the selection,
  and **does not** send to the shell.
- If no selection → `Ctrl+C` sends `CTRL_C_EVENT` to the shell's
  process group.

Implementation:

- xterm.js has a `getSelection()` method. We check it on `Ctrl+C`
  before deciding.
- The clipboard write uses Tauri's clipboard API (avoids xterm.js
  going through `navigator.clipboard.writeText`, which has CSP
  caveats in Tauri).
- The "selection or not" check happens in the keyboard manager, not
  in xterm.js's default handler.

`Ctrl+Shift+C` is also bound to "copy if selected, otherwise nothing"
for users who prefer the explicit shortcut.

### Paste

- `Ctrl+V` → reads the OS clipboard, sends as bracketed paste
  (`\x1b[200~...\x1b[201~`) to the shell.
- `Ctrl+Shift+V` → same as `Ctrl+V`.
- Multi-line paste warning is **off** by default. A setting toggles
  it.

---

## IME (Korean / Japanese / Chinese)

The tray's WebView supports IME composition. Korean input via the
default Windows Korean IME composes Hangul jamo into syllables before
emitting characters.

Requirements:

- The prefix key (`Ctrl+B` and variants) must **bypass** the IME.
  Pressing `Ctrl+B` mid-composition cancels the composition and
  enters AwaitingCommand state.
- xterm.js's WebGL renderer handles wide characters (CJK) correctly
  for display; we have to make sure the input side is right.

Implementation details:

- We listen for `compositionstart`, `compositionupdate`,
  `compositionend` events on the xterm.js textarea.
- During composition, no `PtyInput` is sent for the in-progress
  composition (it would echo garbage).
- On `compositionend`, the composed text is sent as a single
  `PtyInput` payload.
- The prefix detection is at the `keydown` level, before composition,
  so prefix interception works.

### CJK font fallback chain

xterm.js requires a fallback chain for CJK rendering:

```
'Cascadia Code', 'Consolas', 'D2Coding', 'Noto Sans Mono CJK KR', monospace
```

`Cascadia Code` is the primary font on Windows 11 and ships by
default. Korean glyphs fall through to D2Coding (commonly installed
in Korea) or Noto Sans Mono CJK KR.

Users can override the font in settings. WinMux does not bundle
fonts.

---

## `.tmux.conf` Integration

Bindings can be defined in `.tmux.conf`:

```tmux
unbind C-b
set -g prefix C-x

bind r source-file ~/.config/winmux/winmux.conf \; display "config reloaded"
bind | split-window -h
bind - split-window -v
bind -n M-Left select-pane -L
```

The server parses `.tmux.conf` and sends the resulting binding table
to the tray on attach. The tray rebuilds its `KeyboardManager` from
the table.

Reload-on-edit is **manual** (`prefix + r` if bound, or "Reload
config" tray menu item). We do not watch the file.

See [`05-tmux-compat.md`](05-tmux-compat.md) for which directives are
supported per phase.

---

## OS-Reserved Keys

Some keys are intercepted by Windows or the shell before WinMux sees
them. We do not try to override OS behavior:

| Key | Owner |
| --- | --- |
| `Alt+F4` | Windows (close window) |
| `Win` + anything | Windows |
| `Alt+Tab`, `Alt+Esc` | Windows |
| `Ctrl+Alt+Del` | Windows |
| `Print Screen` | Windows |

If a user binds one of these in `.tmux.conf`, the parser issues a
warning at load time.

---

## Conflict Resolution

When a key matches multiple bindings (impossible in well-formed
configs, but possible during reload races):

- Most recent binding wins (`bind` overrides previous `bind`).
- `bind -n` (root table) loses to `bind` (prefix table) — they're in
  different tables, so they don't conflict in practice.

When a key in `bind -n` (root) shadows a normal shell input:

- We do not warn. tmux doesn't either. The user knows what they did.
- One exception: if a user binds a printable ASCII character in the
  root table, the parser warns: "This binding will prevent typing
  this character. Sure?"

---

## Debugging

Verbose mode logs every prefix transition and every binding
resolution at TRACE level. Off by default. Enable in dev:

```
RUST_LOG=winmux_tray=trace npm run tauri dev
```

---

## Related Docs

- tmux directives we support → [`05-tmux-compat.md`](05-tmux-compat.md)
- Config schema → [`09-config.md`](09-config.md)
- i18n + IME (cross-references CJK input) → [`10-i18n.md`](10-i18n.md)
