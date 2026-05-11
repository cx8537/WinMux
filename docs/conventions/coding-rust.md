# Rust Coding Conventions

> Rules for Rust code in WinMux. The bedrock is rustfmt + clippy at
> deny-warnings; this doc covers things they cannot enforce.

---

## Toolchain

- **Edition:** 2024.
- **Channel:** stable. Pinned via `rust-toolchain.toml`.
- **MSRV:** the pinned stable version. We do not chase a lower MSRV.

---

## Formatting

`rustfmt` is the source of truth. Project settings live in
`rustfmt.toml`:

```toml
edition = "2024"
max_width = 100
use_field_init_shorthand = true
use_try_shorthand = true
imports_granularity = "Crate"
group_imports = "StdExternalCrate"
```

Run `cargo fmt --all` before every commit. CI rejects unformatted
code.

---

## Lints

`Cargo.toml` carries workspace lint tables:

```toml
[workspace.lints.rust]
unsafe_code = "deny"
missing_docs = "warn"

[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "warn"
panic = "warn"
todo = "warn"
unimplemented = "warn"
dbg_macro = "deny"
print_stdout = "deny"
print_stderr = "deny"
```

CI runs `cargo clippy --workspace --all-targets -- -D warnings`.

`pedantic` is **not** enabled; it is too noisy for a one-person
project.

### Allow lists are explicit

Need to override? Use `#[allow(clippy::xxx)]` at the smallest possible
scope, with a one-line comment explaining why:

```rust
// SAFETY checked: pane id always exists at this point.
#[allow(clippy::unwrap_used)]
let pane = registry.get(&pane_id).unwrap();
```

In tests, you can apply `#![allow(clippy::unwrap_used)]` at the module
level. Don't litter test code with attributes.

---

## Error Handling

### Library code (`crates/winmux-protocol`, internal lib modules)

Use `thiserror` to define a focused error enum:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("failed to create ConPTY (rows={rows}, cols={cols})")]
    CreatePty {
        rows: u16,
        cols: u16,
        #[source]
        source: std::io::Error,
    },

    #[error("PTY child exited with code {0}")]
    ChildExited(i32),
}
```

Rules:

- One enum per logical domain.
- Always carry context (which session, which pane, which file).
- Use `#[source]` for the underlying cause.
- Don't `impl From<std::io::Error>` blindly; convert at known points
  so the context can be added.

### Application code (`winmux-server` binary, `winmux-tray`, `winmux-cli`)

Use `anyhow::Result` with `.context()`:

```rust
use anyhow::{Context, Result};

fn start_server() -> Result<()> {
    let config = load_config()
        .context("failed to load winmux.toml")?;
    let pipe = create_pipe(&config.username)
        .context("failed to create named pipe")?;
    // ...
    Ok(())
}
```

Rules:

- `anyhow` only at binary crate boundaries (and in `main`).
- Every `?` should have meaningful context above it, either from the
  callee's error type or from a `.context()` call.
- Don't `.unwrap()` on `Result`. Don't `.expect()` either, except for
  invariants that hold at compile time. Always add a comment.

### Panics

- **Library code:** never panic on input. Return `Result`.
- **Application code:** panic only for programmer-invariant
  violations (`unreachable!()` after exhaustive match where the
  compiler cannot prove exhaustiveness).
- `todo!()` and `unimplemented!()` must not survive merge. Use
  `tracing::warn!` and `return Err(...)` for "not yet implemented"
  paths.

### `panic = "abort"`

Release builds set `panic = "abort"`. We do not use `catch_unwind`.
A panic ends the process; the panic hook writes a crash log and the
Job Object cleans up child shells.

---

## Async

### Runtime

One Tokio multi-threaded runtime per process.

```toml
tokio = {
    version = "1.40",
    default-features = false,
    features = ["rt-multi-thread", "macros", "sync", "io-util", "fs", "time"]
}
```

Specific features only. `tokio = { features = ["full"] }` is banned.

### Spawning

- `tokio::spawn` for async tasks.
- `tokio::task::spawn_blocking` for genuinely blocking work (rare).
- **Every `spawn` must have a place that holds the `JoinHandle` or
  uses `JoinSet`.** Fire-and-forget is banned: errors get swallowed
  and we never know.

```rust
let handle = tokio::spawn(async move {
    // ...
});
// stored somewhere that joins or aborts it on shutdown
```

### Locks

- **Async lock:** `tokio::sync::Mutex` or `tokio::sync::RwLock`.
  Use these when the critical section may `.await`.
- **Sync lock:** `std::sync::Mutex` for short, synchronous critical
  sections. Never `.await` while holding it.
- Prefer **channels** (`tokio::sync::mpsc`, `watch`, `broadcast`) over
  shared mutable state when feasible.

### Channels

Bounded only. `unbounded_channel` is banned:

```rust
let (tx, mut rx) = tokio::sync::mpsc::channel::<Msg>(256);
```

Pick channel capacities deliberately:

