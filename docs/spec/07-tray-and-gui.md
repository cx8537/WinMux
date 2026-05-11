# 07 — Tray and GUI

> `winmux-tray.exe`: the Tauri app. Tray icon, main window, panels.
> Color tokens and themes.

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
  (more than zero). Uses `--status-info` token.
- **Sensitive session active:** dot uses `--status-error` token when
  any session has disk-backed scrollback enabled.
- **Server unresponsive:** small overlay using `--status-warn` token
  if a `Ping` hasn't been answered within 90 s.
- **Crashed:** overlay using `--status-error` token if reconnect
  attempts have exhausted. Click to restart.

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
  session is highlighted with `--bg-active`. Right-click for rename,
  kill.
- **Main area (right).** Title bar of the active window, the panel
  layout (xterm.js panes), and a window bar at the bottom.
- **Window bar.** Tabs for windows in the active session. Active one
  highlighted. `+` for new window.
- **Status bar.** Driven by `status-format` from `.tmux.conf`, with
  format strings resolved. Default shows server health, prefix key,
  encoding, shell info. Background uses `--bg-secondary`.

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

The active pane has a subtle border highlight using
`--pane-border-active`. Inactive panes use `--pane-border`.

### Zoom

When a pane is zoomed, the layout component conditionally renders
just that pane in the full area. Other panes are unmounted from the
DOM but their xterm.js Terminal instances are kept in memory in a
hidden ref to avoid losing state.

---

## Sessions Panel

Left sidebar. Lists sessions:

- **Selected session** is highlighted with `--bg-active`; clicking
  the main area shows its windows.
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
   - Terminal palette (Campbell / One Dark / Solarized Dark /
     Tokyo Night / Custom — M2+).
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

WinMux has **two independent theme surfaces**, and they intentionally
use different palettes:

| Surface | Palette source | Reasoning |
| --- | --- | --- |
| UI chrome (tray, window, sidebar, modals, status bar) | **IntelliJ Darcula**-derived | Optimized for hours-long reading; developer-familiar; not pure black |
| Terminal interior (xterm.js content) | **Windows Terminal "Campbell"** ANSI 16 | Familiar to Windows users; `vim`/`ls`/`git` look the way users expect |
| Status signal colors (overlays, indicators) | **Quiet-by-default** | Normal state is silent; only abnormal states use color |

Mixing these surfaces into one palette would compromise both. They
are governed by separate CSS variable namespaces.

### Why these choices

- **IntelliJ Darcula for the chrome.** Among the three candidates we
  considered (IntelliJ Darcula, Sidabari's dashboard aesthetic, raw
  tmux/terminal colors), Darcula is the one designed for long-form
  *reading* of code. Backgrounds are not pure black (`#1E1F22` rather
  than `#000000`), foreground is not pure white (`#BCBEC4` rather
  than `#FFFFFF`), and contrast is dialed for legibility without
  glare. WinMux's main window is something users will keep open all
  day; the chrome must not be visually loud.

- **Campbell for the terminal.** It is the default Windows Terminal
  palette, ships with Cascadia Code, and matches the colors Windows
  developers already know. A `vim`, `ls --color`, or `git status` in
  a WinMux pane should look exactly like the same command in
  Windows Terminal. Users can pick another preset (One Dark, etc.)
  in M2+.

- **Quiet status signaling.** Sidabari's dashboard look is
  attention-grabbing by design — it is *meant* to be looked at for
  five minutes to triage an incident. WinMux is the opposite use
  case: ambient, all-day. We borrow only one principle from
  Sidabari: *normal is silent, abnormal is colored.* No green "OK"
  indicators. No traffic lights when nothing is wrong.

---

## Dark Theme (M1 Default)

### UI chrome tokens

Defined as Tailwind CSS variables in `src/styles/theme-dark.css`:

