// WinMux 메인 윈도우 루트.
//
// M0 PoC 단계의 분기는 단순하다:
// - 아직 어태치된 세션이 없으면 `SessionLauncher`로 빈 상태를 보여 준다.
// - 어태치되어 있으면 활성 윈도우의 첫 패널을 `PaneView`로 렌더링한다.
// - 하단에는 항상 `StatusBar`가 server 상태 한 줄을 보여 준다.
//
// 패널 분할 UI, 윈도우 탭, 세션 사이드바는 M1에서 본 컴포넌트 위에 얹는다
// (`docs/spec/07-tray-and-gui.md` § Window layout).

import { useEffect } from 'react';

import { PaneView } from '@/components/PaneView';
import { SessionLauncher } from '@/components/SessionLauncher';
import { StatusBar } from '@/components/StatusBar';
import { onServerBye, onServerStatus } from '@/lib/server-client';
import { useSessionStore } from '@/store/session';

export function App() {
  const attached = useSessionStore((s) => s.attached);
  const setStatus = useSessionStore((s) => s.setStatus);
  const setAttached = useSessionStore((s) => s.setAttached);

  useEffect(() => {
    let unlistenStatus: (() => void) | null = null;
    let unlistenBye: (() => void) | null = null;
    let disposed = false;

    void onServerStatus((status) => {
      setStatus(status);
    }).then((fn) => {
      if (disposed) fn();
      else unlistenStatus = fn;
    });

    void onServerBye(() => {
      setAttached(null);
      setStatus({ state: 'disconnected', reason: 'ServerBye' });
    }).then((fn) => {
      if (disposed) fn();
      else unlistenBye = fn;
    });

    return () => {
      disposed = true;
      if (unlistenStatus) unlistenStatus();
      if (unlistenBye) unlistenBye();
    };
  }, [setStatus, setAttached]);

  const activePane = attached?.panes[0];

  return (
    <div
      className="flex h-screen w-screen flex-col"
      style={{ backgroundColor: 'var(--bg-primary)' }}
    >
      <main className="flex-1 overflow-hidden">
        {attached && activePane ? (
          <PaneView paneId={activePane.id} />
        ) : (
          <SessionLauncher />
        )}
      </main>
      <StatusBar />
    </div>
  );
}
