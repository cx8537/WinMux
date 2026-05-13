# 01 — IPC Protocol

> The Named Pipe protocol between `winmux-server.exe` and clients
> (`winmux-tray.exe`, `winmux.exe`).

---

## Transport

### Pipe name

`\\.\pipe\winmux-{user_sha8}`

- `{user_sha8}` is the first 8 hex characters of `SHA-256(username)`.
  Using a hash (rather than the raw username) avoids issues with
  usernames that contain characters Named Pipes do not allow.
- One server instance per user. Enforced by Named Mutex
  `Local\WinMux-Server-{user_sha8}`.

### Pipe creation

The server creates the pipe with:

- `PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED`
- `PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT`
- `nMaxInstances = PIPE_UNLIMITED_INSTANCES`
- A security descriptor that:
  - Grants `GENERIC_READ | GENERIC_WRITE` to the current user SID.
  - Grants nothing to anyone else (no Administrators, no SYSTEM,
    no Everyone).
- `FILE_FLAG_FIRST_PIPE_INSTANCE` is non-negotiable. It prevents an
  attacker from racing to create a pipe with the same name and
  impersonating the server.

### Client connection flow

1. Client calls `CreateFile("\\.\pipe\winmux-{user_sha8}", ...)`.
2. If `ERROR_FILE_NOT_FOUND`: no server. Behavior depends on client
   (see `00-overview.md` for tray and CLI specifics).
3. If `ERROR_PIPE_BUSY`: call `WaitNamedPipe` with a short timeout
   (3 s), then retry.
4. On connect, the server:
   - Calls `GetNamedPipeClientProcessId` and `OpenProcessToken` on
     the client.
   - Compares the client's user SID to the server's user SID. Mismatch
     → log and disconnect.
   - Calls `RevertToSelf` to ensure no accidental impersonation.

### Framing

The protocol is **JSON Lines** over the pipe:

- Each message is a single JSON object.
- Messages are separated by exactly one `\n` (line feed).
- Encoding is UTF-8.
- No BOM.
- Max message size: 16 MiB. Larger messages are an error and disconnect.

Why JSON Lines:

- Trivially debuggable. Tail the pipe in PowerShell for a moment and
  read what's flying.
- Forward-compatible with MessagePack in a later version (switch
  encoded based on `HELLO` negotiation).

---

## Versioning

Every message carries a top-level `v` field with the protocol version
as an integer.

```json
{ "v": 1, "type": "Hello", ... }
```

- Current protocol version: `1`.
- Bump on any breaking change (renamed field, changed semantics, new
  required field).
- Additive changes (a new optional field, a new message type) do
  **not** bump the version.

On connect, client sends `Hello` with its `v`. Server responds with
`HelloAck` containing the server's `v`. If incompatible, the server
sends `Error { code: "VERSION_MISMATCH", ... }` and disconnects. The
client surfaces a user-readable error with a "Restart server" button.

---

## State Machine

Every client connection passes through these states:

```
Disconnected
    │
    │  TCP-style: client connects, server accepts
    ▼
Greeting        ── client must send Hello before anything else
    │
    │  client → Hello
    │  server → HelloAck
    ▼
Authenticated   ── client SID verified
    │
    │  any control message
    ▼
Active          ── may send/receive any message
    │
    │  Bye (either direction) or pipe break
    ▼
Disconnected
```

Sending a non-Hello message in `Greeting` state → `Error
{ code: "PROTOCOL_VIOLATION" }` and disconnect.

---

## Message Catalog

Every message has at least:

```json
{
  "v": 1,
  "type": "<MessageType>",
  "id": "<uuid-v4>"   // optional except where noted
}
```

`id` correlates request and response messages. Streaming messages
(such as `PtyOutput`) do not carry an `id`.

### Lifecycle

#### `Hello` (client → server, required first message)

```json
{
  "v": 1,
  "type": "Hello",
  "id": "<uuid>",
  "client": "tray" | "cli",
  "pid": 12345,
  "version": "0.1.0"
}
```

#### `HelloAck` (server → client)

```json
{
  "v": 1,
  "type": "HelloAck",
  "id": "<request-uuid>",
  "server_version": "0.1.0",
  "user": "username"
}
```

