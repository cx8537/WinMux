# TypeScript Coding Conventions

> Rules for TypeScript and React code in WinMux's frontend (`src/`,
> consumed by `winmux-tray`).

---

## Toolchain

- **TypeScript:** 5.x latest.
- **Node:** 20+.
- **Package manager:** `npm`.
- **Build:** Vite.
- **Framework:** React 19, function components only.
- **State:** Zustand. No Redux, no MobX.
- **Styling:** Tailwind + shadcn/ui. No CSS-in-JS runtime libraries.
- **i18n:** react-i18next.
- **Terminal:** xterm.js v6 with WebGL renderer.

---

## `tsconfig.json` essentials

```jsonc
{
  "compilerOptions": {
    "strict": true,
    "noUncheckedIndexedAccess": true,
    "noImplicitOverride": true,
    "noFallthroughCasesInSwitch": true,
    "exactOptionalPropertyTypes": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "forceConsistentCasingInFileNames": true,
    "isolatedModules": true,
    "moduleResolution": "Bundler",
    "target": "ES2022"
  }
}
```

`strict` and `noUncheckedIndexedAccess` are non-negotiable.
`array[index]` returns `T | undefined` and the code must reflect
that.

---

## ESLint

Key rules (`eslint.config.js`):

```javascript
{
  rules: {
    "@typescript-eslint/no-explicit-any": "error",
    "@typescript-eslint/no-unused-vars": "error",
    "@typescript-eslint/consistent-type-imports": "error",
    "@typescript-eslint/no-non-null-assertion": "error",
    "no-console": ["error", { "allow": ["warn", "error"] }],
    "eqeqeq": "error",
    "prefer-const": "error",
    "jsx-a11y/...": "warn"  // accessibility
  }
}
```

CI runs `npm run lint` and rejects warnings.

`any` is banned. Use `unknown` and narrow. `!.` non-null assertions
are banned. Use type guards.

---

## Prettier

`.prettierrc`:

```json
{
  "semi": true,
  "singleQuote": true,
  "trailingComma": "all",
  "printWidth": 100,
  "tabWidth": 2,
  "arrowParens": "always",
  "endOfLine": "lf"
}
```

Matches Rust's 100-char width for consistency.

---

## Error Handling

### `Result` pattern for predictable failures

WinMux uses a hand-rolled `Result<T, E>` for predictable failures
(invalid input, missing data, IPC errors):

```typescript
export type Result<T, E = Error> =
  | { ok: true; value: T }
  | { ok: false; error: E };

export function ok<T>(value: T): Result<T, never> {
  return { ok: true, value };
}

export function err<E>(error: E): Result<never, E> {
  return { ok: false, error };
}
```

Use `Result` for anything coming from outside the function (IPC,
parsing user input, reading files, fetch).

### `throw` only for true exceptions

`throw new Error(...)` is reserved for invariants the programmer
believes will hold:

```typescript
function getRequiredElement(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`element #${id} must exist`);
  return el;
}
```

### `console.log` is banned

Use the logger wrapper in `src/lib/logger.ts`:

```typescript
import { logger } from '@/lib/logger';

logger.info('session attached', { sessionId });
logger.warn('slow IPC response', { ms });
logger.error('failed to attach', { error });
```

In production, the logger forwards to the Tauri-side `tracing`
infrastructure. In dev it also prints to the browser console.

`console.warn` and `console.error` are allowed at top level only as a
fallback when the logger itself fails (rare).

---

## React Conventions

### Function components

Always:

```tsx
export function TerminalPane({ paneId }: { paneId: PaneId }) {
  // ...
}
```

No class components. No `React.FC` (it has caveats around children;
we type props explicitly).

### Hooks rules

- Custom hooks named `useFoo`.
- Hooks live in `src/hooks/` for shared ones, or inline at the top of
  a component file if only used there.
- Don't violate the rules of hooks (no conditional calls). ESLint
  enforces this.

### State management

- **Component-local state:** `useState`, `useReducer`.
- **Shared / app-wide state:** Zustand stores in `src/store/`.
- One store per concern (sessions, panels, settings, …). Do not
  create a single global store.

Zustand store template:

```typescript
import { create } from 'zustand';

interface SessionsState {
  sessions: Session[];
  attachedSessionId: SessionId | null;

  setSessions(list: Session[]): void;
  attach(id: SessionId): void;
  detach(): void;
}

export const useSessionsStore = create<SessionsState>((set) => ({
  sessions: [],
  attachedSessionId: null,

  setSessions: (list) => set({ sessions: list }),
  attach: (id) => set({ attachedSessionId: id }),
  detach: () => set({ attachedSessionId: null }),
}));
```

### Effects

- Keep effects small. One concern per `useEffect`.
- Always return a cleanup if the effect subscribes to anything.
- Avoid effects when a derived value would do (`useMemo` /
  computed selector).

### Tauri IPC

All Tauri calls go through `src/lib/server-client.ts`. Components do
not import `@tauri-apps/api` directly. This makes the IPC boundary
greppable and easier to mock for component tests if we ever need to.

```typescript
// src/lib/server-client.ts
import { invoke } from '@tauri-apps/api/core';

