// IME(Input Method Editor) 명시적 composition 핸들러.
//
// spec § 04-key-handling §§ 191-198, § nonfunctional/accessibility.md §§ 78-130.
//
// 책임:
// 1. xterm.js `term.textarea`에서 compositionstart / compositionupdate /
//    compositionend 이벤트를 청취하고 외부 콜백으로 노출한다.
// 2. composition 중인지 여부를 동기적으로 조회할 수 있는 isComposing()을
//    제공한다 — keyboard 매니저가 prefix 처리 흐름에서 활용한다.
// 3. 합성 중인 텍스트를 표시할 자체 overlay를 렌더한다. xterm.js 내장
//    `.composition-view`는 PTY cursor 셀에 고정되는데, WebView2 + DPI
//    스케일 환경에서 셀폭 측정 오차가 누적되어 한 칸 어긋나 보이는
//    문제가 있었다 (`docs/decisions.md` 후보). 자체 overlay는 xterm가
//    움직여 주는 `xterm-helper-textarea`의 `getBoundingClientRect()`에
//    앵커링하므로 같은 렌더러 좌표계를 그대로 따라간다.
// 4. cancelComposition()으로 진행 중인 합성을 중단할 수 있게 한다 —
//    prefix(Ctrl+B)가 composition 중에 들어왔을 때 keyboard 매니저가
//    호출한다 (spec § 04 §§ 183-185).
//
// 본 모듈은 IPC / xterm.js / React 의존성을 갖지 않는다. 외부와는
// 콜백·접근자로만 통신하므로 단위 테스트가 가능하다.
//
// 데이터 흐름 메모: xterm.js는 compositionend 시점에 자체 코어 서비스로
// 합성 문자열을 onData에 흘려보낸다. 즉 PtyInput 송신 책임은 여전히
// `term.onData` 콜백에 있다 — 본 매니저의 onCommit은 디버깅 / 향후
// "직접 전송" 경로 전환을 위해 노출만 해 두고, PaneView는 onData를
// 그대로 사용한다.

import { logger } from '@/lib/logger';

/** ImeManager 생성 옵션. */
export interface ImeManagerOptions {
  /** xterm.js 가 만들어 둔 input textarea. compositionstart 등 native
   *  이벤트가 발생하는 실제 노드. */
  readonly textarea: HTMLTextAreaElement;
  /** overlay div를 붙일 부모. 일반적으로 `term.element` (xterm 루트). */
  readonly overlayParent: HTMLElement;
  /** xterm.js와 동일한 font-family 문자열. overlay에 같은 글리프
   *  메트릭이 적용되어야 셀 경계와 어긋나지 않는다. */
  readonly fontFamily: string;
  /** xterm.js의 fontSize(px). overlay 가독성 일관성 확보. */
  readonly fontSize: number;
  /** 합성 시작 알림. 외부(예: status indicator)에서 활용. */
  readonly onCompositionStart?: () => void;
  /** 합성 진행 중 텍스트 갱신. */
  readonly onCompositionUpdate?: (data: string) => void;
  /** 합성 종료. `composed`는 최종 확정된 문자열(빈 문자열 가능).
   *
   *  주의: 본 콜백은 PtyInput 전송 책임을 지지 않는다. xterm.js가 같은
   *  타이밍에 onData로 같은 문자열을 흘리기 때문이다. PtyInput 전송은
   *  onData 핸들러가 단독으로 책임진다. onCompositionEnd는 디버깅 /
   *  메트릭 / 미래 우회 경로용 훅으로만 사용한다. */
  readonly onCompositionEnd?: (composed: string) => void;
}

/** 외부 공개 표면. */
export interface ImeManager {
  /** 현재 합성 진행 중인지. native KeyboardEvent.isComposing은 keydown
   *  이벤트에 한해서만 신뢰할 수 있으므로, keyboard 매니저는 본 함수를
   *  매 keydown마다 호출해 정확한 상태를 보장한다. */
  readonly isComposing: () => boolean;
  /** 진행 중인 합성을 즉시 중단한다. prefix가 합성 도중에 눌렸을 때
   *  호출한다. blur → focus 패턴으로 OS IME에 cancel을 전달한다 —
   *  textarea.value를 비우는 방식은 IME 내부 상태를 망가뜨릴 수 있다. */
  readonly cancelComposition: () => void;
  /** 청취자 해제 및 overlay 노드 제거. */
  readonly dispose: () => void;
}

/**
 * xterm.js가 동일 textarea에 자체적으로 등록한 composition 핸들러는
 * 그대로 두고, 본 매니저는 그 위에 “관측자 + 외부 표면” 한 겹을 더
 * 얹는다. 동일 이벤트에 두 listener가 붙어도 충돌이 없도록 어떤
 * preventDefault / stopPropagation도 수행하지 않는다.
 */
