# Git Conventions

> Branches, commits, and PRs for WinMux.

---

## Branching

This is a single-author project. The branching model is intentionally
light.

- `main` is the only long-lived branch and is always shippable.
- Larger features use `feature/<short-name>` branches, merged via PR.
- Small, low-risk changes can land directly on `main`.
- Tags follow Semantic Versioning: `v0.1.0`, `v0.2.0`, `v1.0.0`.

If at any point this project gains more contributors, this section
gets rewritten.

---

## Commit Messages

WinMux uses
[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).

### Format

```
<type>(<scope>): <short summary>

<optional body, wrapped at 72 chars>

<optional footer>
Co-Authored-By: Claude <noreply@anthropic.com>
```

### Types

| Type | Use |
| --- | --- |
| `feat` | A user-visible feature |
| `fix` | A bug fix |
| `refactor` | Code change with no behavioral effect |
| `perf` | A measurable performance improvement |
| `test` | Adding or fixing tests only |
| `docs` | Documentation only |
| `chore` | Build, CI, dependencies, tooling |
| `style` | Formatting only (rare; rustfmt should prevent this) |
| `revert` | Reverting a previous commit |

### Scopes

The crate or area touched. Use lowercase. Common scopes:

- `server` — `winmux-server`
- `tray` — `winmux-tray`
- `cli` — `winmux-cli`
- `protocol` — `winmux-protocol`
- `pty` — PTY/ConPTY internals (inside `server`)
- `terminal` — virtual terminal (inside `server`)
- `ipc` — Named Pipe code on either side
- `ui` — frontend React/Tailwind
- `i18n` — translations and i18n infrastructure
- `docs` — documentation (paired with `docs` type)
- `ci` — GitHub Actions, builds (paired with `chore` type)

### Examples

```
feat(server): add ConPTY-backed session creation

Adds Pty::spawn() that creates a ConPTY, spawns a child shell, and
attaches the child to a Job Object so cleanup is guaranteed.

Closes #12.

Co-Authored-By: Claude <noreply@anthropic.com>
```

```
fix(tray): prevent main window flicker on rapid show/hide

Co-Authored-By: Claude <noreply@anthropic.com>
```

```
refactor(protocol): extract version constants into version.rs

No behavior change. Future-proofs the v2 protocol bump.

Co-Authored-By: Claude <noreply@anthropic.com>
```

```
docs: clarify Phase B compatibility scope in tmux-compat.md

Co-Authored-By: Claude <noreply@anthropic.com>
```

### Breaking changes

Append `!` after the type/scope **and** include a `BREAKING CHANGE:`
footer:

```
feat(protocol)!: bump IPC protocol to v2

Renamed Attach.client_size to Attach.size.

BREAKING CHANGE: v1 clients cannot connect to v2 servers and vice
versa. The server now responds with code VERSION_MISMATCH.

Co-Authored-By: Claude <noreply@anthropic.com>
```

### Language

Commit subjects and bodies are written in **English**, matching the
rest of the codebase.

### `Co-Authored-By` trailer

Every commit Claude Code helped produce carries:

```
Co-Authored-By: Claude <noreply@anthropic.com>
```

This is the project's authorship transparency policy. If a commit is
authored by `cx8537` without Claude Code involvement (rare), the
trailer is omitted.

---

## Pull Requests

### Title

Same shape as a commit subject:

```
feat(server): add ConPTY-backed session creation
```

### Body

A short description, then a checklist:

```markdown
## Summary

What this PR does and why, in 2–4 sentences.

## Changes

- Bullet list of notable changes
- Files touched if it's not obvious

## Related

- Closes #N (if applicable)
- Related issues: #M, #K

## Checklist

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `npm run lint`
- [ ] `npm run typecheck`
- [ ] `npm run test`
- [ ] Affected docs updated in this PR
- [ ] No new dependencies (or: dependency added is justified in body)
- [ ] CHANGELOG.md updated if user-visible
```

A template lives at `.github/pull_request_template.md`.

### Review

This is a single-author project; PR review is done by the same person
who wrote the PR (with Claude Code), and the value of a PR is
primarily as a unit of change with CI gating and a checkpoint.

- CI must pass.
- The checklist must be checked.
- For non-trivial changes, the description should be enough to read
  later and understand why.

---

## Releases

### Versioning

- `0.x.y` until M3 ships. Breaking changes are allowed within a
  zero-major. They are still documented as breaking in the relevant
  commit.
- `1.0.0` ships after M3 is stable.
- Protocol version (`v` field in IPC) tracks separately. It bumps
  whenever the protocol changes incompatibly.

### Cutting a release

1. Update `CHANGELOG.md` under a new version header.
2. Bump versions in `Cargo.toml` workspace and `package.json`.
3. Commit: `chore(release): v0.2.0`.
4. Tag: `git tag -a v0.2.0 -m "v0.2.0"`.
5. Push tag.
6. GitHub Actions builds the installer; attach to a GitHub Release.
7. Release notes from `CHANGELOG.md`, plus SHA-256 hashes of
   artifacts.

See [`../build/release.md`](../build/release.md) for details.

---

## `Cargo.lock`

`Cargo.lock` **is** committed. WinMux ships binaries; reproducible
builds are valuable.

---

## What Not to Commit

- Compiled artifacts (covered by `.gitignore`).
- Local config (`*.local`).
- Personal data: `winmux.toml.local`, `audit.sqlite`, anything from
  `%APPDATA%\winmux\`.
- Secrets of any kind. There should not be any in this repo, but
  enforce by habit.
