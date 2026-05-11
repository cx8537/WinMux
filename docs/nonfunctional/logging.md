# Logging

> What gets logged, where, how, and — critically — what never does.

WinMux uses the `tracing` crate (Rust) and a wrapper around it
exposed to the frontend (`src/lib/logger.ts`). Structured fields are
required; format strings with embedded variables are discouraged.

The single most important rule: **PTY content is never logged.**

---

## Levels

| Level | Use |
| --- | --- |
| `ERROR` | A user-facing failure: command rejected, file unwritable, panic about to happen. Always actionable. |
| `WARN` | Something happened that's recoverable but worth noting: a slow client, a deprecated setting, a retry that succeeded. |
| `INFO` | High-level lifecycle events: process started, client connected, session created, autostart enabled. The default level. |
| `DEBUG` | Per-operation detail useful for diagnosis: IPC message types, state transitions, PTY spawn parameters. Off by default. |
| `TRACE` | Per-byte or per-frame events: queue depths, individual key presses (only the modifier shape, never the data). Off by default. |

`INFO` is the default. `WARN` and `ERROR` are always written.

Per-module filtering via `WINMUX_LOG`:

```
WINMUX_LOG=info,winmux_server::ipc=debug
```

Same format as `RUST_LOG` (it's the same library underneath).

---

## What to Log

### Always (INFO or higher)

- Process start: version, pid, OS info, configuration source.
- Process shutdown: reason, duration.
- Client connect / disconnect: client type (`tray` / `cli`), client
  pid, count of attached clients afterwards.
- Session lifecycle: created, renamed, killed.
- Window / pane lifecycle: opened, closed, killed.
- PTY child exit: pane id, exit code.
- `.tmux.conf` load: path, line count, parse warnings count.
- Autostart toggle: enabled, disabled, by which client.
- Security-sensitive operations: disk scrollback toggled on for a
  session, allow_arbitrary_commands enabled, env filter changes.
- Server-not-found by client (rare, debugging aid).
- Configuration migration: from version X to Y.

### Sometimes (DEBUG)

- IPC message dispatched: type and id (never payload).
- State transitions in the IPC state machine.
- PTY spawn parameters: shell path (canonical), cwd, env override
  keys (not values).
- Snapshot serialization: pane id, output byte count.
- Pipe accept loop: each iteration with reason.
- Scrollback rotation events.

### Rarely (TRACE)

- Key event arrival in the tray.
- Per-frame `PtyOutput` broadcast: pane id, byte count (not bytes).
- Lock acquisition timings.
- Audit log insert rate samples.

### Never

- **PTY input.** Not raw bytes, not even keystroke shapes that could
  reveal typed content.
- **PTY output.** Not raw bytes, not parsed text, not selectively
  decoded strings.
- **Environment variable values.** Names are OK; values never.
- **Command arguments.** The program name (`pwsh`, `git`) is OK; the
  arguments are not.
- **Clipboard content.**
- **File contents of any kind.**
- **Network requests.** WinMux doesn't make any; if a future feature
  does, the URL is OK but the body is not.
- **Passwords, tokens, API keys, SSH keys.** WinMux doesn't see these
  directly, but the rule is restated to make the principle visible.

---

## Output

### Per-process daily files

```
%APPDATA%\winmux\logs\
  server-2026-05-11.log
  server-2026-05-10.log
  tray-2026-05-11.log
  tray-2026-05-10.log
  cli-2026-05-11.log
  crash-server-2026-05-11T09-32-11.log
```

- One file per process per day.
- Rotation at local midnight. The old file is closed; the new file
  opens.
- Crash logs are separate (see [`stability.md`](stability.md)).

### Retention

- 30 days default. Configurable in `winmux.toml`.
- 2 GiB total cap across all logs. Forced rotation kicks in at the
  cap and the oldest files are deleted first.
- Audit log retention is separate and on its own schedule (see
  [`security.md`](security.md)).

### Format

Default: human-readable, multi-line, with structured fields.

```
2026-05-11T09:32:11.234+09:00  INFO  winmux_server::session: session.created session_id="ses-01HKJ..." name="work" shell="pwsh" cwd="C:\\projects"
2026-05-11T09:32:11.567+09:00  INFO  winmux_server::ipc: client.connected pid=12345 type="tray"
2026-05-11T09:32:11.612+09:00  WARN  winmux_server::pty: pty.snapshot.slow pane_id="pane-01HKJ..." duration_ms=42
```

With `--log-format=json` or `WINMUX_LOG_FORMAT=json`, each line is a
JSON object instead. Useful for ingestion into log analysis tools.

```json
{"timestamp":"2026-05-11T09:32:11.234+09:00","level":"INFO","target":"winmux_server::session","fields":{"message":"session.created","session_id":"ses-...","name":"work","shell":"pwsh"}}
```

### Console (dev)

In `cargo run` or `npm run tauri dev`, log lines also go to stderr
with ANSI colors. Off by default in production builds (no console
attached anyway).

The console subscriber and the file subscriber run in parallel via
`tracing_subscriber::Registry`.

### Frontend logger

The TypeScript wrapper:

```typescript
import { logger } from '@/lib/logger';
logger.info('attached to session', { sessionId, paneCount: 4 });
logger.warn('ipc round-trip slow', { ms: 250 });
logger.error('failed to write clipboard', { error });
```

Internals: forwards each call to Tauri via `invoke('log', { level,
message, fields })`. The Rust side emits via `tracing` so logs are
unified across processes.

In dev, also calls `console.<level>` so logs are visible in the
WebView devtools.

---

## Structured Fields

Required for any log line with variables. Bad:

```rust
tracing::info!("session created: {} for shell {}", session_id, shell);
```

Good:

```rust
tracing::info!(
    %session_id,
    shell = %shell.display_name,
    "session.created"
);
```

The `%session_id` syntax uses the `Display` impl; `?session_id` uses
`Debug`. The message ("session.created") is short and event-shaped —
greppable by event name.

Field naming:

- snake_case.
- Same name across the codebase for the same concept (`session_id`,
  not also `sessionId` or `sid`).
- IDs use their newtype `Display` (which produces the prefixed form
  like `ses-01HKJ...`).

---

## Non-blocking Writes

`tracing_appender::non_blocking` is required for the file subscriber.
Blocking on disk I/O in the request path defeats the latency SLOs.

```rust
let (writer, guard) = tracing_appender::non_blocking(file_appender);
// keep `guard` alive for the lifetime of the process
```

If the writer's internal queue fills (very unusual), log events are
dropped rather than the application blocking. A counter in the panel
("logs dropped: N") surfaces this if it happens.

---

## Diagnostic Export

Settings → Advanced → "Export diagnostic bundle" creates a zip:

```
winmux-diag-2026-05-11.zip
├── logs/
│   ├── server-<dates>.log         (last 7 days)
│   ├── tray-<dates>.log           (last 7 days)
│   └── crash-*.log                (all)
├── config/
│   ├── winmux.toml                (with sensitive paths sanitized)
│   └── winmux.conf                (as-is; user-owned)
├── system/
│   ├── version.txt                (winmux, OS, dependency versions)
│   ├── env-names.txt              (names of WINMUX_* env vars only)
│   └── processes.txt              (Get-Process winmux-* output)
└── README.txt                      (what's in here)
```

Sensitive content sanitization:

- Paths under the user's home directory are replaced with `<HOME>`.
- Any matching `filter_env_patterns` are masked.
- The audit log is **not** included unless the user explicitly
  enables a checkbox; even then, only events without payloads
  containing user-supplied identifiers.

The user can review the zip before sharing it with anyone.

---

## Reading the Logs

Quick commands users may find handy (documented in
`ops/troubleshooting.md`):

```powershell
# Tail the server log
Get-Content "$env:APPDATA\winmux\logs\server-$(Get-Date -Format yyyy-MM-dd).log" -Wait

# Find recent ERRORs
Get-Content "$env:APPDATA\winmux\logs\server-*.log" |
    Select-String "ERROR" |
    Select-Object -Last 50

# Convert JSON-formatted logs to a table (if enabled)
Get-Content $log | ConvertFrom-Json |
    Where-Object level -eq "ERROR" |
    Select-Object timestamp, target, fields
```

---

## Pitfalls

- **Don't log inside the panic hook with `tracing`.** The subscriber
  may be in the panic path. Use direct `WriteFile` to the crash log.
- **Don't log payload bytes** even at TRACE. Byte counts only.
- **Don't `Display` user input** if it could be PTY content. Filter
  it to "<N bytes>" instead.
- **Don't log secrets** even if you "intend" to mask them later.
  Mask at the source.

---

## Related Docs

- Security and what counts as sensitive → [`security.md`](security.md)
- Audit log (a separate, structured event store) →
  [`security.md`](security.md)
- Stability and crash logs → [`stability.md`](stability.md)
