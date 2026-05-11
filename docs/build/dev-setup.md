# Dev Setup

> Local development environment for WinMux.

WinMux is a Tauri 2 app with a Rust workspace and a Vite + React 19
frontend. Building requires both toolchains.

---

## Prerequisites

### Operating System

- **Windows 11** (recommended for development; that's the target).
- Windows 10 22H2+ should work but is not regularly tested.
- macOS / Linux: code compiles for cross-checking, but you cannot
  run the Windows-specific paths (ConPTY, Named Pipes).

### Rust

```powershell
# Install via rustup
winget install Rustlang.Rustup
# or: https://rustup.rs/

# Restart shell, then:
rustup show
```

The `rust-toolchain.toml` in the repo pins the stable channel that
the project uses. `rustup` will install it automatically the first
time you run `cargo`.

### Node.js

```powershell
# 20 LTS or later. Via winget:
winget install OpenJS.NodeJS.LTS

# Verify
node --version    # v20.x or later
npm --version
```

The project uses npm (matching Sidabari).

### Tauri prerequisites

Tauri 2 on Windows needs:

- **WebView2 Runtime.** Pre-installed on Windows 11; on Windows 10,
  install from
  <https://developer.microsoft.com/microsoft-edge/webview2/>.
- **Visual Studio Build Tools** with the "Desktop development with
  C++" workload, for the linker and Windows SDK.

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--passive --add Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Workload.NativeDesktop"
```

If you already have Visual Studio installed with C++ desktop
development, you're done.

### Optional

- **PowerShell 7** (`pwsh.exe`) — better dev experience than Windows
  PowerShell. WinMux defaults to it as the spawned shell when
  available.
- **Git** (any recent version).
- **VS Code** with the rust-analyzer extension. The repo ships shared
  settings in `.vscode/settings.shared.json` (rename to
  `settings.json` locally).

---

## Clone

```powershell
git clone https://github.com/cx8537/WinMux.git
cd WinMux
```

---

## Install Dependencies

```powershell
npm install
```

This installs:

- Vite, React, TypeScript, Tailwind, shadcn/ui dependencies.
- `@tauri-apps/cli` and `@tauri-apps/api`.
- xterm.js, react-i18next, Zustand, Zod, etc.
- ESLint, Prettier, Vitest.

Rust dependencies are fetched on first `cargo` command:

```powershell
cargo fetch --workspace
```

---

## Run in Dev Mode

```powershell
npm run tauri dev
```

This:

1. Starts the Vite dev server on `http://localhost:1420`.
2. Compiles `winmux-tray` (the Tauri app).
3. Launches the tray with hot-module reload for the frontend.

The first build takes a few minutes (Rust + WebView2 setup).
Subsequent rebuilds are fast.

To run the **server** standalone (for protocol-level testing):

```powershell
cargo run -p winmux-server
```

To run the **CLI**:

```powershell
cargo run -p winmux-cli -- ls
```

---

## Editor Setup

### VS Code

Recommended extensions (also listed in `.vscode/extensions.json`):

- `rust-lang.rust-analyzer`
- `dbaeumer.vscode-eslint`
- `esbenp.prettier-vscode`
- `bradlc.vscode-tailwindcss`
- `tamasfe.even-better-toml`

Project settings (rename `.vscode/settings.shared.json` to
`settings.json`):

```jsonc
{
  "rust-analyzer.cargo.target": "x86_64-pc-windows-msvc",
  "rust-analyzer.checkOnSave": "clippy",
  "rust-analyzer.check.extraArgs": ["--all-targets"],
  "editor.formatOnSave": true,
  "[rust]": { "editor.defaultFormatter": "rust-lang.rust-analyzer" },
  "[typescript]": { "editor.defaultFormatter": "esbenp.prettier-vscode" },
  "[typescriptreact]": { "editor.defaultFormatter": "esbenp.prettier-vscode" },
  "files.eol": "\n"
}
```

---

## Pre-commit Checks

Before pushing:

```powershell
# Rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# TypeScript / React
npm run lint
npm run typecheck
npm run test
```

A `package.json` script bundles them:

```powershell
npm run verify    # runs all the above in sequence
```

---

## Common Issues

### `link.exe not found`

You're missing the VS Build Tools or the right workload. Install the
"Desktop development with C++" workload (see Prerequisites).

### `WebView2Loader.dll missing`

Install the WebView2 Runtime (see Prerequisites). Should be a
no-op on Windows 11.

### Slow first build

Normal. Tauri pulls in many Windows SDK headers on first compile.
Subsequent incremental builds are fast.

### `cargo` can't find target spec for Windows

Make sure you're on Windows. Cross-compiling to Windows from another
OS is not supported by this project.

### Hot reload doesn't update the React app

Check the Vite terminal output. If it's stuck, restart `npm run
tauri dev`. The Rust side does **not** hot-reload — restart for
backend changes.

### `cargo test` hangs

A PTY-using test may be waiting on a child shell. Increase
`RUST_LOG=debug` and look for the stalled spawn. The Job Object
guarantees cleanup on test process exit, but individual tests should
not hang.

---

## Project Layout (Dev View)

```
WinMux/
├── Cargo.toml              # workspace
├── package.json
├── vite.config.ts
├── tsconfig.json
├── tailwind.config.ts
├── rustfmt.toml
├── clippy.toml
├── rust-toolchain.toml
├── crates/
│   ├── winmux-protocol/    # cargo lib, no binary
│   ├── winmux-server/      # cargo bin
│   ├── winmux-tray/        # cargo bin, Tauri host
│   └── winmux-cli/         # cargo bin
├── src/                    # frontend (consumed by winmux-tray)
├── src-tauri/              # Tauri config; winmux-tray entrypoint
└── docs/
```

`src-tauri/tauri.conf.json` is the Tauri configuration. The
`build.beforeDevCommand` runs Vite; `build.beforeBuildCommand` runs
the production Vite build.

---

## CI

The same `verify` runs on Windows runners on every push. See
[`testing.md`](../nonfunctional/testing.md).

---

## Where to Start

For new contributors:

1. Read this file.
2. `npm run tauri dev` and confirm the tray launches.
3. Read [`../spec/00-overview.md`](../spec/00-overview.md).
4. Pick an issue labeled `good-first-issue`.

---

## Related Docs

- Release builds → [`release.md`](release.md)
- Code style → [`../conventions/coding-rust.md`](../conventions/coding-rust.md),
  [`../conventions/coding-typescript.md`](../conventions/coding-typescript.md)
- Testing → [`../nonfunctional/testing.md`](../nonfunctional/testing.md)
