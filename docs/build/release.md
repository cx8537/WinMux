# Release

> Build the installer, cut a tag, publish to GitHub Releases.

WinMux is distributed as a Windows installer (NSIS by default; MSI
optional). No auto-update — users download new releases from
<https://github.com/cx8537/WinMux/releases>.

---

## Versioning

[Semantic Versioning](https://semver.org/).

- `0.x.y` — pre-1.0. Breaking changes allowed within a zero-major,
  but documented as breaking in commits.
- `1.0.0` — after M3 (full persistence, autostart, tray polish) is
  stable and the manual test checklist passes.
- IPC **protocol version** (the `v` field) bumps independently when
  the wire format breaks compatibility.

---

## Build Profile

Release profile in `Cargo.toml`:

```toml
[profile.release]
opt-level = 3
codegen-units = 1
lto = "fat"
strip = "symbols"
panic = "abort"
```

`lto = "fat"` and `codegen-units = 1` make the release build slower
but produce smaller, faster binaries. `panic = "abort"` is required
(see [`../nonfunctional/stability.md`](../nonfunctional/stability.md)).

---

## Build

```powershell
# Verify
npm run verify

# Production build (Vite + Tauri)
npm run tauri build
```

Outputs:

- `src-tauri/target/release/winmux-tray.exe`
- `target/release/winmux-server.exe`
- `target/release/winmux.exe`
- Installer at `src-tauri/target/release/bundle/nsis/WinMux_<version>_x64-setup.exe`

The installer bundles all three executables.

### Tauri bundle configuration

`src-tauri/tauri.conf.json` controls the installer:

```json
{
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "identifier": "com.cx8537.winmux",
    "icon": ["icons/icon.ico"],
    "windows": {
      "wix": null,
      "nsis": {
        "installerIcon": "icons/icon.ico",
        "installMode": "perUser",
        "languages": ["English", "Korean"]
      }
    }
  }
}
```

- **Per-user install.** `installMode: "perUser"` writes to
  `%LOCALAPPDATA%\Programs\WinMux\`. No admin required.
- **NSIS** (not WiX/MSI) by default. Smaller, simpler, no MSI cache
  considerations.
- Optional MSI build (`targets: ["msi"]`) for enterprise users who
  prefer it.

---

## Cutting a Release

### 1. Update `CHANGELOG.md`

Move items from `[Unreleased]` to a new version section:

```markdown
## [0.2.0] - 2026-05-15

### Added
- ...

### Changed
- ...

### Fixed
- ...
```

### 2. Bump versions

Single source of truth in `package.json` and in each `Cargo.toml`'s
workspace `[workspace.package].version`. A helper script bumps all
of them at once:

```powershell
npm run bump 0.2.0
```

The script edits:

- `package.json`
- `Cargo.toml` workspace version
- `src-tauri/tauri.conf.json` `version` field

### 3. Commit

```powershell
git add CHANGELOG.md package.json Cargo.toml Cargo.lock src-tauri/tauri.conf.json
git commit -m "chore(release): v0.2.0"
```

### 4. Tag

```powershell
git tag -a v0.2.0 -m "v0.2.0"
git push origin main
git push origin v0.2.0
```

### 5. CI builds artifacts

GitHub Actions watches for tag pushes matching `v*` and runs the
release workflow:

1. Build on a Windows runner.
2. Run `npm run verify` and the full test suite.
3. Run `npm run tauri build`.
4. Compute SHA-256 of the installer.
5. Create a GitHub Release draft with the installer attached.

### 6. Publish the release

Edit the GitHub Release draft:

- Release notes copied from `CHANGELOG.md` for this version.
- SHA-256 hashes listed in a "Verification" section.
- Note about SmartScreen warnings (the installer is unsigned in
  pre-1.0 releases).

Publish.

---

## Manual Verification Before Publishing

Before clicking Publish:

- Download the artifact from the CI run.
- Install on a clean VM (Windows 11 fresh install).
- Run through the manual test checklist
  ([`../ops/manual-test-checklist.md`](../ops/manual-test-checklist.md)).
- Test upgrade-over-existing-version.
- Test uninstall and reinstall.

If any item fails, abort. Fix and re-tag.

---

## Code Signing

Pre-1.0 releases are **not signed**. Users see SmartScreen warnings.

We document this in the release notes and in the README:

> Pre-1.0 builds of WinMux are not signed. Windows SmartScreen will
> warn you when you run the installer. Click "More info" → "Run
> anyway." We provide SHA-256 hashes so you can verify the download.

For 1.0, we will reconsider acquiring a code-signing certificate.
EV certificates are expensive; standard certs need reputation buildup
before SmartScreen stops warning. The financial commitment is
deferred until there's a sustainable funding source.

---

## SHA-256 Hashes

The CI workflow generates and uploads SHA-256 hashes of each
artifact alongside the artifact itself. Hashes are computed with
PowerShell's `Get-FileHash`:

```powershell
Get-FileHash -Algorithm SHA256 .\WinMux_0.2.0_x64-setup.exe
```

Users can verify after downloading:

```powershell
$h = Get-FileHash -Algorithm SHA256 .\WinMux_0.2.0_x64-setup.exe
$h.Hash.ToLower()
# Compare against the value in the Release notes.
```

---

## Hotfix Releases

For an urgent fix in a shipped version:

1. Branch from the tag: `git checkout -b hotfix/0.2.1 v0.2.0`.
2. Cherry-pick or write the fix.
3. Update CHANGELOG.
4. Bump patch version: `npm run bump 0.2.1`.
5. Commit.
6. Merge into `main`.
7. Tag from `main`: `git tag -a v0.2.1`.

We don't maintain release branches long-term. The latest release is
the supported release.

---

## Pre-releases

For testing builds:

```
v0.2.0-rc.1
v0.2.0-beta.1
v0.2.0-alpha.1
```

These get a GitHub pre-release flag (not "latest"). Same artifact
build pipeline, separate distribution channel for early testers.

---

## Rolling Back

If a published release has a critical issue:

1. Mark the GitHub release as "Pre-release" (this removes it from
   "latest" display).
2. Edit the release notes with a prominent warning at the top.
3. Issue a hotfix release ASAP (see above).

We do **not** delete old releases. Users may have downloaded them
already and the hashes need to remain verifiable.

---

## Uninstall

The NSIS installer registers an uninstaller in Add/Remove Programs.
Uninstall removes:

- The executables from `%LOCALAPPDATA%\Programs\WinMux\`.
- The Start menu shortcut.
- The `HKCU\...\Run` autostart entry (if enabled).

Uninstall does **not** delete:

- `%APPDATA%\winmux\` (user data, sessions, logs, config).

This is documented in the installer UI and the README. Users who
want a complete clean must delete `%APPDATA%\winmux\` manually.

---

## Reproducible Builds

We aim for reproducibility, not deterministic-binary identity. With
LTO and codegen-units = 1, two builds on the same machine produce
the same binary; across machines, differences in absolute paths in
debug info may differ. We strip symbols in release builds, which
helps.

CI uses a fixed Rust toolchain version (pinned in
`rust-toolchain.toml`) and a fixed Node version (declared in
`.nvmrc`). Builds in CI are reproducible enough that hash
comparison across re-runs is meaningful.

---

## Related Docs

- Dev setup → [`dev-setup.md`](dev-setup.md)
- Versioning details → [`../conventions/git.md`](../conventions/git.md)
- Manual test checklist → [`../ops/manual-test-checklist.md`](../ops/manual-test-checklist.md)
- Decision against auto-update → [`../decisions.md`](../decisions.md)
