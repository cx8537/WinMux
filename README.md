# WinMux

> tmux for Windows, without WSL.

A native Windows terminal multiplexer that brings tmux-style session
persistence, window/pane management, and scriptable terminal control to
PowerShell, cmd, and any other Windows shell — with no WSL required.

[한국어 README](README.ko.md)

![License: MIT](https://img.shields.io/badge/License-MIT-yellow?style=flat-square)
![Platform: Windows 11](https://img.shields.io/badge/Platform-Windows_11-0078D6?style=flat-square&logo=windows11&logoColor=white)
![Tauri 2.x](https://img.shields.io/badge/Tauri-2.x-24C8DB?style=flat-square&logo=tauri&logoColor=white)
![Rust stable](https://img.shields.io/badge/Rust-stable-000000?style=flat-square&logo=rust&logoColor=white)
![Authored by Claude Code](https://img.shields.io/badge/Authored_by-Claude_Code-D97757?style=flat-square&logo=anthropic&logoColor=white)

---

## Why WinMux

tmux is wonderful, but on Windows it lives behind WSL or Cygwin. If you
work in PowerShell, cmd, or a Git Bash that you set up yourself, the
core tmux value — *closing the terminal without killing the work* — is
out of reach.

WinMux runs as a small background process that owns your shell
sessions. Your GUI can come and go. Your laptop can sleep. Your panes
keep running. When you come back, you reattach and pick up where you
left off.

WinMux does not aim for 100% tmux compatibility. It aims for the *core
tmux experience*, expressed in idiomatic Windows.

---

## Architecture in One Picture

```
User ── tray icon ──► winmux-tray.exe ─┐
                                       │  Named Pipe
User ── shell ─────► winmux.exe (CLI) ─┤  \\.\pipe\winmux-<user>
                                       │
                                       ▼
                              winmux-server.exe  ── ConPTY ──► PowerShell / cmd / ...
                              (background, owns PTYs)
```

Three processes, talking over a per-user Named Pipe. The server owns
the shells. The tray gives you a GUI. The CLI is for scripts and
scratch attaches.

See [`docs/spec/00-overview.md`](docs/spec/00-overview.md) for the full
model.

---

## Status

Pre-alpha. Specification and design are stable; implementation is in
progress.

| Milestone | What works |
| --- | --- |
| M0 — PoC | Server spawn, single ConPTY, attach/detach, screen restore |
| M1 — MVP | Multiple sessions/windows/panes, prefix key bindings, basic `.tmux.conf` |
| M2 — Compat | `winmux` CLI, copy mode, more `.tmux.conf` features |
| M3 — Persistence | Session serialization, autostart, full tray polish |
| M4 — Advanced | Multi-client attach, hooks, plugins (TBD) |

This README will be updated as milestones land.

---

## Authorship

> **All code and documentation in this project is written and maintained
> by [Claude Code](https://www.anthropic.com/claude-code). The human
> collaborator (`cx8537`) defines requirements, makes specification
> decisions, performs user testing, and reviews direction.**

| Role | Owner |
| --- | --- |
| Code, refactoring, maintenance | Claude Code |
| All documentation (`README.md`, `docs/**`, `CLAUDE.md`) | Claude Code |
| Specification, requirements, UX decisions, review | cx8537 (human) |
| License and copyright | cx8537 |

Each commit carries a `Co-Authored-By: Claude` trailer.

---

## Features (Planned and Built)

- **Native Windows.** No WSL. No Cygwin. Runs on PowerShell 7,
  Windows PowerShell, cmd, and any shell you point it at.
- **Session persistence across GUI restarts.** Close the tray, reopen
  it, your shells are still there.
- **tmux-style prefix key bindings.** `Ctrl+B` by default (or
  whatever you set in `.tmux.conf`).
- **Multiple sessions, windows, panes.** Split, resize, navigate with
  the keys you already know.
- **Native Windows clipboard integration.** Select text with the mouse,
  `Ctrl+C` copies (when there's a selection) or sends `SIGINT` (when
  there isn't) — the Windows convention.
- **Background daemon with tray icon.** OneDrive-style. Close the
  window, the work keeps running.
- **`winmux` CLI for automation.** `winmux ls`, `winmux attach`,
  `winmux send-keys`, scriptable from PowerShell.
- **Localized UI in English and Korean.** Picks your system language.
- **No telemetry. No autoupdate. No phone home.** You install it, you
  own it.

---

## Tech Stack

**App shell:** Tauri 2.x

**Frontend:** Vite, React 19, TypeScript, Tailwind, shadcn/ui, Zustand,
Zod, react-i18next, xterm.js v6

**Backend (Rust):** Tokio, `portable-pty`, `alacritty_terminal`,
`russh` (for SSH-aware features), `tracing`, `rusqlite`

**IPC:** Windows Named Pipes, JSON Lines protocol

See [`docs/build/dev-setup.md`](docs/build/dev-setup.md) for full setup.

---

## Quick Start

> WinMux is pre-alpha. These instructions are for developers building
> from source.

```powershell
# Prerequisites: Node.js 20+, Rust stable, Windows 11
git clone https://github.com/cx8537/WinMux.git
cd WinMux
npm install
npm run tauri dev
```

For release builds and installer, see
[`docs/build/release.md`](docs/build/release.md).

---

## Project Layout

```
WinMux/
├── CLAUDE.md                  # Working rules for Claude Code
├── README.md                  # This file
├── README.ko.md               # Korean README
├── SECURITY.md                # How to report security issues
├── CONTRIBUTING.md            # Contribution guidelines
├── CHANGELOG.md               # User-visible changes
├── LICENSE                    # MIT
├── crates/
│   ├── winmux-protocol/       # Shared IPC types
│   ├── winmux-server/         # Background server (no GUI deps)
│   ├── winmux-tray/           # Tray + Tauri GUI
│   └── winmux-cli/            # CLI client
├── src/                       # Frontend (React + xterm.js)
├── src-tauri/                 # Tauri shell for winmux-tray
└── docs/                      # All specs and conventions
    ├── INDEX.md
    ├── decisions.md
    ├── known-issues.md
    ├── spec/                  # Functional specs
    ├── conventions/           # Code style, naming, git
    ├── nonfunctional/         # Security, performance, etc.
    ├── build/                 # Dev setup, release
    └── ops/                   # Troubleshooting, manual tests
```

---

## Security

WinMux is a developer tool with significant attack surface: a
long-running background process, Named Pipe IPC, ConPTY child
processes, and `.tmux.conf` parsing.

The security model is documented in
[`docs/nonfunctional/security.md`](docs/nonfunctional/security.md).

To report a vulnerability, see [SECURITY.md](SECURITY.md).

**In scope:** isolation between different user accounts on the same PC,
preventing Named Pipe impersonation, safe handling of `.tmux.conf`.

**Out of scope:** Administrator/SYSTEM-level attackers, physical
access, OS-level vulnerabilities.

---

## Documentation

| Document | Purpose |
| --- | --- |
| [`docs/INDEX.md`](docs/INDEX.md) | Table of contents for all docs |
| [`docs/spec/00-overview.md`](docs/spec/00-overview.md) | Architecture, three-process model |
| [`docs/spec/05-tmux-compat.md`](docs/spec/05-tmux-compat.md) | What tmux features WinMux supports |
| [`docs/decisions.md`](docs/decisions.md) | Major design decisions and rationale |
| [`docs/known-issues.md`](docs/known-issues.md) | Known limits and workarounds |
| [`CLAUDE.md`](CLAUDE.md) | Working rules for Claude Code |

---

## Contributing

This is a single-author project right now, but issues and PRs are
welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first.

Response times will be irregular.

---

## License

MIT. See [LICENSE](LICENSE).

WinMux is a tool that spawns shells and runs commands. It is provided
**as is, without warranty of any kind.** You are responsible for what
you do with it.
