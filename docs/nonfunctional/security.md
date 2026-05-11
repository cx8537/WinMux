# Security

> The threat model, the mitigations, and what is explicitly out of
> scope.

WinMux is a developer tool with a non-trivial attack surface: a
long-running background process owning shell child processes, a Named
Pipe IPC channel, a tmux configuration parser, and persistent state on
disk. This document is the canonical reference for security-related
decisions.

If a code change has security implications, this doc must be read and,
if necessary, updated in the same PR.

---

## Assets

What WinMux protects:

1. **Shell sessions and their output.** PTY input/output streams,
   command history, scrollback.
2. **Sensitive data passing through shells.** Passwords, tokens, SSH
   keys, API keys — anything the user types into or sees from a
   shell.
3. **The user's system.** Arbitrary command execution capability via
   the shells WinMux runs.
4. **Session metadata.** Which sessions were created, which commands
   were started, when. (Less sensitive than the output itself, but
   still user data.)
5. **Audit log integrity.** Tampering would defeat after-the-fact
   investigation.

---

## Adversaries

| Adversary | Example | Priority |
| --- | --- | --- |
| Another user account on the same PC | Sister user reading my sessions | **High** |
| Malicious code with user-level privileges | A npm install that runs `winmux ls` | **High** |
| Untrusted `.tmux.conf` from a download or repo | Repo includes a `.tmux.conf` that runs commands | **Medium** |
| Malicious shell output (escape injection) | A web fetch returns a maliciously crafted ANSI stream | **Medium** |
| Administrator / SYSTEM level attacker | Compromised admin account | **Out of scope** |
| Physical access | Someone with the user's unlocked PC | **Out of scope** |
| OS vulnerabilities | Windows zero-days | **Out of scope** |
| Network attack | None — WinMux is not a network service (M0–M4) | **Out of scope** |

### What "out of scope" means

WinMux assumes:

- Windows process and file isolation works as documented.
- The user is the legitimate owner of the user account WinMux runs
  under.
- Code running with Administrator or SYSTEM privileges has already
  compromised the user; WinMux cannot defend further.

These assumptions are stated openly. We do not pretend to defend
against attackers we cannot defend against.

---

## Process and IPC

### Named Pipe

The single most security-critical surface.

- **Pipe name:** `\\.\pipe\winmux-{user_sha8}`, where `{user_sha8}`
  is the first 8 hex characters of `SHA-256(username)`. Per-user
  scope is enforced by name.
- **Creation flags:** `PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE
  | FILE_FLAG_OVERLAPPED`. `FILE_FLAG_FIRST_PIPE_INSTANCE` is
  non-negotiable — it prevents an attacker from pre-registering the
  same pipe name and impersonating the server.
- **Security descriptor:** explicit. Grants `GENERIC_READ |
  GENERIC_WRITE` to the current user's SID, denies everything to
  everyone else. We do not rely on default ACL inheritance.
- **Client verification:** on connect, server calls
  `GetNamedPipeClientProcessId`, then `OpenProcessToken` +
  `GetTokenInformation(TokenUser)` to read the client's SID. If it
  doesn't match the server's SID, the connection is logged at WARN
  and disconnected.
- **Impersonation:** server calls `RevertToSelf` after pipe accept,
  defensively.

### Single instance

- **Server:** Named Mutex `Local\WinMux-Server-{user_sha8}`. Second
  server instance exits silently with status 0.
- **Tray:** Tauri `single-instance` plugin. Second tray foregrounds
  the first and exits.

### Three-process boundaries

The three-process model is partly a security measure: tray code
cannot accidentally hold a HPCON or read raw PTY bytes off disk,
because it has no PTY dependencies at all.

---

## Input Validation

### IPC messages

- Every incoming message goes through `serde` deserialization to a
  Rust enum.
- Unknown fields → `Error { code: PROTOCOL_VIOLATION }`.
- Unknown message types → `Error { code: UNKNOWN_MESSAGE_TYPE }`.
- Max message size: 16 MiB. Larger messages → `TOO_LARGE` and
  disconnect.
- State machine: `Hello` must be the first message. Out-of-state
  messages → `PROTOCOL_VIOLATION`.

### CLI arguments

