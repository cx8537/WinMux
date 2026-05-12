// Tauri 명령 / 이벤트 통합 어댑터.
//
// 모든 IPC 호출은 본 모듈을 통한다 — 컴포넌트는 직접 `@tauri-apps/api`를
// 임포트하지 않는다 (`docs/conventions/coding-typescript.md` § Tauri IPC).
// 본 모듈의 또 다른 역할은 Tauri 명령의 `invoke<T>(...)` 응답을 도메인
// 타입으로 좁히고, 푸시 이벤트의 listen에 타입 안전한 래퍼를 제공하는
// 것이다.

import { invoke } from '@tauri-apps/api/core';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { listen } from '@tauri-apps/api/event';

import { logger } from '@/lib/logger';
import type {
  AttachArgs,
  AttachOutcome,
  CommandError,
  EventPayload,
  NewSessionArgs,
  PaneId,
  PtyOutputPayload,
  ServerStatus,
  SessionRef,
  SessionSummary,
} from '@/lib/protocol';
import { isCommandError } from '@/lib/protocol';

/**
 * Tauri `invoke` 실패는 `unknown`으로 던져진다. 가능하면 [`CommandError`]로
 * 정규화하고, 아니면 메시지만 보존한다.
 */
function normalizeError(e: unknown): CommandError {
  if (isCommandError(e)) return e;
  if (e instanceof Error) {
    return { message: e.message, recoverable: true };
  }
  return { message: typeof e === 'string' ? e : JSON.stringify(e), recoverable: true };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    const err = normalizeError(e);
    logger.warn(`ipc.${cmd}.failed`, { error: err });
    throw err;
  }
}

/** 현재 IPC 상태 조회 — [`ServerStatus`]. */
export function getServerStatus(): Promise<ServerStatus> {
  return call<ServerStatus>('winmux_server_status');
}

/** 헬스체크 ping. */
export function ping(): Promise<void> {
  return call<void>('winmux_ping');
}

/** `ListSessions`. */
export function listSessions(): Promise<SessionSummary[]> {
  return call<SessionSummary[]>('winmux_list_sessions');
}

/**
 * `NewSession`. detached이면 `null`, 아니면 자동 어태치 결과를 돌려준다.
 */
export function newSession(args: NewSessionArgs): Promise<AttachOutcome | null> {
  return call<AttachOutcome | null>('winmux_new_session', { args });
}

/** `Attach`. */
export function attachSession(args: AttachArgs): Promise<AttachOutcome> {
  return call<AttachOutcome>('winmux_attach', { args });
}

/** `Detach`. */
export function detachSession(): Promise<void> {
  return call<void>('winmux_detach');
}

/** `KillSession`. */
export function killSession(session: SessionRef): Promise<void> {
  return call<void>('winmux_kill_session', { session });
}

/** `PtyInput` — fire and forget. */
export function ptyInput(pid: PaneId, bytesBase64: string): Promise<void> {
  return call<void>('winmux_pty_input', { paneId: pid, bytesBase64 });
}

/** `Resize` — 패널 크기 변경. */
export function resizePane(pid: PaneId, rows: number, cols: number): Promise<void> {
  return call<void>('winmux_resize', { paneId: pid, rows, cols });
}

// ---------------------------------------------------------------------------
// 푸시 이벤트 구독
// ---------------------------------------------------------------------------

/**
 * `pty:output` 이벤트 구독자. 반환값은 unsubscribe 함수.
 *
 * 호출자는 컴포넌트 unmount나 useEffect cleanup에서 호출해야 한다.
 */
export function onPtyOutput(cb: (p: PtyOutputPayload) => void): Promise<UnlistenFn> {
  return listen<PtyOutputPayload>('pty:output', (event) => cb(event.payload));
}

/** `server:status` 이벤트 구독자. */
export function onServerStatus(cb: (s: ServerStatus) => void): Promise<UnlistenFn> {
  return listen<ServerStatus>('server:status', (event) => cb(event.payload));
}

/** `server:event` 이벤트 구독자 (PaneExited, AlertBell 등). */
export function onServerEvent(cb: (e: EventPayload) => void): Promise<UnlistenFn> {
  return listen<EventPayload>('server:event', (event) => cb(event.payload));
}

/** `server:bye` 이벤트 구독자 — 서버가 곧 종료된다는 신호. */
export function onServerBye(cb: () => void): Promise<UnlistenFn> {
  return listen<null>('server:bye', () => cb());
}