export function createImeManager(opts: ImeManagerOptions): ImeManager {
  let composing = false;
  let disposed = false;
  // overlay: 합성 중 텍스트를 표시한다. xterm 내장 `.composition-view`는
  // CSS로 숨겨 두므로(아래 style block 참고) 본 노드만 보이게 된다.
  const overlay = document.createElement('div');
  overlay.className = 'winmux-ime-overlay';
  overlay.setAttribute('aria-hidden', 'true');
  applyOverlayBaseStyle(overlay, opts.fontFamily, opts.fontSize);
  opts.overlayParent.appendChild(overlay);

  // xterm 내장 `.composition-view`는 셀 좌표 계산 오차로 어긋날 수 있어
  // 같은 부모에서 숨긴다. 자체 overlay 하나만 사용한다는 의도를 코드로
  // 명시 — 전역 CSS를 건드리지 않고 인스턴스별로 한정.
  hideXtermBuiltInCompositionView(opts.overlayParent);

  function showOverlay(text: string): void {
    overlay.textContent = text;
    if (text.length === 0) {
      overlay.style.display = 'none';
      return;
    }
    // textarea가 xterm CompositionHelper.updateCompositionElements에
    // 의해 PTY cursor 셀로 이동된 후의 rect를 기준 좌표로 삼는다. 같은
    // 렌더러 좌표계라 셀 경계와 정확히 맞는다.
    const taRect = opts.textarea.getBoundingClientRect();
    const parentRect = opts.overlayParent.getBoundingClientRect();
    overlay.style.display = 'block';
    overlay.style.left = `${taRect.left - parentRect.left}px`;
    overlay.style.top = `${taRect.top - parentRect.top}px`;
  }

  function hideOverlay(): void {
    overlay.textContent = '';
    overlay.style.display = 'none';
  }

  function onStart(): void {
    composing = true;
    hideOverlay();
    opts.onCompositionStart?.();
  }

  function onUpdate(ev: CompositionEvent): void {
    // ev.data는 누적된 진행 중 문자열. (Chromium/Edge 둘 다 동일.)
    const text = typeof ev.data === 'string' ? ev.data : '';
    showOverlay(text);
    opts.onCompositionUpdate?.(text);
  }

  function onEnd(ev: CompositionEvent): void {
    composing = false;
    hideOverlay();
    const composed = typeof ev.data === 'string' ? ev.data : '';
    // 메타데이터만 로깅 — spec § Logging 규칙. 내용은 길이만 본다.
    logger.info('ime.composition.end', { length: composed.length });
    opts.onCompositionEnd?.(composed);
  }

  opts.textarea.addEventListener('compositionstart', onStart);
  opts.textarea.addEventListener('compositionupdate', onUpdate as EventListener);
  opts.textarea.addEventListener('compositionend', onEnd as EventListener);

  function isComposing(): boolean {
    return composing;
  }

  function cancelComposition(): void {
    if (!composing) return;
    // blur → focus 시 브라우저 / OS IME가 진행 중 composition을
    // 자동으로 commit 또는 abort하고 compositionend를 emit한다. 정책은
    // OS · IME에 따라 다르지만 어느 쪽이든 합성 상태는 안전하게 풀린다.
    const ta = opts.textarea;
    try {
      ta.blur();
      ta.focus();
    } catch (e) {
      logger.warn('ime.cancel.focus_failed', { error: String(e) });
    }
    // 만약 어떤 IME가 compositionend를 emit하지 않더라도 내부 플래그는
    // 강제로 내려 둔다 — keyboard 매니저는 plain key로 해석할 수 있어야
    // 한다.
    composing = false;
    hideOverlay();
  }

  function dispose(): void {
    if (disposed) return;
    disposed = true;
    opts.textarea.removeEventListener('compositionstart', onStart);
    opts.textarea.removeEventListener('compositionupdate', onUpdate as EventListener);
    opts.textarea.removeEventListener('compositionend', onEnd as EventListener);
    if (overlay.parentNode) {
      overlay.parentNode.removeChild(overlay);
    }
    composing = false;
  }

  return { isComposing, cancelComposition, dispose };
}

/** overlay div에 1회성 인라인 스타일을 부여한다. xterm 폰트와 동일한
 *  메트릭을 적용해 셀 경계와 한 칸 어긋나지 않도록 한다. */
function applyOverlayBaseStyle(el: HTMLDivElement, fontFamily: string, fontSize: number): void {
  const s = el.style;
  s.position = 'absolute';
  s.left = '0px';
  s.top = '0px';
  s.zIndex = '6'; // xterm-helpers(z-index 5) 위. canvas는 그 아래.
  s.pointerEvents = 'none';
  s.whiteSpace = 'nowrap';
  s.padding = '0';
  s.margin = '0';
  s.border = '0';
  s.background = '#000';
  s.color = '#FFF';
  s.fontFamily = fontFamily;
  s.fontSize = `${fontSize}px`;
  s.lineHeight = `${fontSize + 4}px`;
  s.display = 'none';
}

/** 같은 부모 안에 있는 xterm.js 내장 `.composition-view`를 숨긴다.
 *
 *  글로벌 CSS를 수정하면 모든 인스턴스에 영향을 주고 호환성 위험이 커진다.
 *  여기선 본 매니저가 다루는 패널에 한해서만 인라인 `display: none`을 건다.
 *  매니저 dispose에서 복구하지 않는 이유: PaneView는 매니저 dispose와 동시에
 *  term.dispose()로 DOM 전체를 정리하므로 잔존하지 않는다.
 *
 *  `instanceof HTMLElement` 대신 duck-typing(style 속성 존재 확인)을 쓰는
 *  이유: vitest 단위 테스트는 node 환경이라 `HTMLElement` 전역이 없다. */
function hideXtermBuiltInCompositionView(root: HTMLElement): void {
  const view: unknown = root.querySelector('.composition-view');
  if (
    view !== null &&
    typeof view === 'object' &&
    'style' in view &&
    typeof (view as { style: unknown }).style === 'object'
  ) {
    (view as HTMLElement).style.display = 'none';
  }
}
