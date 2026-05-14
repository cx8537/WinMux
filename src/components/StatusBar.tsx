// 메인 윈도우 하단의 상태바.
//
// M1.1 단계에서 표시하는 것:
// - server 상태 한 줄 (M0부터).
// - prefix가 활성화된 동안 작은 인디케이터 (spec § 04 § State Machine).
// `.tmux.conf`의 `status-format` 처리(docs/spec/07 § Status Bar)는 후속.

import { useTranslation } from 'react-i18next';

import { useSessionStore } from '@/store/session';

export function StatusBar() {
  const { t } = useTranslation();
  const status = useSessionStore((s) => s.status);
  const prefixActive = useSessionStore((s) => s.prefixActive);

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
      {prefixActive ? (
        <span
          className="rounded-sm px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
          style={{
            backgroundColor: 'var(--accent)',
            color: 'var(--text-on-accent)',
          }}
        >
          {t('status.prefixActive')}
        </span>
      ) : null}
    </footer>
  );
}