`clap` parses and validates. Sensitive flags (e.g., `--shell`) are
restricted to known shells unless the user provides an absolute path
they own.

### `.tmux.conf`

Treated as **untrusted input**, even when located in the user's home
directory (a malicious download may have placed it).

Restrictions:

- File size cap: 1 MiB.
- Parsing timeout: 5 seconds.
- `source-file` recursion depth: 5.
- `run-shell`, `if-shell` directives:
  - **Phase A/B (M1, M2):** Parser rejects them with an error
    pointing to the line. The user is told these directives are
    disabled.
  - **Phase C (M4):** Optional, off by default. Enabling requires
    setting `allow_arbitrary_commands = true` in `winmux.toml` AND
    confirming a one-time modal in the GUI. The setting is logged.
- Paths in directives that escape the user's home directory: warn
  but don't reject (M1); decide policy in M2.

### Environment variables

When spawning a child shell:

- Inherit parent (server) env by default.
- WinMux own vars (`WINMUX_SESSION_ID`, `WINMUX_PANE_INDEX`,
  `WINMUX_VERSION`, …) are always set.
- Optional filtering of variables matching patterns like
  `*PASSWORD*`, `*TOKEN*`, `*SECRET*`, `*KEY*`, `*API*`. Off by
  default; user must opt in. Some workflows break under filtering, so
  this is deliberate.

---

## Sensitive Data Handling

### What lives only in memory

- PTY input keystrokes (the raw bytes typed by the user).
- Scrollback when disk mirror is disabled (the default).
- Filtered environment variable values (when filtering is enabled).

Memory holding sensitive data is zeroed via the `zeroize` crate on
the small number of buffers that carry user input across IPC.

### What may be persisted (with care)

- Session metadata: name, creation time, command of first process
  (just the command name, not arguments).
- Audit log (SQLite): events, not content. See below.
- Configuration: paths, settings, no secrets.
- Logs: metadata only, no PTY content.

### What is persisted only with explicit opt-in

- **Disk-backed scrollback.** Off by default. To enable, the user
  must:
  1. Toggle "Persist scrollback to disk" in the session's settings.
  2. Confirm a modal warning: "This session's terminal output will
     be saved to disk. Any password or token output by a command in
     this session (such as `cat .env`, `aws configure`, or
     credential printouts) will be written to disk. Continue?"
  3. The session is then marked with a visible indicator (red dot
     in the GUI, `[DISK]` tag in `winmux ls`).
- **Files:** `%APPDATA%\winmux\scrollback\<session-id>-<window>-<pane>.log`
  with Windows ACL restricting to the current user.
- **Rotation:** size-based with a per-session cap (default 100 MiB)
  and time-based (delete after 7 days unless reset).
- **Optional masking:** user can supply regex patterns to mask lines
  before writing.

### Clipboard

Sidabari-style `Ctrl+C` copies when a selection exists, otherwise
sends `CTRL_C_EVENT`. This is a UX convention.

- OS clipboard is the source of truth; WinMux does not maintain a
  hidden copy buffer for normal copy/paste.
- Bracketed paste mode is enabled by default. Multi-line paste does
  not get a confirmation modal by default; users can enable that in
  settings.
- Audit log records "clipboard copy occurred" with no content. (Even
  this metadata can be useful for incident response.)

---

## Audit Log

Stored at `%APPDATA%\winmux\audit.sqlite` with Windows ACL restricting
to the current user.

### What is recorded

- Process lifecycle: server/tray/cli start, normal exit, crash.
- IPC events: client connect, client disconnect, SID mismatch
  (rejected).
- Session events: created, renamed, closed.
- Spawn events: shell command **name only** (`pwsh`, not the
  arguments).
- Autostart events: registration, removal.
- Sensitive operations:
  - Disk scrollback enabled / disabled per session.
  - `.tmux.conf` load (path + SHA-256 of file).
  - `run-shell` / `if-shell` execution (only when enabled).
  - Forceful pane/window/session kill.

### What is never recorded

- PTY input or output content.
- Keystrokes.
- Environment variable values.
- Clipboard content.
- Command arguments (just the program name).
- File contents.

### Retention

- Default 90 days. Configurable. Maximum enforced cap of 365 days.
- Daily rotation. Old data deleted on server startup.

