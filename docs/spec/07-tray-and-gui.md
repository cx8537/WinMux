# 07 — Tray and GUI

> `winmux-tray.exe`: the Tauri app. Tray icon, main window, panels.

The tray is the user-facing GUI for WinMux. It is always running when
WinMux is in use, displaying a tray icon that opens the main window
on demand. OneDrive is the design reference: small footprint when
not in use, full UI on click.

---

## Process Layout

- **Backend (Rust):** `winmux-tray.exe`. Tauri host. Handles tray
  icon, window lifecycle, Named Pipe client connection to the
  server. No PTY code.
- **Frontend (React):** `src/`. Renders the terminal panes with
  xterm.js, the session list, settings, and modals. Talks to the
  Rust side via Tauri commands.

---

## Tray Icon

### Visibility

Always shown when the tray process is running. No "show icon when
needed" mode.

### Icon states

- **Normal:** default monochrome icon.
- **Active sessions:** small dot on the icon when there are sessions
  (more than zero).
- **Sensitive session active:** red dot when any session has
  disk-backed scrollback enabled.
- **Server unresponsive:** yellow exclamation overlay if a `Ping`
  hasn't been answered within 30 s. Click to view details.
- **Crashed:** red X overlay if the connection to the server has
  dropped and reconnect attempts have exhausted. Click to restart.

### Left-click

Opens the main window. If already open, foregrounds it.

### Right-click (context menu)

```
WinMux 0.1.0
─────────────
✓ Open main window
─────────────
New session…
Sessions  ▶
  • work
  • build
  • docs
─────────────
Settings…
About WinMux
─────────────
Check for updates
─────────────
Quit WinMux
```

`Sessions ▶` submenu lists current sessions (max 10 shown; "More…"
opens main window).

`Check for updates` runs the optional manual update check (one HTTPS
call to GitHub Releases API).

---

## Main Window

### First-run behavior

- Window is **not** shown automatically.
- A one-time toast appears: "WinMux is running. Click the tray icon
  to open it." Stored in `winmux.toml` (`first_run_toast_shown = true`)
  so it doesn't reappear.
- After ~3 days of use (heuristic: server uptime sum reaches 1 hour),
  a different toast asks: "Add WinMux to startup?" with Yes / Later /
  Don't ask again. The user's answer is persisted.

### Window layout

```
┌────────────────────────────────────────────────────────────┐
│ ☰  WinMux                              ─  □  ✕            │  ← chrome
├──────────────┬─────────────────────────────────────────────┤
│              │  ┌──────────────────────────────────────┐   │
│  Sessions    │  │ work : 0 zsh                          │   │
│              │  │ ┌────────────┬────────────┐          │   │
│  • work *    │  │ │            │            │          │   │
│  • build     │  │ │   pane 0   │   pane 1   │          │   │
│  • docs      │  │ │            │            │          │   │
│              │  │ └────────────┴────────────┘          │   │
│  + New       │  └──────────────────────────────────────┘   │
│              │   ◀ 0 ▶ 1 ▶ 2 │ + new window               │  ← window bar
├──────────────┼─────────────────────────────────────────────┤
│ status: server ok │ prefix: C-b │ utf-8 │ pwsh 7.4 │       │  ← status bar
└────────────────────────────────────────────────────────────┘
```

Components:

- **Sessions panel (left).** Collapsible. Lists sessions; active
  session is highlighted with `*`. Right-click for rename, kill.
- **Main area (right).** Title bar of the active window, the panel
  layout (xterm.js panes), and a window bar at the bottom.
- **Window bar.** Tabs for windows in the active session. Active one
  highlighted. `+` for new window.
- **Status bar.** Driven by `status-format` from `.tmux.conf`, with
  format strings resolved. Default shows server health, prefix key,
  encoding, shell info.

### Window chrome

- Standard Windows 11 chrome (Title bar, minimize, maximize, close).
- "Close" hides the window — server keeps running. A one-time toast
  explains this on first close: "WinMux is still running in the tray.
  Right-click the tray icon and choose Quit to fully exit."
- Optional "Confirm exit when closing window" setting (default off,
  since close = hide).

### Size and position

- Persisted in `winmux.toml`.
- On startup, restored if the screen still has a monitor at the
  saved position; otherwise centered on the primary monitor.
- DPI changes (moving to a different monitor) handled by Tauri.

---

## Panel System (Terminal Area)

The panel area renders the active window's pane layout. Each pane is
an xterm.js instance.

### Pane rendering

- xterm.js v6 with WebGL renderer (canvas fallback if WebGL fails).
- Each pane is a React component that mounts/unmounts an xterm.js
  Terminal in a `ref`. xterm.js owns its canvas; React doesn't
  re-render the canvas tree.
- The Terminal subscribes to `PtyOutput` events for its pane ID and
  writes them.
- The Terminal sends `PtyInput` for keystrokes via the keyboard
  manager (see [`04-key-handling.md`](04-key-handling.md)).

### Layout

The pane layout is a binary tree (see
[`03-session-model.md`](03-session-model.md)). The React component
tree mirrors it:

```tsx
function PaneLayout({ layout }: { layout: PaneLayoutData }) {
  if (layout.kind === 'single') return <PaneView paneId={layout.id} />;
  return (
    <ResizableSplit direction={layout.direction} ratio={layout.ratio}>
      <PaneLayout layout={layout.first} />
      <PaneLayout layout={layout.second} />
    </ResizableSplit>
  );
}
```