#### `Ping` / `Pong` (either direction)

For health checks. Tray sends `Ping` every 30 s. Server replies within
5 s with `Pong` or the tray surfaces a "server unresponsive" notice
in the tray icon. No automatic restart.

#### `Bye` (client → server) / `ServerBye` (server → all clients)

```json
{ "v": 1, "type": "Bye" }
```

Clean disconnect. Server `Bye`s are sent immediately before shutdown.

### Sessions

#### `ListSessions` / `SessionList`

Request → response. `SessionList` carries an array of:

```json
{
  "id": "ses-01HKJ...",
  "name": "work",
  "created_at": "2026-05-11T09:32:11+09:00",
  "windows": 3,
  "attached_clients": 1
}
```

#### `NewSession`

```json
{
  "v": 1,
  "type": "NewSession",
  "id": "<uuid>",
  "name": "work",         // optional
  "shell": "pwsh",        // optional, default from config
  "cwd": "C:/projects",   // optional, default user home
  "env": { ... },         // optional, additive over user env
  "detached": false       // if true, do not auto-attach this client
}
```

Response: `Attached` (or `Error`).

#### `KillSession`

```json
{ "v": 1, "type": "KillSession", "id": "<uuid>", "session": "work" }
```

Response: `Ok` or `Error`.

### Attach / detach

#### `Attach`

```json
{
  "v": 1,
  "type": "Attach",
  "id": "<uuid>",
  "session": "work" | { "id": "ses-..." },
  "client_size": { "rows": 40, "cols": 120 }
}
```

#### `Attached` (server → client)

```json
{
  "v": 1,
  "type": "Attached",
  "id": "<request-uuid>",
  "session_id": "ses-...",
  "active_window": "win-...",
  "panes": [ { "id": "pane-...", "rows": 40, "cols": 120, ... } ],
  "initial_snapshots": [
    { "pane_id": "pane-...", "bytes_base64": "<...>" }
  ]
}
```

`initial_snapshots` is the server's virtual terminal state serialized
into escape sequences for each pane, sent once per attach. The client
writes this to xterm.js to reproduce the screen.

#### `Detach`

```json
{ "v": 1, "type": "Detach", "id": "<uuid>" }
```

### PTY I/O

#### `PtyInput` (client → server)

```json
{
  "v": 1,
  "type": "PtyInput",
  "pane_id": "pane-...",
  "bytes_base64": "<base64 of raw bytes>"
}
```

Streaming. No `id`. Server forwards to ConPTY input handle. No
buffering at the IPC layer beyond what is necessary; for keyboard
input, latency is more important than throughput.

#### `PtyOutput` (server → client)

```json
{
  "v": 1,
  "type": "PtyOutput",
  "pane_id": "pane-...",
  "bytes_base64": "<base64 of raw bytes>"
}
```

Streaming. Broadcast to every client attached to the same session.

Server batches PTY reads into messages of roughly 4 KiB or 16 ms,
whichever comes first. Keyboard echo, which is small, still arrives
quickly because of the time bound.

### Window / pane control

#### `NewWindow`

```json
{
  "v": 1,
  "type": "NewWindow",
  "id": "<uuid>",
  "session": "work",
  "shell": "pwsh"   // optional
}
```

#### `SplitPane`

```json
{
  "v": 1,
  "type": "SplitPane",
  "id": "<uuid>",
  "pane_id": "pane-...",
  "direction": "horizontal" | "vertical",
  "percentage": 50    // optional
}
```

#### `KillPane`, `KillWindow`

```json
{ "v": 1, "type": "KillPane", "id": "<uuid>", "pane_id": "pane-..." }
```

#### `Resize`

```json
{
  "v": 1,
  "type": "Resize",
  "id": "<uuid>",
  "pane_id": "pane-...",
  "rows": 30,
  "cols": 100
}
```

Triggers `ResizePseudoConsole` on the server and updates the virtual
terminal.

#### `SelectPane`, `SelectWindow`

```json
{ "v": 1, "type": "SelectPane", "id": "<uuid>", "direction": "left" }
```

### tmux-style commands

#### `Command`

```json
{
  "v": 1,
  "type": "Command",
  "id": "<uuid>",
  "tmux": "send-keys",
  "args": ["-t", "work:0", "ls", "Enter"]
}
```

