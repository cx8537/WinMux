# Testing

> The unit definition, the testing strategy, and the must-pass
> scenarios.

The most important decision in this doc: **the unit**. Without
agreement on what "unit" means in this project, "unit tests" devolve
into either testing nothing or testing everything.

---

## The Unit

> **A unit in WinMux is the public API of one module or one
> type.**

Implications:

- A unit test exercises that public API. Private helpers are tested
  indirectly through it.
- We do **not** unit-test individual functions when the function is an
  implementation detail. Refactoring those should not require
  rewriting tests.
- We do **not** unit-test self-evident wrappers, getters, or `Display`
  impls.
- External dependencies (PTYs, Named Pipes, the filesystem) are used
  **for real**, not mocked, unless real use is impossible in CI.

This is the Sidabari precedent and it has worked.

### What is a unit?

Examples from WinMux:

- `pty::Pty` — the type's public methods (`spawn`, `read`, `write`,
  `resize`, `kill`) are one unit.
- `terminal::VirtualTerm` — wraps `alacritty_terminal`; tested
  through its public API.
- `session::Session` — pure data structure with manipulation methods.
- `ipc::PipeServer` — accepting clients, framing, error handling.
- `conf::Parser` — `.tmux.conf` parser. Pure function input → AST.
- `protocol::Message` — encoding/decoding, equality.

### What is *not* a unit?

- A single function that's an implementation detail.
- A struct field.
- The whole `winmux-server` binary (that's an E2E target, not a
  unit).

---

## Test Levels

```
                      ▲
                     ╱ ╲
                    ╱E2E╲          5–10 scenarios, real binaries
                   ╱─────╲
                  ╱  IT   ╲         30–50 cases, real PTY/pipe
                 ╱─────────╲
                ╱   Unit    ╲       100–200 module-level tests
               ╱─────────────╲
              ╱ Pure unit     ╲     50–100 fast pure-Rust tests
             ╱─────────────────╲
```

### Pure unit

- No external resources. No PTY, no Named Pipe, no filesystem
  (except `tempfile`), no DB.
- Pure functions, parsers, codecs, data structures.
- Targets: `protocol::Message` round-trip, `conf::Parser`, ID
  formatting, `keymap` resolution.
- Performance: hundreds run in well under a second.

### Unit

- Module-level. May spawn a real PTY, open a real Named Pipe, write
  to a temp dir, or open an in-memory SQLite.
- Targets: `Pty::spawn`, `PipeServer::accept`, `Scrollback::append`,
  `AuditLog::record`.
- Performance: low hundreds of ms typical.

### Integration

- Multiple units glued together. Still single-process.
- Targets: server boots, accepts a client, spawns a PTY, broadcasts
  output, all in one test.
- Performance: ~1–5 seconds per test.

### E2E

- Multiple real binaries communicating over a real Named Pipe.
- Driven by a `winmux-e2e` test crate that spawns
  `winmux-server.exe` and a synthetic client.
- Targets: full lifecycle, detach/reattach, crash recovery.
- Performance: 5–30 seconds per test.

---

## Must-Pass Scenarios

These are regression-protection scenarios. **Every one must exist as
an automated test.** Adding or modifying these requires updating this
list.

### E2E

1. **Basic lifecycle.** Start server. Connect client. Create session.
   Spawn `pwsh`. Send `echo hello\n`. Receive output containing
   `hello`. Detach. Reconnect. Initial snapshot contains `hello`.
2. **Persistence across tray restart.** Spawn session. Kill client
   process. Start a new client. Attach to same session. Session is
   alive and shell responsive.
3. **vim screen restore.** Spawn session. Launch `vim` (or any
   alt-screen program). Edit a buffer. Detach. Reattach. Screen
   matches state at detach (no flashing previous content).
4. **Clear and reconnect.** Spawn session. Print output. Run `cls`.
   Print one line. Detach. Reattach. Only the post-`cls` line is
   visible.
5. **High-volume output.** Run `seq 1 100000` (or PowerShell
   equivalent). Confirm all 100,000 lines arrive at the client
   without loss when both are local.
6. **Multiple clients (M4 only).** Two clients attached to same
   session. Input from client A reflects in client B's output.
   Disconnect of client A leaves client B unaffected.
7. **Slow client isolation.** Stall one client's read loop. Confirm
   the other client continues to receive output. Confirm the slow
   client is eventually disconnected with a warning.
