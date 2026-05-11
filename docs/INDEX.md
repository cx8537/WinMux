# Documentation Index

This index is for humans browsing the docs. Claude Code uses the
table in [`CLAUDE.md`](../CLAUDE.md), which is task-oriented.

All docs are in English. The two READMEs (`README.md` and
`README.ko.md`) are the only multilingual files.

---

## Start Here

| Doc | What it covers |
| --- | --- |
| [`decisions.md`](decisions.md) | The handful of decisions that shape everything else. Read first. |
| [`known-issues.md`](known-issues.md) | Known limitations of ConPTY, `alacritty_terminal`, `portable-pty`, etc. Avoid duplicate discoveries. |
| [`spec/00-overview.md`](spec/00-overview.md) | The three-process model. Required reading. |

---

## Specifications (`spec/`)

Numbered roughly in the order you would read them.

| Doc | Topic |
| --- | --- |
| [`spec/00-overview.md`](spec/00-overview.md) | Project identity, three-process architecture, milestones |
| [`spec/01-ipc-protocol.md`](spec/01-ipc-protocol.md) | Named Pipe protocol, message catalog, versioning |
| [`spec/02-pty-and-terminal.md`](spec/02-pty-and-terminal.md) | ConPTY, `portable-pty`, `alacritty_terminal`, scrollback |
| [`spec/03-session-model.md`](spec/03-session-model.md) | Sessions, windows, panes, shells |
| [`spec/04-key-handling.md`](spec/04-key-handling.md) | Prefix state machine, key tables, `.tmux.conf` integration |
| [`spec/05-tmux-compat.md`](spec/05-tmux-compat.md) | Compatibility matrix by phase (A/B/C) |
| [`spec/06-cli.md`](spec/06-cli.md) | `winmux.exe` CLI commands |
| [`spec/07-tray-and-gui.md`](spec/07-tray-and-gui.md) | Tray icon, main window, panel layout |
| [`spec/08-persistence.md`](spec/08-persistence.md) | Session serialization, restore on next boot |
| [`spec/09-config.md`](spec/09-config.md) | `winmux.toml` schema |
| [`spec/10-i18n.md`](spec/10-i18n.md) | English/Korean, react-i18next setup |

---

## Conventions (`conventions/`)

| Doc | Topic |
| --- | --- |
| [`conventions/coding-rust.md`](conventions/coding-rust.md) | rustfmt, clippy rules, error handling, async patterns |
| [`conventions/coding-typescript.md`](conventions/coding-typescript.md) | tsconfig, ESLint, Prettier, React patterns |
| [`conventions/naming.md`](conventions/naming.md) | File names, identifiers, brand naming |
| [`conventions/git.md`](conventions/git.md) | Branches, Conventional Commits, PR template |

---

## Non-functional Requirements (`nonfunctional/`)

| Doc | Topic |
| --- | --- |
| [`nonfunctional/security.md`](nonfunctional/security.md) | Threat model, mitigations, audit log |
| [`nonfunctional/performance.md`](nonfunctional/performance.md) | SLOs, measurement, hot paths |
| [`nonfunctional/stability.md`](nonfunctional/stability.md) | Resource cleanup, graceful shutdown, panics |
| [`nonfunctional/logging.md`](nonfunctional/logging.md) | tracing setup, log levels, what to never log |
| [`nonfunctional/accessibility.md`](nonfunctional/accessibility.md) | Keyboard access, IME, fonts, contrast |
| [`nonfunctional/testing.md`](nonfunctional/testing.md) | Unit definition, scenarios, CI |

---

## Build (`build/`)

| Doc | Topic |
| --- | --- |
| [`build/dev-setup.md`](build/dev-setup.md) | Local development environment |
| [`build/release.md`](build/release.md) | Release builds, installer, versioning |

---

## Operations (`ops/`)

| Doc | Topic |
| --- | --- |
| [`ops/troubleshooting.md`](ops/troubleshooting.md) | Common problems and fixes |
| [`ops/manual-test-checklist.md`](ops/manual-test-checklist.md) | Things to verify by hand before each release |

---

## How These Docs Are Organized

- **One topic per file.** Files split when they exceed roughly a
  thousand lines, except where splitting would be unnatural.
- **No diagrams unless necessary.** When a Mermaid diagram appears, all
  labels are quoted. Text trees and tables are preferred.
- **Code examples are short.** Long examples link to source files
  instead of being duplicated here.
- **Each file is independently readable.** Cross-links exist, but a
  reader should not need to ping-pong between five files to understand
  one decision.

If a doc is unclear, contradictory, or missing, open an issue or ask
the user. Do not silently fix design problems in code.
