# 02 — PTY and Virtual Terminal

> How WinMux talks to Windows shells, and how it remembers what they
> drew.

The server is the only process that touches PTYs. Tray and CLI receive
already-cooked terminal bytes via IPC.

---

## The Stack

```
Child shell (pwsh.exe / cmd.exe / ...)
        ▲    │
   stdin│    │stdout/stderr
        │    ▼
     ConPTY  (Windows pseudo-console)
        ▲    │
        │    │
        │    ▼
   portable-pty   (Rust abstraction over ConPTY)
        ▲    │
        │    │
        │    ▼
   pty::Pty      (WinMux's wrapper: lifetime, Job Object, signals)
        ▲    │
        │    │ raw bytes
        │    ▼
   terminal::VirtualTerm   (wraps alacritty_terminal)
        ▲    │
        │    │ parsed grid state + emitted bytes
        │    ▼
   scrollback::Scrollback  (memory + optional disk)
        │
        ▼
   IPC broadcast (PtyOutput frames)
```

Reading: each pane has one `pty::Pty`, one `terminal::VirtualTerm`,
one `scrollback::Scrollback`. They live and die together inside a
`pane::Pane` aggregate.

---

## ConPTY via `portable-pty`

We use `portable-pty` (`PtyPair`, `PtySize`, `CommandBuilder`) so
WinMux is not coupled to win32 ConPTY APIs directly. This is also why
nothing in our code calls `CreatePseudoConsole` itself.

### Spawn

```rust
let pty_system = native_pty_system();
let pair = pty_system.openpty(PtySize {
    rows, cols, pixel_width: 0, pixel_height: 0,
})?;
let mut cmd = CommandBuilder::new(&shell_path);
cmd.cwd(cwd);
for (k, v) in env { cmd.env(k, v); }
let child = pair.slave.spawn_command(cmd)?;
// drop pair.slave — we no longer need the slave handle in the parent
drop(pair.slave);
```

The `pair.master` is the read/write side. We hold:

- A `Box<dyn MasterPty>` for resize.
- A reader (`Box<dyn Read + Send>`) for stdout/stderr stream.
- A writer (`Box<dyn Write + Send>`) for stdin.
- The `Box<dyn Child + Send + Sync>` for `wait` and `kill`.

### Reading

The reader is run on a `spawn_blocking` task because `MasterPty::read`
is blocking. It reads into a fixed buffer (default 64 KiB) and
forwards bytes to the virtual terminal via a bounded channel.

### Writing

Writes happen from the IPC dispatcher when `PtyInput` arrives. They
go through a `Mutex<Box<dyn Write + Send>>` because multiple async
tasks may share the writer.

### Resize

```rust
master.resize(PtySize { rows, cols, .. })?;
virtual_term.resize(rows, cols);
```

Both must succeed or the pane is marked degraded and the user is
notified.

### Child lifetime

- The child shell is wrapped in a Windows Job Object (see below) so
  that server termination guarantees descendant cleanup.
- `Child::wait` runs on a `spawn_blocking` task. When it returns, the
  pane emits a `PaneExited` event with the exit code.

---

## Job Object

Every child shell is added to a Job Object with:

- `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: when the job handle closes,
  every process in the job dies.
- Per-process Job per pane, so killing one pane doesn't take down
  others.
- Optional `ProcessMemoryLimit` (later; off in M0–M2).

The Job handle is owned by the `pty::Pty` struct. Drop the `Pty`,
drop the Job, kill the shell — even if the child has spawned
grandchildren that ignore CTRL_BREAK.

---

## Virtual Terminal (`alacritty_terminal`)

Raw replay of the PTY byte stream is **forbidden** (see `decisions.md`
D-8). Programs that clear the screen would replay every byte ever
written, including escapes that move the cursor backwards through
prior content. The result is a flashing mess.

Instead, the server runs the PTY bytes through `alacritty_terminal`'s
parser and keeps an in-memory grid. When a client attaches or
reattaches, the server **serializes** the current grid state into a
single block of escape sequences (cursor positions, colors, glyphs)
and sends that as `initial_snapshots` in the `Attached` message.

### `VirtualTerm` API

```rust
pub struct VirtualTerm { /* ... */ }

impl VirtualTerm {
    pub fn new(rows: u16, cols: u16) -> Self;

    /// Feed PTY output bytes through the parser, updating grid state.
    pub fn feed(&mut self, bytes: &[u8]);

    pub fn resize(&mut self, rows: u16, cols: u16);

    /// Serialize the current grid to escape sequences that, when
    /// written to a fresh xterm-compatible terminal, reproduce the
    /// visible screen.
    pub fn snapshot(&self) -> Vec<u8>;

    /// Title last set via OSC 0/2.
    pub fn title(&self) -> Option<&str>;