`ResizableSplit` is a Tailwind-styled split component (built with
shadcn/ui resize handles). Dragging the divider sends a `Resize`
message; the server updates the layout and broadcasts the new pane
sizes.

### Active pane indicator

The active pane has a subtle border highlight (active border color
from `pane-active-border-style`). Inactive panes have a muted border
(`pane-border-style`).

### Zoom

When a pane is zoomed, the layout component conditionally renders
just that pane in the full area. Other panes are unmounted from the
DOM but their xterm.js Terminal instances are kept in memory in a
hidden ref to avoid losing state.

---

## Sessions Panel

Left sidebar. Lists sessions:

- **Selected session** is highlighted; clicking the main area shows
  its windows.
- Right-click on a session: rename, kill, "open in new window"
  (M3+).
- Bottom: `+ New session` opens the NewSession modal.

The panel can be collapsed (icon-only mode) via a header button.

---

## Modals

shadcn/ui Dialog components. Three primary modals:

### New session

Fields:

- Name (defaults to `untitled-N`).
- Shell (dropdown of detected shells, plus "Custom path…").
- Initial directory (defaults to user home).
- Initial command (optional, sent as `send-keys` after spawn).
- Detached? (don't attach this client to it).
- "Persist scrollback to disk" toggle (with security warning
  on hover).

Pressing Enter or "Create" sends `NewSession`. The modal closes on
success and the new session becomes active.

### Settings

Tabs:

1. **General**
   - UI language (System / English / 한국어).
   - Color theme (Dark only in M1; Dark / Light / System in M2+).
   - Font family and size.
   - Autostart toggle.
2. **Terminal**
   - Default shell.
   - Scrollback line limit (memory).
   - Bracketed paste toggle.
   - Mouse mode toggle.
3. **Keys**
   - Prefix key (sequence picker).
   - `.tmux.conf` path.
   - Reload config button.
   - List bound keys (read-only view).
4. **Security**
   - Disk scrollback default (per new session) — off by default.
   - Environment filtering toggle and patterns.
   - Allow `run-shell` / `if-shell` (Phase C; off by default; warning).
   - Audit log retention days.
5. **Advanced**
   - Log level.
   - Diagnostic export button (zips recent logs + config).
   - About / version info.

### Confirm dialogs

- Killing a session.
- Enabling disk-backed scrollback.
- Enabling Phase C arbitrary command directives.
- Resetting settings to defaults.

---

## i18n

The UI is localized into English and Korean. See
[`10-i18n.md`](10-i18n.md).

System language is detected on first launch. The user can override
in Settings → General.

---

## Themes

### Dark (M1, default)

Inspired by Tokyo Night / Solarized Dark, but tuned. Defined as
Tailwind CSS variables in `src/styles/theme-dark.css`.

ANSI 16 palette: a balanced set with high contrast for readability.

### Light (M2+)

A clean light theme. Same palette inverted where appropriate.

### System (M2+)

Follow Windows app mode (`AppsUseLightTheme` registry value, watched
for change).

### Custom

Not in M1–M4. May be addressed later by accepting `.tmuxrc`-style
color directives.

---

## Notifications and Toasts

Three classes:

1. **In-window toasts.** shadcn/ui Sonner. Auto-dismiss after
   5 seconds, dismissible. Used for: "Build complete" from
   `display-message`, "Config reloaded", "Session created."
2. **Tray balloon notifications.** Windows-native (toast/banner).
   Used for: "Session work has activity", first-run welcome, server
   unresponsive warning.
3. **Modal alerts.** Reserved for actionable errors: "Server
   crashed. Restart?" or "Config file has errors. Show details?"

---

## Status Bar

Bottom of the main window. Driven by `status-format` from
`.tmux.conf`, with `#{...}` placeholders resolved.

Default if no custom format:

```
[server: ok] [prefix: C-b] [utf-8] [pwsh 7.4] [work:0]
```

Right side:

```
[12:34] [docs:2 windows]
```

---

## Multi-Monitor

Tauri reports the monitor for each window. WinMux remembers the
window's last position and:

- On startup: if the saved monitor is connected and the position is
  within its bounds, restore there. Otherwise center on the primary
  monitor.
- On disconnect of the current monitor: move the window to the
  primary monitor's center.
- On DPI change: the WebView re-renders automatically.

Multi-monitor pane spread (panes across screens) is **out of scope**
through M4. The main window is single-monitor.

---

## Performance

The GUI's job is to render xterm.js at 60 fps when scrolling and
process keystrokes within 8 ms (see
[`../nonfunctional/performance.md`](../nonfunctional/performance.md)).
This requires:

- xterm.js's WebGL renderer.
- Avoiding React re-renders for terminal byte updates (xterm.js
  manages its own canvas).
- Bounded React component trees (no per-cell components).
- Zustand stores with narrow selectors so an update to one session
  doesn't re-render every component.

---

## Related Docs

- Key handling and IME details → [`04-key-handling.md`](04-key-handling.md)
- Session/window/pane model → [`03-session-model.md`](03-session-model.md)
- i18n → [`10-i18n.md`](10-i18n.md)
- Accessibility (keyboard, screen reader basics) →
  [`../nonfunctional/accessibility.md`](../nonfunctional/accessibility.md)