```css
:root[data-theme="dark"] {
  /* Surfaces */
  --bg-primary:        #1E1F22;  /* main content area */
  --bg-secondary:      #2B2D30;  /* sidebar, status bar, panels */
  --bg-tertiary:       #3C3F41;  /* hover, input fields, code blocks */
  --bg-active:         #2E436E;  /* selected items (IntelliJ blue) */
  --bg-overlay:        rgba(30, 31, 34, 0.85);  /* modal backdrop */

  /* Borders */
  --border-subtle:     #393B40;  /* between panels */
  --border-strong:     #4E5157;  /* input fields, dividers */
  --border-focus:      #4A88C7;  /* focus ring */

  /* Text */
  --text-primary:      #BCBEC4;  /* body text */
  --text-secondary:    #868A91;  /* muted, labels, captions */
  --text-disabled:     #54585E;  /* disabled controls */
  --text-on-accent:    #FFFFFF;  /* text on accent-colored buttons */
  --text-link:         #4A88C7;
  --text-link-hover:   #5394D6;

  /* Accent (interactive elements) */
  --accent:            #4A88C7;  /* IntelliJ-style blue */
  --accent-hover:      #5394D6;
  --accent-active:     #3F77B0;

  /* Pane borders */
  --pane-border:           #393B40;
  --pane-border-active:    #4A88C7;

  /* Status signals — quiet by default */
  --status-ok:         #54A150;  /* used sparingly */
  --status-info:       #4A88C7;
  --status-warn:       #C9A227;
  --status-error:      #C75450;  /* IntelliJ error tone */
}
```

### Terminal palette (Campbell)

Passed to xterm.js `terminal.options.theme`:

```typescript
export const campbellPalette = {
  background:     '#0C0C0C',
  foreground:     '#CCCCCC',
  cursor:         '#FFFFFF',
  cursorAccent:   '#0C0C0C',
  selectionBackground: 'rgba(255, 255, 255, 0.25)',

  black:          '#0C0C0C',
  red:            '#C50F1F',
  green:          '#13A10E',
  yellow:         '#C19C00',
  blue:           '#0037DA',
  magenta:        '#881798',
  cyan:           '#3A96DD',
  white:          '#CCCCCC',

  brightBlack:    '#767676',
  brightRed:      '#E74856',
  brightGreen:    '#16C60C',
  brightYellow:   '#F9F1A5',
  brightBlue:    '#3B78FF',
  brightMagenta:  '#B4009E',
  brightCyan:     '#61D6D6',
  brightWhite:    '#F2F2F2',
} as const;
```

The terminal interior background (`#0C0C0C`) is intentionally darker
than the surrounding UI chrome (`#1E1F22`). This visually separates
the "content" (what your shell wrote) from the "frame" (what WinMux
drew), and matches the convention in code editors that have a
distinct editor background.

### Where tokens are used

| Component | Token |
| --- | --- |
| Main window background | `--bg-primary` |
| Sessions sidebar background | `--bg-secondary` |
| Status bar background | `--bg-secondary` |
| Window bar (tabs) background | `--bg-secondary` |
| Modal background | `--bg-secondary` |
| Modal backdrop | `--bg-overlay` |
| Button (default) | `--bg-tertiary` background, `--text-primary` text |
| Button (primary action) | `--accent` background, `--text-on-accent` text |
| Button (hover) | `--accent-hover` |
| Input field | `--bg-tertiary` background, `--border-strong` border |
| Input focus ring | `--border-focus` |
| Selected session in sidebar | `--bg-active` background |
| Pane border (inactive) | `--pane-border` |
| Pane border (active) | `--pane-border-active` |
| Status bar text (normal) | `--text-secondary` |
| Status bar indicator (sensitive session) | `--status-error` |
| Status bar indicator (server slow) | `--status-warn` |
| Toast (info) | `--bg-tertiary` background, `--text-primary` text |
| Toast (error) | `--status-error` accent stripe |

Hard-coded hex values are forbidden in components — they bypass the
theme system and break high-contrast mode. Lint catches this with a
custom rule (M2+).

### Light theme (M2+)

