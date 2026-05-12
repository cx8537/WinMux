// 메인 윈도우 하단의 상태바.
//
// M0 단계에서는 server 상태 한 줄만 표시한다. `.tmux.conf`의
// `status-format` 처리(docs/spec/07 § Status Bar)는 후속.

import { useTranslation } from 'react-i18next';

import { useSessionStore } from '@/store/session';

export function StatusBar() {
  const { t } = useTranslation();
  const status = useSessionStore((s) => s.status);

  let text = '';
  let tone: 'normal' | 'warn' = 'normal';
  if (status.state === 'connecting') {
    text = t('status.connecting');
  } else if (status.state === 'connected') {
    text = t('status.connected', { version: status.server_version });
  } else {
    text = t('status.disconnected', { reason: status.reason });
    tone = 'warn';
  }

  return (
    <footer
      className="flex h-7 items-center gap-3 border-t px-3 text-xs"
      style={{
        backgroundColor: 'var(--bg-secondary)',
        borderColor: 'var(--border-subtle)',
        color: tone === 'warn' ? 'var(--status-warn)' : 'var(--text-secondary)',
      }}
    >
      <span>{text}</span>
    </footer>
  );
}
