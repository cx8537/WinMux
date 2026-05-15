// WinMux 프로토콜 v1의 TypeScript 거울.
//
// Rust `winmux-protocol` 크레이트와 1:1 매칭되어야 한다.
// 와이어 정의는 `docs/spec/01-ipc-protocol.md`.

// ---------------------------------------------------------------------------
// 브랜드 ID (Rust newtype 미러)
// ---------------------------------------------------------------------------

export type SessionId = string & { readonly __brand: 'SessionId' };
export type WindowId = string & { readonly __brand: 'WindowId' };
export type PaneId = string & { readonly __brand: 'PaneId' };
export type MessageId = string & { readonly __brand: 'MessageId' };

/** 와이어에서 받은 raw 문자열을 PaneId로 좁힌다. 검증은 호출자 책임. */
export function paneId(raw: string): PaneId {
  return raw as PaneId;
}

/** 와이어에서 받은 raw 문자열을 SessionId로 좁힌다. */
export function sessionId(raw: string): SessionId {
  return raw as SessionId;
}

/** 와이어에서 받은 raw 문자열을 WindowId로 좁힌다. */
export function windowId(raw: string): WindowId {
  return raw as WindowId;
}

// ---------------------------------------------------------------------------
// 페이로드 타입
// ---------------------------------------------------------------------------

export interface PaneSize {
  rows: number;
  cols: number;
}

export interface SessionSummary {
  id: SessionId;
  name: string;
  created_at: string;
  windows: number;
  attached_clients: number;
}

export interface WindowSummary {
  id: WindowId;
  index: number;
  name?: string;
  active_pane: PaneId;
}

export interface PaneSummary {
  id: PaneId;
  size: PaneSize;
  index: number;
  title?: string;
  alive: boolean;
}

export interface PaneSnapshot {
  pane_id: PaneId;
  bytes_base64: string;
}

export interface AttachOutcome {
  session_id: SessionId;
  active_window: WindowId;
  windows: WindowSummary[];
  panes: PaneSummary[];
  initial_snapshots: PaneSnapshot[];
}

// ---------------------------------------------------------------------------
// Tauri command 인자 모양
// ---------------------------------------------------------------------------

export interface NewSessionArgs {
  name?: string;
  shell?: string;
  cwd?: string;
  env?: Record<string, string>;
  detached?: boolean;
}

export type SessionRef =
  | { kind: 'name'; name: string }
  | { kind: 'id'; id: SessionId };

export interface AttachArgs {
  session: SessionRef;
  rows: number;
  cols: number;
}

// ---------------------------------------------------------------------------
// 푸시 이벤트 페이로드
// ---------------------------------------------------------------------------

/** `server:status` 이벤트가 webview에 emit하는 페이로드. */
export type ServerStatus =
  | { state: 'connecting' }
  | { state: 'connected'; server_version: string; user: string }
  | { state: 'disconnected'; reason: string };

/** `pty:output` 이벤트가 webview에 emit하는 페이로드. */
export interface PtyOutputPayload {
  pane_id: PaneId;
  bytes_base64: string;
}

/** `server:event` 이벤트가 webview에 emit하는 페이로드. */
export type EventPayload =
  | { type: 'PaneExited'; v: number; pane_id: PaneId; exit_code: number }
  | { type: 'WindowClosed'; v: number; window_id: WindowId }
  | { type: 'SessionRenamed'; v: number; session_id: SessionId; name: string }
  | { type: 'PaneTitleChanged'; v: number; pane_id: PaneId; title: string }
  | { type: 'AlertBell'; v: number; pane_id: PaneId };

// ---------------------------------------------------------------------------
// Tauri command 오류
// ---------------------------------------------------------------------------

export interface CommandError {
  message: string;
  code?: string;
  recoverable: boolean;
}

/** alien object가 CommandError 모양인지 확인하는 type guard. */
export function isCommandError(value: unknown): value is CommandError {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return typeof v.message === 'string' && typeof v.recoverable === 'boolean';
}