Executes a tmux-equivalent command. The server parses `tmux + args`
into its internal command enum and runs it.

#### `CommandResult`

```json
{
  "v": 1,
  "type": "CommandResult",
  "id": "<request-uuid>",
  "ok": true,
  "stdout": "...",
  "stderr": null
}
```

### Events (server → client, push)

After successful attach, the client implicitly subscribes to events
for the attached session.

- `PaneExited { pane_id, exit_code }`
- `WindowClosed { window_id }`
- `SessionRenamed { session_id, name }`
- `PaneTitleChanged { pane_id, title }`
- `AlertBell { pane_id }`

Events carry no `id`.

### Errors

```json
{
  "v": 1,
  "type": "Error",
  "id": "<request-uuid>",   // optional; correlates if caused by a request
  "code": "PROTOCOL_VIOLATION",
  "message": "Hello required before any other message",
  "recoverable": false
}
```

Common codes:

| Code | Meaning |
| --- | --- |
| `VERSION_MISMATCH` | Client and server protocol versions differ |
| `PROTOCOL_VIOLATION` | Message out of sequence or malformed |
| `UNKNOWN_MESSAGE_TYPE` | Type field not recognized at this protocol version |
| `SESSION_NOT_FOUND` | Referenced session does not exist |
| `WINDOW_NOT_FOUND` | Referenced window does not exist |
| `PANE_NOT_FOUND` | Referenced pane does not exist |
| `PERMISSION_DENIED` | Client SID does not match server SID |
| `TOO_LARGE` | Message exceeded 16 MiB |
| `TIMEOUT` | Request did not get a response in 5 s |
| `INTERNAL` | Server bug; details in the log |

`recoverable: true` means the client may continue. `false` means the
server is going to disconnect; the client should not retry on the same
connection.

---

## Timeouts and Backoff

| Operation | Timeout |
| --- | --- |
| Request without explicit timeout | 5 s |
| `Attach` (includes initial snapshot serialization) | 10 s |
| `NewSession` (includes shell spawn) | 10 s |
| `Ping` round trip | 5 s |
| Client reconnect retry pattern | 100 ms, 300 ms, 1 s, 3 s, give up |

Streaming messages (`PtyInput`, `PtyOutput`) have no per-message
timeout, but the broadcast queue is bounded per client.

---

## Broadcast Queue Limits

Each attached client has a bounded outgoing queue on the server side:

- **Soft limit:** 16 MiB. Server starts dropping `PtyOutput` frames
  for that client and logs a warning.
- **Hard limit:** 64 MiB. Server disconnects the slow client.

Other clients on the same session are unaffected.

When a disconnected client reconnects with `Attach`, it receives a
fresh `initial_snapshot` and continues from there.

---

## Unknown Fields

The server is **strict**: unknown top-level fields in incoming
messages are an error (`PROTOCOL_VIOLATION`). The reasoning is that
silent acceptance of unknown fields makes versioning bugs hard to
catch.

Within message payloads we may relax this later if needed, but for v1
strict everywhere.

---

## Schema Validation

Every incoming message is deserialized into a Rust enum via `serde`.
Deserialization failure → `Error { code: "PROTOCOL_VIOLATION" }`.

The TypeScript client mirrors the same types in `winmux-protocol`
re-exported via `tauri::generate_handler` and `serde` JSON.

There is no separate JSON Schema file; the source of truth is the
Rust enum in `crates/winmux-protocol/src/messages.rs`.

---

## What Is Logged for IPC

- INFO: client connect/disconnect, message type counts per minute,
  message size buckets.
- DEBUG: message types and IDs as they pass through (no payload).
- TRACE: payload sizes only (never content).
- **Never logged:** message payload bytes, `PtyInput` content,
  `PtyOutput` content.

See [`../nonfunctional/logging.md`](../nonfunctional/logging.md) for
the full policy.

---

## Related Docs

- ConPTY and virtual terminal details → [`02-pty-and-terminal.md`](02-pty-and-terminal.md)
- Session model → [`03-session-model.md`](03-session-model.md)
- Security details for IPC → [`../nonfunctional/security.md`](../nonfunctional/security.md)
