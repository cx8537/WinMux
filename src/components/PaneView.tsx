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

import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';

import type { UnlistenFn } from '@tauri-apps/api/event';
import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';

import { base64ToBytes, utf8ToBase64 } from '@/lib/codec';
import { logger } from '@/lib/logger';
import type { PaneId } from '@/lib/protocol';
import { onPtyOutput, onServerEvent, ptyInput, resizePane } from '@/lib/server-client';

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
}

export function PaneView({ paneId }: PaneViewProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement | null>(null);
  // i18n `t` 변경으로 effect가 재실행되지 않도록 ref로 빼서 항상 최신값만 본다.
  const tRef = useRef(t);
  useEffect(() => {
    tRef.current = t;
  }, [t]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({
      fontFamily: '"Cascadia Mono", "Consolas", "Courier New", monospace',
      fontSize: 14,
      cursorBlink: true,
      theme: CAMPBELL_THEME,
      scrollback: 10_000,
      convertEol: false,
      allowProposedApi: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    term.open(container);

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
    // 서버는 NewSession 시 임시 40×120으로 PTY를 만든다. fit으로 계산한
    // 실제 사이즈를 즉시 전송해 ConPTY가 wrap을 다시 그리게 한다.
    void resizePane(paneId, term.rows, term.cols);

    // 사용자 입력 → server.
    const dataDisposable = term.onData((data) => {
      void ptyInput(paneId, utf8ToBase64(data));
    });

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

    // PaneExited 이벤트 — 종료 코드 표시.
    void onServerEvent((evt) => {
      if (evt.type !== 'PaneExited' || evt.pane_id !== paneId) return;
      const msg = tRef.current('pane.exited', { code: evt.exit_code });
      term.write(`\r\n${msg}\r\n`);
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
      if (unlistenOutput) unlistenOutput();
      if (unlistenEvent) unlistenEvent();
      term.dispose();
    };
  }, [paneId]);

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