### Tamper resistance

- SQLite WAL mode, normal transactions, no special crypto.
- We do not claim cryptographic integrity. Audit log is for the
  legitimate user's review, not a forensic-grade tamper-evident log.

---

## Autostart

- Registered under
  `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`,
  never `HKEY_LOCAL_MACHINE`.
- Value name: `WinMux`.
- Value data: absolute path to `winmux-tray.exe` in the installation
  directory.
- Registration requires explicit user confirmation in the tray menu;
  it is **off by default**.
- Removal happens on toggle-off and on uninstall.

The installation directory is in `%LOCALAPPDATA%\Programs\WinMux\`
(per-user install). Not in `%TEMP%`, not in the user's Downloads
folder, not anywhere writable by unprivileged processes.

---

## Spawning Commands

WinMux itself spawns:

- Child shells (`pwsh`, `powershell`, `cmd`, `bash`, …) via ConPTY.
- The server process from the tray or CLI, via `DETACHED_PROCESS`.

No other arbitrary commands. The list is in code; runtime decisions
are based on user configuration or explicit IPC requests.

Rules:

- Always use `std::process::Command::arg`, never string concatenation
  for shell construction.
- The shell path is an absolute path resolved at spawn time. PATH is
  consulted only to find the configured shell binary, and the result
  is canonicalized before use.
- Spawned children are placed in a Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so that server termination
  guarantees descendant cleanup.

---

## Escape Sequence Safety

The PTY output stream may contain hostile ANSI escape sequences:

- Setting persistent colors → annoying but harmless.
- Cursor movement faking prompts → confuses the user.
- OSC 0/2 (window title) → mostly safe; we allow it.
- OSC 50 (font change), OSC 52 (clipboard write/read) → potentially
  dangerous. **Disabled by default.** Enabling requires a setting.

`alacritty_terminal` handles most of this correctly; we additionally
filter dangerous OSCs at the boundary between the virtual terminal
and the client output stream.

---

## Crash and Panic

- Release builds use `panic = "abort"`. We do not `catch_unwind`.
- A panic hook (set in `main.rs` of each process):
  - Writes a crash log to `%APPDATA%\winmux\logs\crash-<ts>.log`.
  - Includes panic message, location, and backtrace.
  - **Excludes** any PTY content (it shouldn't be on the panic
    stack, but the hook is conservative).
- The Job Object on `winmux-server.exe` guarantees that descendant
  shells are terminated when the server dies, preventing zombie
  processes that survive the crash.

Crash logs stay on disk. WinMux does not upload them. The user may
attach a crash log to a GitHub issue voluntarily.

---

## Network

WinMux is not a network application.

The only outbound network call WinMux ever makes is the optional
manual update check, which contacts GitHub's Releases API and only
when the user clicks "Check for updates" in the tray menu, OR if the
"check on startup" toggle is enabled (default off).

Any other network activity from WinMux is a bug. Report it.

---

## Code Signing

Pre-1.0 binaries are **not signed**. Users will see Windows
SmartScreen warnings ("Windows protected your PC") on first run and
must explicitly click "More info" → "Run anyway."

The README explains this and provides SHA-256 hashes for each release
artifact, so users can verify their download.

Signing is reconsidered at 1.0 if there is demand and a sustainable
funding source for the certificate.

---

## What WinMux Does Not Do

By design:

- No telemetry.
- No automatic updates.
- No phone-home.
- No remote attach over the network.
- No multi-user federation.
- No cryptographic key storage (we do not handle passwords or keys
  ourselves; the shell does).
- No "secure enclave" type protections — we cannot defend against
  malicious admin code on the same machine, and we do not pretend
  otherwise.

---

## User Education

The README's Security section, and the equivalent in `README.ko.md`,
explicitly state:

- Use `.tmux.conf` files only from trusted sources.
- Enabling disk-backed scrollback writes terminal output to disk.
- Clipboard paste should be inspected, especially from web pages.
- WinMux does not defend against admin-level attackers or physical
  access.

Putting this in user-facing docs is part of the security model.

---

## Reporting Vulnerabilities

See [`../../SECURITY.md`](../../SECURITY.md).