export async function listSessions(): Promise<Result<Session[]>> {
  try {
    const list = await invoke<Session[]>('list_sessions');
    return ok(list);
  } catch (e) {
    return err(new Error(`list_sessions failed: ${String(e)}`));
  }
}
```

---

## Types

### `interface` vs `type`

- `interface` for object shapes that may extend or be implemented.
- `type` for unions, intersections, mapped types, and aliases.

```typescript
interface Pane {
  id: PaneId;
  rows: number;
  cols: number;
}

type PaneDirection = 'left' | 'right' | 'up' | 'down';
```

### Branded types for IDs

Match Rust's newtype pattern:

```typescript
export type SessionId = string & { readonly __brand: 'SessionId' };
export type PaneId = string & { readonly __brand: 'PaneId' };

export function sessionId(s: string): SessionId {
  return s as SessionId;
}
```

This prevents passing a `PaneId` where a `SessionId` is expected.

### Don't put types in identifier names

```typescript
// no
const sessionArray: Session[] = [];
const userObj: User = ...;

// yes
const sessions: Session[] = [];
const user: User = ...;
```

---

## Imports

### Order

ESLint and Prettier handle import sorting. Logical groups:

1. Node built-ins (rare in this project).
2. External packages (`react`, `zustand`, …).
3. Tauri (`@tauri-apps/...`).
4. Aliased internal (`@/...`).
5. Relative (`./...`).

### `import type`

Required for type-only imports:

```typescript
import type { Session } from '@/lib/protocol';
import { listSessions } from '@/lib/server-client';
```

Enforced by `@typescript-eslint/consistent-type-imports`.

### Path alias

`@/` maps to `src/`. Set in `tsconfig.json` and Vite config. No
`../../../` relative climbs.

---

## Naming

- **Components:** `PascalCase`, file `PascalCase.tsx`.
- **Hooks:** `useFoo`, file `useFoo.ts`.
- **Other modules:** `kebab-case.ts` (`server-client.ts`,
  `protocol-codec.ts`).
- **Stores:** `useFooStore`, file `foo.ts` in `src/store/`.
- **Functions, variables:** `camelCase`.
- **Constants:** `SCREAMING_SNAKE_CASE` for true compile-time
  constants. `camelCase` for "configuration that happens to not
  change."
- **Types and interfaces:** `PascalCase`.

See [`naming.md`](naming.md) for the full table.

---

## Components

### Props

- Explicit prop types. No spread props except as a deliberate
  pattern.
- Children prop only when the component genuinely wraps content.
- Default values via destructuring, not `defaultProps`:

```tsx
export function Toast({ duration = 5000, children }: {
  duration?: number;
  children: ReactNode;
}) {
  // ...
}
```

### Styling

- Tailwind utilities first.
- `cn(...)` helper from shadcn/ui for conditional classes.
- For complex one-off styles, prefer a `<div className="...">` over
  a custom CSS module unless the styles are reused.

### Accessibility

- Every interactive element is reachable by keyboard.
- Use semantic HTML before ARIA. `<button>` not `<div role="button">`.
- shadcn/ui handles most ARIA for us; in custom components, verify.

---

## Testing

See [`../nonfunctional/testing.md`](../nonfunctional/testing.md). For
style:

- **Framework:** Vitest.
- **UI tests:** React Testing Library, sparingly. Most UI is verified
  manually.
- **Unit tests:** Logic in `src/lib/` and `src/store/` should be
  thoroughly unit-tested.
- Test files: `foo.test.ts` next to `foo.ts`.

---

## Performance

- React 19's automatic memoization helps; don't sprinkle `useMemo`
  /`useCallback` defensively.
- xterm.js: WebGL renderer enabled, with canvas fallback.
- Avoid re-renders by structuring stores so subscriptions are narrow.
- For terminal output, the React tree should not re-render per byte;
  xterm.js owns the canvas and we feed it through a ref.

---

## What's Banned

- `any`.
- `!.` non-null assertions.
- `console.log` (use `logger`).
- `as` casts except when narrowing from a safer base type.
- `Function` type. Use specific signatures.
- `Object` as a type. Use `Record<string, unknown>` or a defined
  interface.
- HTML `<form>` inside Tauri artifacts that submit (Tauri caveat
  documented in their guide).
- Inline `style={{ ... }}` for anything but truly dynamic values
  (computed transforms, etc.). Use Tailwind otherwise.