    /// Active styling state (cursor pos, colors, modes). Internal.
    fn state(&self) -> &TermState;
}
```

`snapshot()` is the operation that makes reattach correct. It emits:

1. `ESC[2J` to clear, `ESC[H` to move to top-left.
2. For each cell with content: SGR (color, attributes) escapes + the
   cell glyph. Adjacent cells with same style are merged.
3. Cursor position restore.
4. Active modes (alt screen state, application keypad, …).

Edge cases:

- **Alt screen.** If the virtual terminal is currently in alt screen
  (`?1049h`), the snapshot includes the switch and emits alt-screen
  content. Reverting to main screen on detach is the alt-screen
  program's job, not the snapshotter's.
- **Wide characters.** Emit the CJK character once; do not emit a
  placeholder for the following cell.
- **Reverse video, italics, underline:** preserved.
- **256-color and truecolor:** preserved.

### Why not just `vt100`?

`vt100` is lighter but less complete on modern escapes (truecolor,
mouse modes, OSC variants). `alacritty_terminal` has years of
production use behind Alacritty.

### Wrapping vs depending directly

We depend on `alacritty_terminal` only via `crates/winmux-server/src/terminal.rs`.
The rest of the server uses our `VirtualTerm` type. This lets us swap
implementations later (or pin to a forked version) with a one-file
change.

---

## Scrollback

Each pane has a bounded scrollback buffer.

### Memory

- Default capacity: 10,000 lines.
- Configurable per session via `winmux.toml` (`scrollback_lines`).
- Stored as a ring buffer of `Line` objects (vector of styled cells).
- Eviction is FIFO.

### Disk (opt-in)

When `persist_scrollback` is enabled for a session:

- Append-only file at
  `%APPDATA%\winmux\scrollback\<session-id>-<window>-<pane>.log`.
- One line per row, terminated by `\n`. Color/style metadata stored
  as ANSI escapes (so the file is human-readable in a normal pager).
- ACL: current user only.
- Rotation: size cap (default 100 MiB per pane) and time cap
  (default 7 days). Whichever hits first.
- Optional masking: regex patterns from settings applied before
  write.
- See `security.md` for the warning modal flow.

### Copy mode

The user enters copy mode via the prefix key. The client requests
`EnterCopyMode { pane_id }`. The server responds with a copy-mode
view: the current screen plus N lines of scrollback (default: the
full scrollback). The client renders this in xterm.js's normal
scrollback area, and key handling enters vi or emacs movement mode.

When the user presses `Enter` to yank a selection, the client copies
to OS clipboard directly. The server is not involved beyond providing
the scrollback content.

---

## Sequence: New Session

```
Client                      Server                  ConPTY      Child
  │                           │                       │           │
  │ NewSession{...}           │                       │           │
  ├──────────────────────────►│                       │           │
  │                           │ openpty(rows,cols)    │           │
  │                           ├──────────────────────►│           │
  │                           │                       │           │
  │                           │ spawn_command(pwsh)   │           │
  │                           ├──────────────────────►│ spawn ───►│
  │                           │                       │           │
  │                           │ JobObject.assign      │           │
  │                           │                       │           │
  │                           │ VirtualTerm::new      │           │
  │                           │                       │           │
  │                           │                       │ stdout    │
  │                           │ feed(bytes)           ◄───────────┤
  │                           ◄───────────────────────┤           │
  │                           │                       │           │
  │                           │ snapshot()            │           │
  │ Attached{snapshot,...}    │                       │           │
  ◄───────────────────────────┤                       │           │
  │                           │                       │           │
  │ PtyOutput frames...       │                       │           │
  ◄═══════════════════════════│ (broadcast loop)      │           │
```

---

## Sequence: Reattach

```
Client                      Server                  (PTY running)
  │                           │
  │ Hello → HelloAck          │
  │                           │
  │ Attach{session, size}     │
  ├──────────────────────────►│
  │                           │
  │                           │ if size != current: Resize
  │                           │
  │                           │ for each pane:
  │                           │   bytes = vterm.snapshot()
  │                           │
  │ Attached{initial_snapshots}
  ◄───────────────────────────┤
  │                           │
  │ continue broadcast        │
  ◄═══════════════════════════│
```

---

## Failure Modes

| Scenario | Detection | Action |
| --- | --- | --- |
| Shell process exits | `Child::wait` returns | Emit `PaneExited`. Mark pane as dead. Keep scrollback. |
| `MasterPty::read` returns EOF | reader loop | Same as above. |
| ConPTY resize fails | `master.resize` returns `Err` | Log error. Surface to user. Pane marked degraded. |
| Virtual terminal feed exceeds resource | `alacritty_terminal` rejects | Log. Drop the chunk. Continue. (Should not happen with bounded inputs.) |
| Scrollback disk write fails | I/O error | Disable disk mirror for this pane. Surface notification. Memory scrollback continues. |
| Job Object handle leak | Drop guard | Caught via Drop impl. `cfg(debug_assertions)` panics; release logs WARN. |

---

## Known Issues

See [`../known-issues.md`](../known-issues.md) — sections CP, PP, AT
cover ConPTY, `portable-pty`, and `alacritty_terminal` quirks.
