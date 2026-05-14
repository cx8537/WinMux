// 한 PTY 패널의 xterm.js 마운트.
//
// 책임:
// 1. xterm.js Terminal 인스턴스를 만들고 컨테이너에 연결.
// 2. WebGL 렌더러를 시도하고 실패하면 조용히 캔버스로 폴백
//    (`docs/spec/07-tray-and-gui.md` § Pane rendering).
// 3. `pty:output` 이벤트로 들어오는 바이트를 base64 해독 후 그대로 write.
// 4. xterm `onData`로 잡힌 사용자 입력을 UTF-8 → base64로 인코딩해
//    `winmux_pty_input` 명령으로 server에 보낸다.
// 5. ResizeObserver로 컨테이너 크기 변화를 감지해 FitAddon으로 새 rows/cols를
//    계산하고 `winmux_resize`를 호출.
// 6. `PaneExited` 이벤트가 도착하면 한 줄 텍스트로 알리고 종료 상태로 들어간다.
// 7. IME composition은 `createImeManager`에 위임한다. 매니저는 자체 overlay로
//    합성 중 텍스트를 그리고, keyboard 매니저에 composing 상태를 노출해
//    prefix(Ctrl+B) 가로채기와 충돌하지 않도록 한다 (spec § 04 §§ 191-198).

import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';

import type { UnlistenFn } from '@tauri-apps/api/event';
import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';

import { base64ToBytes, bytesToBase64, utf8ToBase64 } from '@/lib/codec';
import { createImeManager } from '@/lib/ime';
import type { ImeAnchorMode, ImeManager } from '@/lib/ime';
import { createKeyboardManager } from '@/lib/keyboard';
import { logger } from '@/lib/logger';
import type { PaneId } from '@/lib/protocol';
import {
  detachSession,
  onPtyOutput,
  onServerEvent,
  ptyInput,
  resizePane,
} from '@/lib/server-client';
import { useSessionStore } from '@/store/session';

/**
 * Windows Terminal "Campbell" 팔레트 (`docs/spec/07-tray-and-gui.md`
 * § Terminal palette). UI chrome 팔레트와는 독립된 namespace다.
 */
const CAMPBELL_THEME = {
  background: '#0C0C0C',
  foreground: '#CCCCCC',
  cursor: '#FFFFFF',
  cursorAccent: '#0C0C0C',
  selectionBackground: 'rgba(255, 255, 255, 0.25)',

  black: '#0C0C0C',
  red: '#C50F1F',
  green: '#13A10E',
  yellow: '#C19C00',
  blue: '#0037DA',
  magenta: '#881798',
  cyan: '#3A96DD',
  white: '#CCCCCC',

  brightBlack: '#767676',
  brightRed: '#E74856',
  brightGreen: '#16C60C',
  brightYellow: '#F9F1A5',
  brightBlue: '#3B78FF',
  brightMagenta: '#B4009E',
  brightCyan: '#61D6D6',
  brightWhite: '#F2F2F2',
} as const;

interface PaneViewProps {
  /** 본 패널의 식별자. 모든 IPC 호출에 그대로 실어 보낸다. */
  paneId: PaneId;
  /**
   * 어태치 응답에 함께 들어온 초기 화면 스냅샷(base64). 마운트 직후 한 번
   * `term.write` 되어 reattach 시 직전 화면이 복원된다. `undefined`면
   * 빈 화면으로 시작 (새 세션 첫 attach의 경우 — 그래도 ConPTY가 곧
   * banner를 보낸다).
   */
  initialSnapshotBase64?: string | undefined;
}

