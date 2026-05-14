// tmux-style prefix 키 상태기 + 키 바인딩.
//
// spec § 04-key-handling. tray(frontend)가 책임지는 영역으로, xterm.js의
// `attachCustomKeyEventHandler` 위에 얹는다. server는 prefix 키를 절대 보지
// 않으며, 매니저가 해석한 action(IPC) 또는 PTY 바이트만 받는다.
//
// 본 모듈은 React/Tauri/IPC 의존성을 갖지 않는다 — 외부와는 `onAction` /
// `onStateChange` 콜백으로만 통신해 순수 객체 단위 테스트가 가능하다.

import { logger } from '@/lib/logger';

/** 매니저가 발사하는 외부 action.
 *
 * M1.1 범위: detach(`Prefix d`)와 리터럴 prefix 전송(`Prefix Prefix`). 후속
 * 마일스톤에서 `c`/`n`/`p`/`%`/`"` 등이 이 union에 추가된다. */
export type KeyboardAction =
  | { readonly kind: 'detach' }
  | { readonly kind: 'send-bytes'; readonly bytes: Uint8Array };

/** 외부 인디케이터(StatusBar 등)가 표시하기 위한 가시 상태.
 *
 * `executing`은 spec에서 정의되지만 동기적 dispatch라 거의 보이지 않으므로
 * M1.1에서는 두 값만 사용한다. */
export type KeyboardState = 'idle' | 'awaiting';

/** 매니저 생성 옵션. */
export interface KeyboardManagerOptions {
  /** AwaitingCommand에 머무는 최대 시간. spec 기본 3000 ms. */
  readonly prefixTimeoutMs?: number;
  /** Action 트리거 콜백. 외부 dispatcher는 본 콜백에서 IPC를 실행한다. */
  readonly onAction: (action: KeyboardAction) => void;
  /** state 변화를 외부에 알려 indicator를 갱신하기 위한 옵션 콜백. */
  readonly onStateChange?: (state: KeyboardState) => void;
  /** 현재 IME composition 진행 중인지 알려 주는 동기 조회자.
   *
   *  spec § 04 §§ 183-185, § accessibility.md §§ 87-91: prefix는 keydown
   *  단계에서 IME보다 먼저 잡혀야 한다. 일반 키가 composition 도중에
   *  들어오면 매니저는 손대지 않고 xterm/IME 측에 넘긴다(true 반환). */
  readonly isComposing?: () => boolean;
  /** 합성 도중 prefix가 눌렸을 때 합성을 취소하기 위해 호출되는 훅.
   *  매니저는 호출 후 AwaitingCommand로 진입한다. */
  readonly onCancelComposition?: () => void;
}

/** 본 모듈의 공개 표면. */
export interface KeyboardManager {
  /** xterm.js의 `attachCustomKeyEventHandler`에 그대로 넘긴다.
   *
   * 반환값 의미는 xterm.js 규약과 같다. `true`이면 xterm.js가 키를
   * 정상 처리(즉 PTY로 흘러간다)하고, `false`이면 매니저가 키를 소비
   * 한다(예: prefix 자체, 명령 키). */
  readonly handle: (event: KeyboardEvent) => boolean;
  /** 현재 state. 테스트와 외부 indicator에서 사용. */
  readonly getState: () => KeyboardState;
  /** 보류 중인 timeout을 정리. unmount 시 호출. */
  readonly dispose: () => void;
}

/** spec § State Machine 기본값. */
const DEFAULT_TIMEOUT_MS = 3000;

/** Default prefix `Ctrl+B`의 PTY 리터럴 바이트. C-b == 0x02. */
const LITERAL_PREFIX_BYTE = 0x02;

function isPrefixKeyDown(e: KeyboardEvent): boolean {
  if (e.type !== 'keydown') return false;
  if (!e.ctrlKey) return false;
  if (e.shiftKey || e.altKey || e.metaKey) return false;
  // `event.key`는 layout/locale에 따라 'b' 또는 'B'가 될 수 있다. 소문자
  // 비교로 일관성 확보.
  return e.key.toLowerCase() === 'b';
}

function isModifierOnly(e: KeyboardEvent): boolean {
  return e.key === 'Control' || e.key === 'Shift' || e.key === 'Alt' || e.key === 'Meta';
}

/** spec § Custom xterm.js Key Handler 흐름을 구현한 매니저를 만든다. */
export function createKeyboardManager(opts: KeyboardManagerOptions): KeyboardManager {
  const timeoutMs = opts.prefixTimeoutMs ?? DEFAULT_TIMEOUT_MS;
  let state: KeyboardState = 'idle';
  let timer: ReturnType<typeof setTimeout> | null = null;

  function clearTimer(): void {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function transitionTo(next: KeyboardState): void {
    if (state === next) return;
    state = next;
    opts.onStateChange?.(next);
  }

  function enterAwaiting(): void {
    transitionTo('awaiting');
    clearTimer();
    timer = setTimeout(() => {
      timer = null;
      transitionTo('idle');
    }, timeoutMs);
  }

  function returnToIdle(): void {
    clearTimer();
    transitionTo('idle');
  }

  function handle(event: KeyboardEvent): boolean {
    // keypress/keyup, 단독 modifier는 매니저가 손대지 않는다.
    if (event.type !== 'keydown') return true;
    if (isModifierOnly(event)) return true;

    // IME composition 중에는 모든 일반 키 입력을 IME가 소화해야 한다 —
    // 자모 한 글자가 keydown 단위로 들어와도 합성 결과는 compositionend
    // 시점에 한 번에 들어온다. 예외: prefix(Ctrl+B). 합성을 취소하고
    // AwaitingCommand로 진입한다. (spec § 04 §§ 183-185.)
    const composing = opts.isComposing?.() === true;
    if (composing) {
      if (isPrefixKeyDown(event)) {
        opts.onCancelComposition?.();
        if (state !== 'awaiting') {
          enterAwaiting();
        }
        return false;
      }
      // 그 외엔 IME가 처리하도록 통과시킨다 — 매니저는 절대 합성 중
      // 키를 byte로 해석하지 않는다.
      return true;
    }

    if (state === 'idle') {
      if (isPrefixKeyDown(event)) {
        enterAwaiting();
        return false;
      }
      return true;
    }

    // state === 'awaiting'
    if (isPrefixKeyDown(event)) {
      // `Ctrl+B Ctrl+B` — 리터럴 prefix를 셸로 보낸다.
      returnToIdle();
      opts.onAction({
        kind: 'send-bytes',
        bytes: new Uint8Array([LITERAL_PREFIX_BYTE]),
      });
      return false;
    }

    // M1.1 prefix 테이블: `d` → detach.
    const noModifiers = !event.ctrlKey && !event.altKey && !event.metaKey;
    if (noModifiers && event.key.toLowerCase() === 'd') {
      returnToIdle();
      opts.onAction({ kind: 'detach' });
      return false;
    }

    // 알려지지 않은 키: spec에 따라 prefix를 취소하고 키는 버린다.
    returnToIdle();
    logger.info('keyboard.prefix.unmatched', { key: event.key });
    return false;
  }

  function getState(): KeyboardState {
    return state;
  }

  function dispose(): void {
    clearTimer();
    state = 'idle';
  }

  return { handle, getState, dispose };
}
