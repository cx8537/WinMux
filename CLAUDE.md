# WinMux — Claude Code Instructions

> This file is loaded at the start of every Claude Code session.
> Keep it short. Deep details live in `docs/`.

## Mission

Build a Windows-native terminal multiplexer with tmux-style session
semantics, without WSL. Single primary user. Three-process architecture.

Repository: <https://github.com/cx8537/WinMux>

For full context, read `docs/spec/00-overview.md` before non-trivial work.

## Absolute Rules — Do Not Violate

1. **Never log PTY input/output content.** Names and metadata only.
2. **Never write to `HKEY_LOCAL_MACHINE`.** `HKEY_CURRENT_USER` only.
3. **Never build shell commands by string concatenation.** Use structured
   APIs (`std::process::Command::arg`).
4. **Never write to locations outside the user's `%APPDATA%\winmux\`,
   the install dir, or explicit user-chosen paths.**
5. **No `unwrap()` or `expect()` in non-test Rust code** unless an
   invariant is guaranteed at compile time and explained in a comment.
6. **No `any` in TypeScript. No `!.` non-null assertions.** Use `unknown`
   and explicit narrowing.
7. **No automatic retries on user-initiated commands.** Infra retries
   (pipe reconnect) are bounded with explicit backoff.
8. **No new dependencies without user approval.** Justify with a short
   rationale: why this crate, what alternatives, maintenance health.
9. **Named Pipes always with explicit ACL and
   `FILE_FLAG_FIRST_PIPE_INSTANCE`.** Never trust default ACLs.
10. **All IPC messages are schema-validated.** Unknown messages return
    an explicit error; never silently ignored.
11. **Three-process boundaries are strict.**
    - `winmux-server` has no GUI dependencies (no Tauri, no React).
    - `winmux-tray` and `winmux-cli` have no PTY dependencies (no
      `portable-pty`, no `alacritty_terminal`).
    - Cross-process communication uses only the Named Pipe protocol
      defined in `crates/winmux-protocol`.
12. **No `println!`, `eprintln!`, `dbg!`, or `console.log` in committed
    code.** Use `tracing` (Rust) or the logger wrapper (TypeScript).
13. **`unsafe` Rust requires a `// SAFETY:` comment** explaining
    invariants. Default deny via Cargo lints.
14. **Optimization requires measurement.** No "this looks slow"
    refactors. PRs that touch hot paths must attach before/after
    numbers.
15. **Tests use real environments, not mocks.** Real PTY via
    `portable-pty`, real Named Pipes, real SQLite (in-memory OK), real
    files in `tempdir`.

## Workflow

Before any non-trivial change:

1. Identify which docs from the table below are relevant. Read them.
2. If the change spans process boundaries, also read
   `docs/spec/00-overview.md`.
3. Implement the change.
4. Run, in order:
   - `cargo fmt --all`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace`
   - `npm run lint`
   - `npm run typecheck`
   - `npm run test`
5. Update affected docs in the same PR. If you did not, state why in the
   PR description.
6. Commit using Conventional Commits and include the
   `Co-Authored-By: Claude <noreply@anthropic.com>` trailer.

## When to Read Which Doc

| If you are working on... | Read first |
| --- | --- |
| Project goals, architecture, process boundaries | `docs/spec/00-overview.md` |
| IPC messages, protocol versioning, pipe ACLs | `docs/spec/01-ipc-protocol.md` |
| PTY, ConPTY, virtual terminal, scrollback | `docs/spec/02-pty-and-terminal.md` |
| Session, window, pane data model | `docs/spec/03-session-model.md` |
| Key handling, prefix, `.tmux.conf` parsing | `docs/spec/04-key-handling.md` |
| What tmux features we do or do not support | `docs/spec/05-tmux-compat.md` |
| CLI commands and arguments | `docs/spec/06-cli.md` |
| Tray, GUI, panel layout, window management | `docs/spec/07-tray-and-gui.md` |
| Persistence, session serialization, restore | `docs/spec/08-persistence.md` |
| `winmux.toml` schema, defaults, migration | `docs/spec/09-config.md` |
| i18n, locale, react-i18next setup | `docs/spec/10-i18n.md` |
| Rust style, clippy lints, error handling | `docs/conventions/coding-rust.md` |
| TypeScript style, ESLint, React patterns | `docs/conventions/coding-typescript.md` |
| Type naming, file naming, identifier rules | `docs/conventions/naming.md` |
| Branches, commits, PR conventions | `docs/conventions/git.md` |
| Security model, threats, mitigations | `docs/nonfunctional/security.md` |
| Performance SLOs and measurement | `docs/nonfunctional/performance.md` |
| Resource limits, graceful shutdown, panics | `docs/nonfunctional/stability.md` |
| Log levels, what to log, what never to log | `docs/nonfunctional/logging.md` |
| Keyboard access, IME, CJK, contrast | `docs/nonfunctional/accessibility.md` |
| Test units, scenarios, CI policy | `docs/nonfunctional/testing.md` |
| Dev environment, build, release | `docs/build/dev-setup.md`, `docs/build/release.md` |
| Manual test checklist, troubleshooting | `docs/ops/manual-test-checklist.md`, `docs/ops/troubleshooting.md` |
| Major decisions and their rationale | `docs/decisions.md` |
| Known issues and workarounds | `docs/known-issues.md` |

If two docs disagree, ask the user. Do not guess.

## Process Boundary Quick Reference

| Responsibility | server | tray | cli |
| --- | :---: | :---: | :---: |
| Owns ConPTY handles | ✓ | | |
| Owns child shell processes (via Job Object) | ✓ | | |
| Maintains virtual terminal state (`alacritty_terminal`) | ✓ | | |
| Holds the scrollback buffer | ✓ | | |
| Hosts the Named Pipe server | ✓ | | |
| Parses `.tmux.conf` | ✓ | | |
| Stores audit log (SQLite) | ✓ | | |
| Manages `HKCU\...\Run` autostart | ✓ | | |
| Runs xterm.js terminal renderer | | ✓ | |
| Implements prefix key state machine | | ✓ | |
| Shows the tray icon and main window | | ✓ | |
| Handles `Ctrl+C` copy-when-selection behavior | | ✓ | |
| Performs single-shot commands then exits | | | ✓ |
| Communicates only via Named Pipe `\\.\pipe\winmux-{user}` | client | client | client |

Shared message types live in `crates/winmux-protocol`. Both clients and
the server depend on it. Nothing else is shared.

## When in Doubt

- **Ask the user.** Do not invent behavior to fill gaps.
- **Find the canonical decision** in `docs/spec/*` or
  `docs/decisions.md`. If unclear, ask.
- **Prefer no change over a guess.** A stale TODO is safer than a wrong
  implementation.