8. **CLI roundtrip.** `winmux new-session -s t -d`. `winmux ls`
   shows `t`. `winmux send-keys -t t:0 "echo X" Enter`. `winmux
   capture-pane -t t:0` contains `X`. `winmux kill-session -t t`.
   `winmux ls` does not contain `t`.
9. **Protocol mismatch.** Start a server. Modify the client to send
   `v: 999`. Server responds `VERSION_MISMATCH` and disconnects;
   client surfaces the error.
10. **Graceful shutdown.** `winmux shutdown`. All PTY child
    processes die. No handle leaks (verified by process probing).
    All temp files cleaned.

### Integration (sample)

- `Pty` + `VirtualTerm`: feed a vim escape stream to a spawned PTY,
  verify `VirtualTerm` reaches the expected cell content.
- `VirtualTerm` + `Scrollback`: write beyond capacity; verify oldest
  lines are evicted; disk mirror writes correct bytes.
- `Session` + `Serializer`: round-trip a complex session (3 windows,
  varied pane layouts).
- `PipeServer` + `Dispatcher`: client connects, sends `Hello`, server
  responds `HelloAck`. Wrong order → `PROTOCOL_VIOLATION`.
- `JobHandle` + `Pty`: spawn PTY. Drop the `Pty` (which holds the
  Job). Confirm child process is gone within 5 seconds.

### Unit (sample)

- `protocol::Message::encode` / `decode` round-trip for every message
  variant.
- `protocol::Version::is_compatible` truth table.
- `conf::Parser` on hand-crafted `.tmux.conf` inputs (Phase A and B
  features).
- `keymap::resolve` for representative key combinations.
- `audit::AuditLog::record` then query.

### Pure unit (sample)

- ULID format parsing and IDs.
- Scrollback line counting at boundaries.
- Time / locale formatting.

---

## Test Style

- File location:
  - Pure unit and unit: `#[cfg(test)] mod tests` at the bottom of the
    file.
  - Integration: `crates/<crate>/tests/<name>.rs`.
  - E2E: `crates/winmux-e2e/tests/`.
- Test names: snake_case, descriptive.
  - `test_pipe_server_rejects_other_user_sid`.
  - `test_vim_screen_restored_after_reattach`.
- AAA pattern: Arrange / Act / Assert separated visually.
- One scenario per test. Use multiple tests rather than one big test.
- `#[tokio::test(flavor = "multi_thread")]` for async.
- `unwrap` / `expect` are fine in tests (lint opt-out at module
  level).
- Temp dirs via `tempfile::TempDir`. Avoid hardcoded paths.

---

## What We Don't Test (Automated)

These are verified manually before each release. See
[`../ops/manual-test-checklist.md`](../ops/manual-test-checklist.md).

- Tray icon visual rendering.
- Tray menu interaction (right-click flow).
- Main window show/hide animation.
- Tauri WebView startup time.
- xterm.js visual rendering correctness (font, colors, ligatures).
- DPI changes when moving between monitors.
- Windows SmartScreen behavior on a fresh install.
- IME composition flow with a real IME.

Trying to automate these against the OS is expensive and brittle. A
manual checklist takes minutes and catches what matters.

---

## Tools

- **Rust unit / integration / E2E:** built-in `#[test]`, `#[tokio::test]`.
- **Property-based:** `proptest`, used in the `.tmux.conf` parser to
  fuzz syntactically-valid configurations.
- **Performance:** `criterion`. Adopted from M2 onward.
- **TypeScript unit:** Vitest.
- **TypeScript UI (light):** React Testing Library, mainly for the
  prefix key state machine and store logic, **not** for full
  component rendering.

---

## CI

GitHub Actions, Windows runners.

On every push to a PR and to `main`:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run lint
npm run typecheck
npm run test
```

E2E suite runs:

- Nightly on `main` (scheduled).
- On every tag push.
- Optional `e2e` label on a PR.

Performance bench suite runs:

- Nightly on `main`, comparing against last published baseline.
- Regressions surface as comments on the latest commit; not a hard CI
  fail (false positives are common).

---

## Coverage

We do not target a coverage percentage. Coverage gates create
incentives for tests with no value. We do measure coverage occasionally
(`cargo llvm-cov` or `tarpaulin`) to spot whole modules with no
tests, and add tests when that finding aligns with the must-pass list.

---

## Flaky Tests

Inherited from Sidabari: **no automatic retries**. A flaky test is
either fixed or deleted. We do not paper over timing dependencies
with `--retries 3`.

If a test depends on a Windows behavior with timing variance (e.g.,
"child has exited within 5 seconds"), the test waits with an explicit
bound and asserts within it. Bounds are picked with margin but
documented.
