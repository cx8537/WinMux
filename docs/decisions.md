# Major Design Decisions

The handful of decisions that shape everything else. Each entry
records what was chosen, what was rejected, and why. These are not
ADRs in the strict sense; they are short notes meant to prevent
re-litigation.

If you are adding a feature and find yourself wanting to change one of
these, **open an issue first**. Do not silently revisit.

---

## D-1. Three-process architecture

**Decision.** WinMux ships as three separate executables:

- `winmux-server.exe` — background daemon, owns ConPTY handles and
  virtual terminal state. No GUI dependencies.
- `winmux-tray.exe` — Tauri app with tray icon and main window. No
  PTY dependencies.
- `winmux.exe` — single-shot CLI. Minimal dependencies.

They communicate via Named Pipe (`\\.\pipe\winmux-{user}`).

**Rejected alternatives.**

- *Single-process with hidden window.* Simple, but a tray crash kills
  every running shell. WinMux's whole premise is "GUI dies, shells
  live." A single process makes that premise fragile.
- *Two processes (server + combined tray/CLI).* The CLI is supposed
  to be lightweight and scriptable. Bundling it with the Tauri shell
  would make `winmux ls` take a second to start up.
- *Windows Service.* Service processes run in session 0 by default,
  cannot see the user's environment naturally, and require admin
  privileges to install. A user-mode `DETACHED_PROCESS` server
  reaches the same goals without the friction.

**Why this matters.** Every code change has to respect the boundaries.
If a new module needs both PTY and GUI, the design is wrong: the work
crosses the IPC.

---

## D-2. Background server uses `DETACHED_PROCESS`, not Windows Service

**Decision.** `winmux-server.exe` is launched as a user-mode process
with `CREATE_DETACHED_PROCESS | CREATE_NO_WINDOW`. Either the tray or
the CLI starts it on demand. Optional autostart is via
`HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`, not
a service.

**Rejected.** Windows Service registration. Required admin rights,
needed session 0 workarounds, blocked easy uninstall. The benefits
(start before login) don't apply: WinMux is a per-user tool.

---

## D-3. Per-user installation, never `HKLM`

**Decision.** WinMux installs into `%LOCALAPPDATA%\Programs\WinMux\`
by default. All persistent state lives in `%APPDATA%\winmux\`. All
registry writes go to `HKEY_CURRENT_USER`. No `HKEY_LOCAL_MACHINE`,
ever.

**Why.** Users with multiple Windows accounts on the same machine must
be isolated. Per-machine installation breaks this. It also means
WinMux installs without admin rights.

---

## D-4. tmux compatibility is "core experience," not "100% bytewise"

**Decision.** WinMux implements the *concepts* of tmux: session,
window, pane, prefix key, copy mode, `.tmux.conf` for binding and
options. It does not aim to be a drop-in replacement for tmux scripts
that depend on niche behavior.

Implementation is phased:

- **Phase A** — `set`/`setw`, `bind`/`unbind`, core key tables,
  prefix.
- **Phase B** — `bind -n`, `bind -T <table>`, command chaining,
  `source-file`.
- **Phase C** — `if-shell`, hooks, `run-shell` (opt-in; runs
  arbitrary commands).

See [`spec/05-tmux-compat.md`](spec/05-tmux-compat.md) for the
explicit matrix.

**Rejected.** "100% compatibility." That commitment would never end
and would force support for legacy tmux quirks that have no value on
Windows.

---

## D-5. No automatic updates

**Decision.** WinMux does not check for updates automatically and
does not download or install them on its own. Users update by
downloading a new installer from GitHub Releases.

**Mitigations.**

- Optional manual "Check for updates" item in the tray menu (fires a
  single request to GitHub Releases API).
- Optional "check on startup" toggle, default off.
- Protocol version mismatches show a clear error with a "Restart
  server" button.
- Settings file uses a `schema_version` field; the app migrates
  forward and writes a backup file.

**Why.** Auto-update is significant maintenance work, requires code
signing to avoid endless SmartScreen warnings, and is a recurring
source of security problems. A one-person open-source project should
not own that complexity.

---

## D-6. No telemetry, ever

**Decision.** WinMux does not collect usage analytics, error
telemetry, ping-back, or any data that leaves the user's machine
without an explicit user action.

The only outbound HTTP request the app can ever make is the optional
manual "Check for updates" call to GitHub.

**Why.** It is the right default for a developer tool and a
single-author project. Removing telemetry later is much harder than
never adding it.

---

## D-7. Background process owns ConPTY handles

**Decision.** All `CreatePseudoConsole` calls happen inside
`winmux-server.exe`. Tray and CLI never hold HPCON handles.

**Why.** ConPTY handles are tied to the lifetime of the process that
holds them. If the tray owned them, closing the tray would kill the
shells — the opposite of what we want.

**Consequence.** Tray and CLI cannot do anything PTY-related directly;
they always send a request over the Named Pipe and the server
responds. This is by design.

---

## D-8. Use `alacritty_terminal` for virtual terminal state

**Decision.** The server maintains an in-memory virtual terminal per
pane using `alacritty_terminal`. When a client attaches or reattaches,
the server serializes the current screen state into escape sequences
and sends it as one block. Raw replay of the byte stream is forbidden.

**Why.** Raw replay breaks for any program that clears the screen
(vim, less, htop). Re-running every byte through a virtual terminal
and then snapshotting the result is the only correct way.

**Alternative considered.** `vt100` crate. Lighter but with weaker
coverage of modern escape sequences. `alacritty_terminal` has been
battle-tested by a popular terminal emulator.

---

## D-9. Named Pipe with explicit ACL and `FILE_FLAG_FIRST_PIPE_INSTANCE`

**Decision.** Every Named Pipe instance is created with:

- An explicit security descriptor that allows only the current user's
  SID full access. Everyone else is denied.
- The `FILE_FLAG_FIRST_PIPE_INSTANCE` flag, so an attacker cannot
  pre-register the pipe and impersonate the server.

The server also verifies the connecting client's SID using
`GetNamedPipeClientProcessId` + `OpenProcessToken`. Mismatched SIDs
are logged and disconnected.

**Why.** A Named Pipe is the IPC backbone. Anything that goes wrong
here compromises the entire process boundary model.

---

## D-10. UI is English + Korean, code is English-only

**Decision.**

- All code, comments, identifiers, commit messages, and docs are in
  English.
- The GUI is localized into English and Korean. The system language
  is detected on first launch and the user can switch later.
- `react-i18next` provides the infrastructure. Translation keys are
  English (`session.create.title`), values are localized strings.

**Why.** Code-side English keeps the project approachable to outside
contributors. UI-side Korean acknowledges that the primary user is
Korean.

---

## D-11. Disk scrollback is opt-in per session

**Decision.** Memory scrollback is on by default (default 10,000
lines). Disk-backed scrollback is off by default and must be enabled
per-session, with an explicit confirmation modal that warns about
sensitive output.

**Why.** PTY output regularly contains secrets (`.env` dumps, AWS
configure, git credential prompts). Persisting it to disk is a real
risk. The user must opt in with full awareness.

---

## D-12. Auto-retry policy: never on user commands

**Decision.** If a user-issued command fails (spawn a shell, run a
build), WinMux stops and surfaces the error. It does not retry
silently or automatically.

Infra operations (reconnecting a Named Pipe when the server has just
started) may retry with a bounded backoff (100 ms, 300 ms, 1 s, 3 s,
then give up).

**Why.** Inherited from the Sidabari project. Hidden retries make
errors slippery and timing-dependent; explicit failure is easier to
debug.
