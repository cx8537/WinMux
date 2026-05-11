# Contributing to WinMux

Thanks for your interest. This is a single-author project — `cx8537`
defines the specification and Claude Code writes the code. Outside
contributions are welcome but reviewed carefully.

## Before You Start

- Read [`README.md`](README.md) for the project's goals.
- Read [`docs/spec/00-overview.md`](docs/spec/00-overview.md) for the
  architecture.
- Read [`CLAUDE.md`](CLAUDE.md) — even if you are not Claude Code, it
  describes the working rules this project follows.
- Major design decisions live in
  [`docs/decisions.md`](docs/decisions.md). If your idea conflicts with
  one of them, please open an issue to discuss before writing code.

## Reporting Issues

Use GitHub Issues: <https://github.com/cx8537/WinMux/issues>

A good bug report includes:

- WinMux version (`winmux --version`)
- Windows version (`winver`)
- Shell you were running (`pwsh`, `cmd`, etc.)
- Exact steps to reproduce
- What you expected vs what happened
- Relevant log excerpts (`%APPDATA%\winmux\logs\`), with sensitive
  content redacted

For feature requests, please explain the use case before suggesting
the implementation.

## Security Issues

**Do not file security issues as public bug reports.** See
[`SECURITY.md`](SECURITY.md).

## Pull Requests

Before opening a PR:

- [ ] Code formatted: `cargo fmt --all` and `npm run lint`
- [ ] Clippy clean: `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] Tests pass: `cargo test --workspace` and `npm run test`
- [ ] TypeScript checks: `npm run typecheck`
- [ ] Affected docs in `docs/` updated in the same PR
- [ ] Commit messages follow
  [Conventional Commits](https://www.conventionalcommits.org/)
  (see [`docs/conventions/git.md`](docs/conventions/git.md))
- [ ] No new dependencies added without prior discussion in an issue

Smaller PRs are reviewed faster.

## What This Project Will Not Accept

- Telemetry, analytics, or any phone-home behavior
- Auto-update mechanisms (this is intentional — see
  [`docs/decisions.md`](docs/decisions.md))
- Dependencies that require a paid service or call out to the network
  for normal operation
- Changes that break the three-process boundary (server / tray / cli)
- Code that violates the absolute rules in [`CLAUDE.md`](CLAUDE.md)

## Response Time

Realistic expectation: days to weeks. This is a hobby project.

## License

By submitting a PR you agree your contribution is licensed under the
MIT License, the same as the rest of the project.