- Control plane (requests, replies): 64.
- Data plane (`PtyOutput` broadcast): 256.
- Document the choice in a comment.

When a sender hits capacity, decide explicitly: drop (with a warn
log), block (rare), or disconnect the slow consumer.

### Cancellation safety

`tokio::select!` cancels the loser. Some futures are not
cancel-safe. If you `select!` over a non-cancel-safe future, comment
about it and use `Box::pin` + manual polling instead. Read the docs
for any future before putting it in a `select!`.

---

## Types

### Newtype IDs

WinMux is full of IDs (sessions, windows, panes, clients,
messages…). Wrap each in a `pub struct Foo(u64)` newtype to make
parameter mix-ups impossible at compile time:

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SessionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PaneId(pub u64);
```

A function that moves a pane should take `from: PaneId, to: PaneId`,
not `from: u64, to: u64`.

### Enums for state

Two booleans usually want to be an enum. Three booleans always do.

```rust
// no
struct Pane { is_active: bool, is_zoomed: bool, is_dead: bool }

// yes
enum PaneState { Active, Inactive, Zoomed, Dead }
```

### `#[non_exhaustive]`

Apply to public enums and structs that may grow new variants/fields:

```rust
#[non_exhaustive]
pub enum CommandResult { Ok, NotFound, PermissionDenied }
```

### Borrow over own

Function parameters: `&str`, `&Path`, `&[T]` by default. Own only when
the function genuinely needs ownership.

### `impl Trait` in signatures

Fine for return types where the concrete type is internal. Avoid
heavy use in parameter position; it can make signatures hard to read.

---

## Module Structure

Use **filename style** (no `mod.rs`):

```
src/
  pty.rs
  pty/
    conpty.rs
    job_handle.rs
  ipc.rs
  ipc/
    pipe_server.rs
    dispatcher.rs
```

`mod.rs` style is deprecated for new code in this project.

### Visibility

- Default to `pub(crate)` for items shared within a crate.
- Use `pub` only for items meant to be consumed from another crate
  (mostly in `winmux-protocol`).
- Avoid `pub(super)` unless it genuinely helps.

### Re-exports

Top-level `lib.rs` re-exports the small set of types most consumers
need. Glob re-exports (`pub use foo::*`) are discouraged: they make
it hard to see what's actually exported.

---

## Comments and Docs

### Doc comments

Required on every `pub` item. Use the standard sections:

```rust
/// Spawns a new shell process backed by a ConPTY.
///
/// The returned `Pty` owns both the HPCON and the child process.
/// Dropping it sends `CTRL_BREAK_EVENT` and waits 5 seconds before
/// `TerminateProcess`.
///
/// # Errors
///
/// Returns [`PtyError::CreatePty`] if `CreatePseudoConsole` fails,
/// or [`PtyError::Spawn`] if the child process cannot start.
///
/// # Examples
///
/// ```no_run
/// # use winmux_server::pty::Pty;
/// let pty = Pty::spawn("pwsh", 40, 120)?;
/// # Ok::<(), winmux_server::pty::PtyError>(())
/// ```
pub fn spawn(shell: &str, rows: u16, cols: u16) -> Result<Pty, PtyError> {
    // ...
}
```

For non-`pub` items, write a doc comment when the "why" is not
obvious from the code.

### Inline comments

Comments should explain **why**, not **what**. The code already says
what.

Special prefixes:

- `// TODO(#42): ...` — must include an issue number.
- `// FIXME(#42): ...` — same.
- `// SAFETY: ...` — required on every `unsafe` block.
- `// SECURITY: ...` — for non-obvious security-relevant choices.

---

## Testing

See [`../nonfunctional/testing.md`](../nonfunctional/testing.md) for
strategy. For style:

- Tests live next to the code they test (`#[cfg(test)] mod tests` at
  the bottom of the file) for unit tests, or in `tests/` for
  integration tests.
- Test functions use descriptive snake_case:
  `test_session_serializer_roundtrip_preserves_layout`.
- `#[tokio::test]` for async tests.
- Real PTYs, real Named Pipes, real temp dirs. No mocking.
- `unwrap` / `expect` are allowed in test code; lint is opted out at
  module level.

---

## Dependencies

Adding a new crate requires:

1. A discussion with the user (or an open issue).
2. A short rationale in the PR: what it does, alternatives considered,
   maintenance health (last release, issue tracker activity).
3. Pin to a minor version (`tokio = "1.40"`), not `*`.
4. Disable default features and opt in to what we need.

Updating crates is fine without ceremony unless it's a major version
bump.

---

## Performance

See [`../nonfunctional/performance.md`](../nonfunctional/performance.md)
for SLOs. For style:

- **No speculative optimization.** "This might be slow" → measure
  first.
- Optimize algorithms before micro-tuning.
- Hot paths get a `// HOT PATH:` comment so the next maintainer knows
  to be careful.
- Use `criterion` for any measurement you cite in a PR description.