A clean IntelliJ Light-derived theme. Same token names, different
values:

```css
:root[data-theme="light"] {
  --bg-primary:        #FFFFFF;
  --bg-secondary:      #F7F8FA;
  --bg-tertiary:       #EBECF0;
  --bg-active:         #D4E3F7;
  /* ... */
  --text-primary:      #1F1F1F;  /* not pure black */
  --text-secondary:    #6C707E;
  --accent:            #3574F0;
  /* ... */
}
```

Light terminal palette (planned, M2+): a light variant of Campbell
or "GitHub Light." TBD; the M2 release will lock in defaults.

### System theme (M2+)

Follows Windows app mode (`AppsUseLightTheme` registry value, watched
for change via the Tauri side). Switches between the dark and light
themes above.

### High contrast (Windows accessibility)

When Windows is in High Contrast mode, the WebView reports it via
`prefers-contrast: more` and `forced-colors: active`. The theme
yields to system colors:

- UI chrome uses `CanvasText` / `Canvas` / `ButtonFace` system
  tokens.
- Terminal background and foreground use system tokens.
- Pane border active uses `Highlight`.

See [`../nonfunctional/accessibility.md`](../nonfunctional/accessibility.md)
for the full contrast story.

---

## Terminal Palette Presets (M2+)

Users can pick a different terminal palette in Settings → General.
Built-in presets:

| Preset | Default? | Note |
| --- | --- | --- |
| Campbell | ✓ (Dark) | Windows Terminal default |
| One Dark | | Atom / VS Code "One Dark Pro" derived |
| Solarized Dark | | Classic |
| Tokyo Night | | Popular among Neovim users |
| GitHub Light | (default if Light theme picked) | M2+ |
| Custom | | User pastes ANSI 16 hex values into Settings |

Switching presets only changes `terminal.options.theme`. The UI
chrome stays on its IntelliJ-derived palette regardless. This is
intentional: users who pick "Solarized Dark" for their shell don't
want their sidebar repainted to match.

`.tmux.conf` color directives (`pane-border-style fg=colour238`,
status-bar colors) are honored for elements drawn *by the server's
status format* — they remain in the ANSI palette. WinMux's own
sidebar / modal / chrome are not affected by `.tmux.conf` colors.

---

## Notifications and Toasts

Three classes:

1. **In-window toasts.** shadcn/ui Sonner. Auto-dismiss after
   5 seconds, dismissible. Used for: "Build complete" from
   `display-message`, "Config reloaded", "Session created."
   - Info: `--bg-tertiary` background.
   - Warn: `--status-warn` accent stripe.
   - Error: `--status-error` accent stripe.
2. **Tray balloon notifications.** Windows-native (toast/banner).
   Used for: "Session work has activity", first-run welcome, server
   unresponsive warning.
3. **Modal alerts.** Reserved for actionable errors: "Server
   crashed. Restart?" or "Config file has errors. Show details?"

---

## Status Bar

Bottom of the main window. Driven by `status-format` from
`.tmux.conf`, with `#{...}` placeholders resolved.

Background: `--bg-secondary`. Text: `--text-secondary` by default;
fragments may explicitly use one of the `--status-*` tokens.

Default if no custom format:

```
[server: ok] [prefix: C-b] [utf-8] [pwsh 7.4] [work:0]
```

Right side:

```
[12:34] [docs:2 windows]
```

Indicators on the status bar:

- **Sensitive session active.** Small red dot
  (`--status-error`) next to the session name if any session has
  disk-backed scrollback enabled.
- **Server slow.** Yellow dot (`--status-warn`) if ping round-trip
  exceeds 2 s for three consecutive pings.
- **Otherwise: silent.** No green "ok" lamp.

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
- Accessibility (keyboard, screen reader basics, high-contrast) →
  [`../nonfunctional/accessibility.md`](../nonfunctional/accessibility.md)
- Configuration tokens user-overridable → [`09-config.md`](09-config.md)
