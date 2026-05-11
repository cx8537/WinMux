# Naming Conventions

> File names, identifiers, and brand naming. Both Rust and TypeScript.

---

## Brand

- **Project name (display):** `WinMux`
- **Identifier (lowercase):** `winmux`
- **Korean spelling:** 윈먹스 (Wuhn-muhk-suh)
- **Pronunciation (English):** "win-mux"

The display name `WinMux` is used in:

- README headers and prose
- GUI strings
- Documentation
- Issue titles
- Marketing material (such as it is)

The lowercase identifier `winmux` is used in:

- Binary names (`winmux-server.exe`, `winmux-tray.exe`, `winmux.exe`)
- Crate names (`winmux-protocol`, `winmux-server`, …)
- npm package name (if ever published)
- Pipe name (`\\.\pipe\winmux-{user_sha8}`)
- Mutex name (`Local\WinMux-Server-{user_sha8}` — note the
  PascalCase here, which matches Windows convention for mutex
  names; this is the one exception)
- Registry value (`HKCU\...\Run` → value name `WinMux`)
- AppData folder (`%APPDATA%\winmux\`)
- GitHub repository (`cx8537/WinMux` — also PascalCase for
  user-facing visibility)

No other capitalizations. Not `Winmux`, not `winMux`, not `WINMUX`.

---

## Rust

| Kind | Convention | Example |
| --- | --- | --- |
| Module / file | `snake_case` | `pty_host.rs` |
| Type (struct, enum, trait) | `PascalCase` | `PtyHost`, `IpcMessage` |
| Function, method | `snake_case` | `spawn_powershell` |
| Constant (`const`, `static`) | `SCREAMING_SNAKE_CASE` | `DEFAULT_SCROLLBACK_LINES` |
| Local variable | `snake_case` | `pty_handle` |
| Type parameter | `PascalCase`, short | `T`, `K`, `Msg` |
| Lifetime | leading `'`, short lower | `'a`, `'pty` |

Examples:

```rust
pub struct PtyHost { /* ... */ }

pub fn spawn(shell: &str, rows: u16, cols: u16) -> Result<Pty> { /* ... */ }

const DEFAULT_SCROLLBACK_LINES: usize = 10_000;
```

clippy's `rustc::nonstandard_style` family catches most violations.

---

## TypeScript / React

| Kind | Convention | Example |
| --- | --- | --- |
| Component file | `PascalCase.tsx` | `TerminalPane.tsx` |
| Hook file | `useFoo.ts` | `usePrefixKey.ts` |
| Module file | `kebab-case.ts` | `server-client.ts` |
| Component | `PascalCase` | `TerminalPane` |
| Function | `camelCase` | `sendMessage` |
| Hook | `useCamelCase` | `usePrefixKey` |
| Variable | `camelCase` | `pendingMessages` |
| Constant (compile-time) | `SCREAMING_SNAKE_CASE` | `DEFAULT_TIMEOUT_MS` |
| Configuration object (not constant) | `camelCase` | `defaultTimeout` |
| Type, interface | `PascalCase` | `IpcMessage`, `SessionState` |
| Store | `useFooStore` | `useSessionsStore` |
| Branded type | `PascalCase` | `SessionId`, `PaneId` |
| Enum-like union | `kebab-case` strings | `'left' | 'right'` |

Examples:

```typescript
// src/components/TerminalPane.tsx
export function TerminalPane({ paneId }: { paneId: PaneId }) { /* ... */ }

// src/hooks/usePrefixKey.ts
export function usePrefixKey(): PrefixState { /* ... */ }

// src/lib/server-client.ts
export async function listSessions(): Promise<Result<Session[]>> { /* ... */ }

// src/store/sessions.ts
export const useSessionsStore = create<SessionsState>(/* ... */);

// constants
const DEFAULT_TIMEOUT_MS = 5_000;
const REGISTRY_KEY = 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run';
```

---

## IDs

WinMux uses several ID namespaces. Internally they are `u64` (Rust) /
branded `string` (TypeScript). Externally (in IPC messages, logs, and
the CLI) they are stringified with a short prefix:

| ID | Prefix | Example |
| --- | --- | --- |
| Session | `ses-` | `ses-01HKJ4Z6PXA7G3M2F9XQ7VWERT` |
| Window | `win-` | `win-01HKJ4Z6PYW...` |
| Pane | `pane-` | `pane-01HKJ4Z6PZ...` |
| Client | `cli-` | `cli-01HKJ4Z6Q0...` |
| Message correlation | `msg-` | `msg-01HKJ4Z6Q1...` |

After the prefix, IDs are ULIDs (sortable, 26 chars). Display names
(`work`, `deploy`, …) are user-facing labels, not IDs.

---

## File Layout Conventions

### Rust workspace

```
crates/
  winmux-protocol/
    src/
      lib.rs
      messages.rs           # IPC message enum
      version.rs            # protocol version constants
      types.rs              # shared types (SessionId, PaneId, ...)
      errors.rs             # protocol-level errors
  winmux-server/
    src/
      main.rs
      lib.rs
      pty.rs
      pty/
        conpty.rs
        job.rs
      terminal.rs           # alacritty_terminal wrapper
      scrollback.rs
      session.rs
      ipc/
        pipe_server.rs
        dispatcher.rs
      config.rs
      audit.rs
      registry.rs           # HKCU autostart
  winmux-tray/
    src/
      main.rs               # Tauri entry, glue
      ipc.rs                # client side of Named Pipe
      commands.rs           # Tauri commands exposed to frontend
  winmux-cli/
    src/
      main.rs
      args.rs
      commands/
        attach.rs
        ls.rs
        new_session.rs
        # ...
```

### Frontend

```
src/
  main.tsx                  # React entry
  App.tsx
  components/
    TerminalPane.tsx
    PanelLayout.tsx
    SessionList.tsx
    modals/
      Settings.tsx
      NewSession.tsx
  hooks/
    usePrefixKey.ts
    useTerminal.ts
  store/
    sessions.ts
    panels.ts
    settings.ts
  lib/
    server-client.ts
    protocol.ts
    logger.ts
    keymap.ts
  locales/
    en.json
    ko.json
  styles/
    globals.css
```

---

## Test Naming

### Rust

`test_<unit>_<scenario>_<expected>`, snake_case, descriptive:

```rust
#[test]
fn test_session_serializer_roundtrip_preserves_pane_layout() { /* ... */ }

#[tokio::test]
async fn test_pipe_server_rejects_other_user_sid() { /* ... */ }
```

### TypeScript

Vitest `describe`/`it` blocks, with English narrative:

```typescript
describe('protocol codec', () => {
  it('round-trips a NewSession message', () => { /* ... */ });
  it('rejects a message larger than 16 MiB', () => { /* ... */ });
});
```

---

## Avoid These

- Abbreviations beyond well-known ones. `PtyHost` is fine; `MsgDsptchr`
  is not.
- Hungarian notation. `lpszName`, `nCount`. Not in 2026.
- Boolean names without a verb. `available` becomes `isAvailable` or
  `hasAvailability`.
- Numbered suffixes on similar things (`session2`, `Manager2`). If you
  need a second one, the names are wrong.
- Trailing underscores or other private-marking conventions. Use
  module visibility instead.
