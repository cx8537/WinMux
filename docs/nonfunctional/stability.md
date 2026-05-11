# Stability

> Resource management, panic handling, shutdown sequencing, watchdog
> behavior, retry policy.

The server is a long-running background process that owns child
shells. A leak, a panic, or a stuck thread means the user's work is
in danger. Stability is the most important non-functional quality of
WinMux.

---

## Resources

### Handles

Every Windows handle (`HANDLE`, `HPCON`, `HJOB`, file handles, pipe
handles) is owned by a Rust struct with a `Drop` impl that releases
it. We do not pass raw handles around without an owning wrapper.

Patterns:

- `pty::Pty` — owns HPCON, child handles, master pipe handles. Drop
  sends `CTRL_BREAK_EVENT` to the process group, waits up to 5 s,
  then `TerminateProcess`.
- `job::JobHandle` — owns HJOB; Drop closes the job (which kills
  members due to `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
- `ipc::Connection` — owns the named pipe handle and per-client
  state; Drop disconnects.
- `audit::AuditDb` — owns the SQLite connection; Drop flushes WAL.

`Drop` implementations must not block forever. If a wait could exceed
a reasonable bound, the Drop sets a timer and uses `TerminateProcess`
or equivalent after the timeout.

### Files

`std::fs::File`, `tokio::fs::File`, `tempfile::NamedTempFile` —
RAII. No `unsafe` file handle juggling.

### Channels

Bounded `tokio::sync::mpsc` only. Channel close on sender drop is
how we signal "no more work."

### Tasks

Every spawned task is tracked: a `JoinSet`, a `JoinHandle` held by a
struct, or a documented fire-and-forget with a clear rationale.

Fire-and-forget is rare. The default is "if it fails, log it" plus
a structured shutdown path.

---

## Panics

### Release builds: `panic = "abort"`

```toml
[profile.release]
panic = "abort"
```

A panic terminates the process. We do **not** use `catch_unwind` to
recover. The reasoning:

- Recovery from a panic invariably leaves the state space in a
  questionable position. The Rust borrow checker doesn't help here.
- A clean abort + restart is simpler and safer than a maybe-corrupted
  continuation.
- The Job Object ensures descendants die when we do.

### Panic hook

In each process's `main`:

```rust
std::panic::set_hook(Box::new(|info| {
    // Write a crash log including panic location, message, and
    // backtrace. Path: %APPDATA%\winmux\logs\crash-<process>-<ts>.log
    write_crash_log(info);
    // Then let the default behavior happen (abort).
}));
```

The hook:

- Does not allocate excessively.
- Writes to a fixed-size buffer and `WriteFile` directly to the log
  file (no `tracing` calls, which may be in the panic path).
- Includes panic message, file/line, and a backtrace if available.
- Includes process name, pid, version.

Logged content does **not** include PTY input/output, environment
values, or other sensitive data.

### Debug builds

`panic = "unwind"` in debug. Tests can use `should_panic`. The Drop
impls still run for cleanup during unwind.

---

## Graceful Shutdown

### Server

Triggered by:

- `Shutdown` IPC message (from tray "Quit WinMux" or CLI
  `kill-server`).
- `WM_QUERYENDSESSION` / `WM_ENDSESSION` (Windows shutdown).
- `Ctrl+C` in a console-attached server (dev only).
- Receipt of a child-controlled signal (rare).

Sequence:

1. Set the global "shutting down" flag (`AtomicBool`).
2. Stop accepting new clients on the pipe.
3. Send `ServerBye` to every attached client. Wait briefly (300 ms)
   for clients to acknowledge.
4. For each session:
   - Snapshot the virtual terminal state.
   - Serialize to `%APPDATA%\winmux\sessions\<id>.json`.
5. For each pane:
   - Send `CTRL_BREAK_EVENT` to the child shell's process group.
   - Wait up to 5 s for the child to exit.
   - If still alive, `TerminateProcess`.
6. Flush audit log. Flush trace logs. Close all files.
7. Release the Named Mutex.
8. Drop the pipe.
9. Return from `main`.

Total time budget: 10 s. After 10 s, the watchdog forcibly aborts
(see below).

### Tray

Triggered by "Quit WinMux" or system shutdown.

1. Send `Bye` to the server. (The server has its own shutdown path;
   the tray's `Bye` is just a courtesy.)
2. Save GUI state to `winmux.toml` (window position, etc.).
3. Tear down the WebView.
4. Exit.

If the user closes the main window (X button), the tray stays alive
with the icon. Quitting is explicit.

### CLI

Always short-lived. Each invocation:

1. Connect.
2. Send request, await response (5 s default timeout).
3. Print result.
4. Disconnect.
5. Exit.

No shutdown protocol needed. Drop everything in `main` via Rust
normal scope exit.

---

## Watchdog

The tray pings the server every 30 s. If three consecutive pings
(over ~90 s) fail to receive a `Pong`:

1. Tray icon overlay turns yellow (warning).
2. Tray menu surfaces "Server unresponsive…"
3. No automatic restart. The user decides.

If the user clicks "Restart server":

1. Tray sends one last `Shutdown` (best-effort).
2. Waits 2 s.
3. Spawns a new `winmux-server.exe`.
4. Reconnects.

The server has no internal watchdog over its own threads — we rely on
panic-and-abort, and on the bounded channels to fail loudly when a
consumer is stuck.

---

## Auto-retry Policy

The Sidabari rule, restated:

- **User-initiated commands never auto-retry.** A failed
  `NewSession` returns an error to the user. The user decides what
  to do.
- **Infrastructure operations may retry with bounded backoff.**
  Specifically:
  - Client → server pipe connect: 100 ms, 300 ms, 1 s, 3 s, give up.
  - Server startup retry from a tray/CLI auto-start: same backoff.
  - Audit DB write retry on `SQLITE_BUSY`: up to 5 times with
    20–100 ms backoff (SQLite WAL is rarely contended).

Anything beyond these is a bug.

---

## Watchdog Timeouts

Where the server enforces explicit timeouts:

| Operation | Timeout | Action on miss |
| --- | --- | --- |
| Client `Hello` after pipe accept | 5 s | Disconnect with `PROTOCOL_VIOLATION` |
| IPC request handler | 5 s default | Return `TIMEOUT` error to caller |
| Child shell wait after `CTRL_BREAK` | 5 s | `TerminateProcess` |
| Full server shutdown | 10 s | Abort process (watchdog) |
| Snapshot generation | 5 s | Return error to attaching client; pane marked degraded |
| `.tmux.conf` parse | 5 s | Reject the config; surface error to user |

The full-server-shutdown watchdog is implemented as a separate
thread spawned at the start of the shutdown sequence: it sleeps
for 10 s, then calls `std::process::abort()` if the main shutdown
sequence has not yet returned.

---

## Backpressure

The server enforces bounded queues at every IPC boundary:

- Per-client outgoing message queue.
- PTY reader → pane manager channel.
- Pane manager → IPC broadcast channel.

When a queue fills:

- **Control-plane messages:** sender blocks briefly (1 s). If still
  full, return error. This is rare; control plane is low-volume.
- **Data-plane (PtyOutput):** drop the oldest frames for the slow
  client and log a WARN. After dropping too many (16 MiB cumulative),
  disconnect the client with a clear error.

Other clients on the same session continue unaffected.

---

## Deadlocks

We avoid the conditions that produce them:

- **Lock ordering:** if a code path takes more than one lock, the
  order is documented at the call site. Lock order is enforced by
  code review and clippy where possible.
- **No async lock + await held:** held `tokio::sync::Mutex` may not
  be carried across `.await` unless the awaited future cannot block
  on the same lock. (Clippy's `await_holding_lock` is enabled.)
- **No channel-of-channels patterns** that can produce mutual waits.

If a deadlock is suspected:

1. Enable `RUST_LOG=trace` and `tokio-console` (in dev).
2. Capture a thread dump (`procdump` or VS).
3. Find the cycle. Fix the lock order or the design.

---

## Resource Leaks

Detection:

- Drop guards in debug: a `Pty::drop` that finds the child still
  alive after the timeout panics (in `cfg(debug_assertions)`).
- Tests verify that processes have exited and handles have closed.
- Manual: run for a day, check `Get-Process` and `Get-Counter
  Process.HandleCount`. Numbers should be flat.

If a leak slips through, the Job Object ensures descendants die when
the server itself does — which limits the blast radius but doesn't
excuse the bug.

---

## Process Restart Semantics

If the server crashes (panic + abort):

1. The tray detects via failed pings.
2. The tray surfaces the crash notice.
3. The user clicks "Restart server."
4. New server starts. Reads `sessions/*.json`. Offers restore to the
   first connecting client.
5. PTY shell child processes are gone (Job Object killed them when
   the server died). Sessions restored with fresh shells.

This is acceptable because crashes should be rare; if they're not,
the project has bigger problems than nicer recovery.

If the tray crashes:

1. The server keeps running.
2. The user opens the tray manually (Start menu or executable).
3. The new tray connects to the existing server. Attach to a session
   and continue. No work is lost.

This is the case we optimize for.

---

## Test Coverage

See [`testing.md`](testing.md). The relevant must-pass scenarios:

- Persistence across tray restart (Scenario 2).
- Slow client isolation (Scenario 7).
- Graceful shutdown leaves no zombies, no temp files, no handle
  leaks (Scenario 10).
- Crash recovery: kill `-9 ` the server, verify Job Object kills
  children, verify subsequent restart restores sessions.

---

## Related Docs

- IPC backpressure details → [`../spec/01-ipc-protocol.md`](../spec/01-ipc-protocol.md)
- PTY child lifecycle → [`../spec/02-pty-and-terminal.md`](../spec/02-pty-and-terminal.md)
- Persistence on shutdown → [`../spec/08-persistence.md`](../spec/08-persistence.md)
- Logging during shutdown and panic → [`logging.md`](logging.md)
