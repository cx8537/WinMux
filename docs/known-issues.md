# Known Issues and Limitations

A living document. When the development team (Claude Code + cx8537)
discovers a quirk, gotcha, or boundary in a dependency or the Windows
platform, it goes here. The goal is to avoid re-discovering the same
problem twice.

Entries are grouped by source. Within each group, entries are dated.

---

## ConPTY

### CP-1. ConPTY intercepts certain escape sequences (Windows-version dependent)

Some terminal escape sequences are handled inside ConPTY itself rather
than being forwarded as raw bytes. The set differs across Windows
versions (1809, 1903, 21H2, Windows 11). This can cause subtle
discrepancies between what the child shell thinks it wrote and what
`alacritty_terminal` receives.

**Workaround.** Always update the virtual terminal state from the
bytes we read from the ConPTY output handle, not by trying to predict
what the shell sent. Trust the byte stream as the source of truth.

**Status.** Mitigated by design. Investigate per-case if behavior
diverges.

---

### CP-2. HPCON handle ownership

The HPCON returned by `CreatePseudoConsole` belongs to the process
that called it. If that process dies, the child shell dies too, even
if other processes hold the input/output pipe handles.

**Implication.** `winmux-server.exe` must own the HPCON. The tray and
CLI must not be in this position.

**Status.** Enforced by the three-process architecture (D-1, D-7 in
[`decisions.md`](decisions.md)).

---

### CP-3. Resize requires `ResizePseudoConsole`

Resizing a ConPTY does not happen by changing the underlying pipe. You
must call `ResizePseudoConsole(HPCON, COORD)`. After this, you also
have to inform the virtual terminal of the new size.

**Implication.** A `RESIZE` IPC message triggers two updates: the
ConPTY and the `alacritty_terminal` instance. Both must succeed or
the pane state will diverge.

---

## `portable-pty`

### PP-1. Crate is pre-1.0

`portable-pty` is on version `0.x`. API breakage between minor
versions is possible. Pin the minor version in `Cargo.toml`. Audit the
changelog before bumping.

**Status.** Pin to `0.8` (or whichever the project currently uses).
Track the project for any signs of dormancy.

---

### PP-2. Windows-only features behind feature flags

Some features (`win32-named-pipe`, ConPTY tuning) are gated behind
crate features. Explicitly enable what we need; do not rely on
defaults.

---

## `alacritty_terminal`

### AT-1. Internal API, not a stable public surface

`alacritty_terminal` is published as a crate but is primarily
developed for use inside Alacritty. The team has been clear that the
API may evolve. Treat it as we would an unstable dependency: pin
version, wrap it in our own `crates/winmux-server/src/terminal.rs`
abstraction so swap-out is a one-file change.

---

### AT-2. Not every terminal feature is supported

Sixel graphics, Kitty image protocol, full xterm modifyOtherKeys, and
some OSC sequences are not handled or are handled differently than
xterm. Out of scope for WinMux's first releases; document if a user
report mentions one of these.

---

### AT-3. tmux passthrough (`\x1bPtmux;...`)

A real tmux session can wrap escape sequences in `\x1bPtmux;`
passthrough so the outer terminal can interpret them. WinMux does not
implement passthrough. If your shell is running inside tmux inside
WinMux (please don't), things will break in interesting ways.

---

### AT-4. M0 snapshot emits glyphs only (no SGR)

`VirtualTerm::snapshot` in M0 PoC re-emitted cell glyphs and final cursor
position but discarded per-cell SGR state (foreground/background, bold,
underline, etc.). Reattach therefore showed the same characters in the
same positions but in default colors, until the next live PTY output
re-painted with full attributes.

**Rationale.** SGR diff serialization is a meaningful chunk of code and
was not on the M0 acceptance path (`docs/spec/00-overview.md` only
requires "detach and reattach show the previous screen"). Color-faithful
snapshot was tracked for M1 alongside scrollback persistence.

**Workaround.** None needed for M0. Users who needed exact-color
reattach could wait briefly for the shell to redraw.

**Status.** Resolved in M1.5. `VirtualTerm::snapshot` now walks the grid
and emits SGR transitions (named 16 colors, indexed 256, truecolor RGB;
bold, dim, italic, underline, reverse, strikeout) only at style
boundaries, then restores the cursor. Verified by two unit tests
(`snapshot_round_trips_sgr_through_fresh_vterm`,
`snapshot_round_trips_indexed_and_truecolor`) that feed the snapshot
bytes back into a fresh `VirtualTerm` and assert per-cell fg/bg/attrs
equality.

---

## Tauri 2.x

### TA-1. Tray APIs are newer than the rest of Tauri

Tauri's tray icon API stabilized later than the main app shell. Some
combinations of tray menu and main-window behavior require small
workarounds. Test the tray flow on a clean install before each
release.

---

### TA-2. Single-instance plugin scope

The `single-instance` plugin enforces single instance on the GUI
process (`winmux-tray.exe`). Server single-instance is handled
separately with a Named Mutex in `winmux-server.exe`. Do not assume
one mechanism covers both.

---

## Windows

### WIN-1. PSReadLine binds `Ctrl+B`

PowerShell's PSReadLine binds `Ctrl+B` to backward-char. WinMux uses
`Ctrl+B` as the default prefix, which the GUI intercepts before
PSReadLine sees it. This is the whole point of an intercepting prefix,
but a user who changes their prefix to something PSReadLine doesn't
use should expect their old `Ctrl+B` muscle memory to surprise them.

---

### WIN-2. `Ctrl+Z` is not SIGTSTP on Windows

Windows has no `SIGTSTP`. The closest signal you can deliver to a
console process group is `CTRL_BREAK_EVENT` or `CTRL_C_EVENT`. WinMux
sends these for `Ctrl+C` and `Ctrl+Break` respectively; all other
modifier keys are passed through as escape sequences for the shell to
interpret.

---

### WIN-3. UTF-8 is not the default console code page everywhere

Older PowerShell defaults and some `cmd` configurations still emit
CP949 (or other legacy code pages). WinMux assumes UTF-8 output and
displays invalid bytes as the replacement character (`U+FFFD`).

**Workaround for users.** Set `[Console]::OutputEncoding =
[System.Text.Encoding]::UTF8` in PowerShell, or use PowerShell 7
which defaults to UTF-8.

---

### WIN-4. Display scaling and DPI changes

If a user moves the WinMux window between monitors with different DPI
scaling, the rendered terminal can be briefly blurry or mis-sized.
Tauri handles most of this, but xterm.js renderer reconfiguration may
lag a frame.

---

## Filesystem

### FS-1. `%APPDATA%` can be a roaming profile

In some enterprise environments, `%APPDATA%` is synced to a network
share. SQLite files (audit log) and large scrollback files do not
behave well over network shares.

**Status.** No action planned. Document, and recommend users keep
disk-backed scrollback off in roaming environments.

---

## Network

WinMux does not perform network operations. The only exception is the
optional manual update check, which contacts the GitHub Releases API.

If a user reports network activity from WinMux outside of this case,
treat it as a bug.

---

## How to Add an Entry

```markdown
### <SOURCE>-<N>. <short title>

<2–4 paragraph description>

**Workaround.** ...

**Status.** Mitigated / Active / Won't fix / Investigating.
```

Increment the number within the source. Date-stamp commit messages if
the entry depends on a specific dependency version.