export function PaneView({ paneId, initialSnapshotBase64 }: PaneViewProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const setAttached = useSessionStore((s) => s.setAttached);
  const setPrefixActive = useSessionStore((s) => s.setPrefixActive);
  // i18n `t` 변경으로 effect가 재실행되지 않도록 ref로 빼서 항상 최신값만 본다.
  const tRef = useRef(t);
  useEffect(() => {
    tRef.current = t;
  }, [t]);
  // IME overlay anchor 모드. 서버의 PaneCursorVisibility 이벤트로 갱신된다.
  // - PTY cursor 가시(기본 셸): helper-textarea(=PTY cursor 셀) 추적.
  // - PTY cursor 숨김(ESC[?25l; claude/lazygit/htop/btop 같은 TUI):
  //   helper-textarea가 의미 없는 위치에 머무르므로 패널 bottom-left로 전환.
  // baseline은 'textarea' — 서버도 같은 baseline에서 시작해 전이가 일어날
  // 때만 이벤트를 발사한다.
  const imeAnchorRef = useRef<ImeAnchorMode>('textarea');

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    // 한글 등 CJK 글리프는 라틴 폰트에 없으므로 spec § accessibility.md
    // §118의 폴백 체인을 따라간다. CJK 폰트의 advance가 라틴 셀 너비와
    // 맞아야 IME composition이 셀 경계 밖으로 삐져 보이지 않는다.
    const fontFamily =
      '"Cascadia Code", "Cascadia Mono", "Consolas", "D2Coding", "Malgun Gothic", "Noto Sans Mono CJK KR", monospace';
    const fontSize = 14;

    const term = new Terminal({
      fontFamily,
      fontSize,
      cursorBlink: true,
      theme: CAMPBELL_THEME,
      scrollback: 10_000,
      convertEol: false,
      allowProposedApi: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    term.open(container);

    // WebGL renderer는 페인트 성능상 기본 활성화. xterm 내장
    // `.composition-view`의 cellWidth skew는 자체 IME overlay(아래)가
    // PTY cursor 셀이 아닌 helper-textarea 좌표를 따라가도록 설계되어 있어
    // composition 위치에 영향을 주지 않는다.
    try {
      const webgl = new WebglAddon();
      term.loadAddon(webgl);
    } catch (e) {
      // WebGL 사용이 불가능하면 캔버스 렌더러로 자연스럽게 폴백된다.
      logger.warn('xterm.webgl.unavailable', { error: String(e) });
    }

    try {
      fit.fit();
    } catch (e) {
      logger.warn('xterm.fit.initial_failed', { error: String(e) });
    }
    // 초기 스냅샷이 있으면 fit 직후·resize 호출 전에 그린다. 이렇게 해야
    // (a) 잠시 빈 화면이 보이는 깜박임이 없고
    // (b) 곧이어 발생하는 ConPTY resize가 wrap을 다시 계산하도록 한다.
    // 스냅샷 자체는 ESC[2J + 셀 글리프 + cursor 이동이라 wrap을 임의로
    // 다시 그려도 글리프 정합성은 무너지지 않는다.
    if (initialSnapshotBase64) {
      try {
        const snap = base64ToBytes(initialSnapshotBase64);
        term.write(snap);
      } catch (e) {
        logger.warn('pane.initial_snapshot.decode_failed', { error: String(e) });
      }
    }
    // 서버는 NewSession 시 임시 40×120으로 PTY를 만든다. fit으로 계산한
    // 실제 사이즈를 즉시 전송해 ConPTY가 wrap을 다시 그리게 한다.
    void resizePane(paneId, term.rows, term.cols);

    // 사용자 입력 → server.
    const dataDisposable = term.onData((data) => {
      void ptyInput(paneId, utf8ToBase64(data));
    });

    // IME composition 매니저. xterm.js가 노출한 textarea에 직접 listener를
    // 걸고, 자체 overlay로 합성 중 텍스트를 그린다. xterm 내장
    // `.composition-view`는 PTY cursor 셀에 고정되는데 WebView2 + DPI
    // 환경에서 측정 오차로 한 칸 어긋나 보이는 문제가 있어, 같은 좌표계를
    // 따라가는 자체 overlay로 대체한다(spec § 04 §§ 191-198).
    //
    // TUI 앱(`ESC[?25l`로 PTY cursor 숨김)에 대비해 anchor 모드를 ref에서
    // 매번 읽어 온다. ref는 PaneCursorVisibility 이벤트로 토글되며, mount
    // 시점의 기본값은 'textarea'다.
    let ime: ImeManager | null = null;
    if (term.element && term.textarea) {
      ime = createImeManager({
        textarea: term.textarea,
        overlayParent: term.element,
        fontFamily,
        fontSize,
        getAnchorMode: () => imeAnchorRef.current,
      });
    } else {
      logger.warn('pane.ime.disabled', {
        reason: 'term.element or term.textarea is undefined after open()',
      });
    }
    // 새 PaneRuntime마다 baseline은 'textarea'로 초기화한다. 이전 attach
    // 에서 'pane-bottom-left'로 둔 상태가 잔존하면 새 셸의 첫 출력이
    // 어색한 위치에 앵커될 수 있다.
    imeAnchorRef.current = 'textarea';

    // tmux-style prefix 키 매니저. xterm.js의 custom key handler 위에 얹어
    // prefix(Ctrl+B)와 그 뒤의 명령 키를 가로챈다. action 디스패치는 본
    // closure가 책임지고, 매니저는 IPC를 직접 호출하지 않는다 — 그래야
    // 단위 테스트가 가능하다.
    const manager = createKeyboardManager({
      onAction: (action) => {
        if (action.kind === 'detach') {
          // detach: 서버는 attach 가드를 드롭하고 클라이언트는 store에서
          // attached를 비워 SessionLauncher로 자동 복귀한다.
          void detachSession()
            .catch((e: unknown) => {
              logger.warn('pane.detach.failed', { error: String(e) });
            })
            .finally(() => {
              setAttached(null);
            });
          return;
        }
        // 'send-bytes' — Prefix Prefix 등 literal forwarding.
        void ptyInput(paneId, bytesToBase64(action.bytes));
      },
      onStateChange: (state) => {
        setPrefixActive(state === 'awaiting');
      },
      isComposing: () => ime?.isComposing() === true,
      onCancelComposition: () => {
        ime?.cancelComposition();
      },
    });
    term.attachCustomKeyEventHandler(manager.handle);

    // PTY 출력 구독. pane_id가 다른 패널의 이벤트는 무시.
    let unlistenOutput: UnlistenFn | null = null;
    let unlistenEvent: UnlistenFn | null = null;
    let disposed = false;

    void onPtyOutput((payload) => {
      if (payload.pane_id !== paneId) return;
      try {
        const bytes = base64ToBytes(payload.bytes_base64);
        term.write(bytes);
      } catch (e) {
        logger.warn('pane.output.decode_failed', { error: String(e) });
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlistenOutput = fn;
    });

    // server:event 구독. 본 PaneView가 관심 있는 변종만 골라 처리한다.
    // 다른 이벤트(WindowClosed, SessionRenamed 등)는 무시 — 다른 컴포넌트가
    // 자체 구독을 가진다. CLAUDE.md Rule 10: 새 variant도 알 수 없는 형태가
    // 아니라 type-checked union이므로 silent ignore가 아닌 explicit pass다.
    void onServerEvent((evt) => {
      if (evt.type === 'PaneExited') {
        if (evt.pane_id !== paneId) return;
        const msg = tRef.current('pane.exited', { code: evt.exit_code });
        term.write(`\r\n${msg}\r\n`);
        return;
      }
      if (evt.type === 'PaneCursorVisibility') {
        if (evt.pane_id !== paneId) return;
        // TUI 앱이 PTY cursor를 숨겼다면 anchor를 pane-bottom-left로,
        // 다시 보이면 textarea로 복귀. IME composition이 진행 중이지
        // 않더라도 다음 compositionupdate에 적용된다.
        imeAnchorRef.current = evt.visible ? 'textarea' : 'pane-bottom-left';
        return;
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlistenEvent = fn;
    });

    // 컨테이너 크기 변화 추적 — fit → resize 알림.
    const observer = new ResizeObserver(() => {
      try {
        fit.fit();
      } catch (e) {
        logger.warn('xterm.fit.failed', { error: String(e) });
        return;
      }
      void resizePane(paneId, term.rows, term.cols);
    });
    observer.observe(container);

    return () => {
      disposed = true;
      observer.disconnect();
      dataDisposable.dispose();
      manager.dispose();
      if (ime) ime.dispose();
      // 패널이 사라질 때 prefix 인디케이터가 켜진 상태로 굳지 않도록.
      setPrefixActive(false);
      if (unlistenOutput) unlistenOutput();
      if (unlistenEvent) unlistenEvent();
      term.dispose();
    };
    // 스냅샷이 바뀌었다는 것은 새 어태치라는 뜻이므로 effect를 재실행해
    // 깨끗한 Terminal에 새 화면을 그리도록 둔다. (보통 paneId가 같이 바뀌므로
    // 어느 deps로 트리거되든 결과는 같다.)
  }, [paneId, initialSnapshotBase64, setAttached, setPrefixActive]);

  return (
    <div
      ref={containerRef}
      className="h-full w-full"
      style={{
        backgroundColor: CAMPBELL_THEME.background,
        padding: '4px',
      }}
    />
  );
}
