// 어태치된 세션이 없을 때 보여 주는 빈 상태 화면.
//
// 사용자 이름을 입력하지 않으면 서버가 `untitled-N`을 부여한다.
// Shell 입력란이 비어 있으면 server 측 `resolve_shell`이 COMSPEC 혹은
// `powershell.exe`로 결정한다 (`docs/spec/02-pty-and-terminal.md`).

import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { logger } from '@/lib/logger';
import { newSession } from '@/lib/server-client';
import { useSessionStore } from '@/store/session';

export function SessionLauncher() {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [shell, setShell] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const setAttached = useSessionStore((s) => s.setAttached);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const outcome = await newSession({
        ...(name.trim() ? { name: name.trim() } : {}),
        ...(shell.trim() ? { shell: shell.trim() } : {}),
      });
      if (outcome) {
        setAttached(outcome);
      }
    } catch (e: unknown) {
      const message = typeof e === 'object' && e && 'message' in e ? String((e as { message: unknown }).message) : String(e);
      setError(t('errors.newSessionFailed', { message }));
      logger.warn('session-launcher.failed', { message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex h-full w-full items-center justify-center">
      <form
        onSubmit={submit}
        className="flex w-[24rem] flex-col gap-3 rounded-md border p-6"
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderColor: 'var(--border-subtle)',
        }}
      >
        <h2 className="text-base font-semibold" style={{ color: 'var(--text-primary)' }}>
          {t('launcher.title')}
        </h2>
        <p className="text-xs" style={{ color: 'var(--text-secondary)' }}>
          {t('launcher.description')}
        </p>

        <label className="flex flex-col gap-1 text-xs" style={{ color: 'var(--text-secondary)' }}>
          {t('launcher.newSessionLabel')}
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t('launcher.newSessionPlaceholder')}
            disabled={busy}
            className="rounded-sm border px-2 py-1 text-sm outline-none"
            style={{
              backgroundColor: 'var(--bg-tertiary)',
              borderColor: 'var(--border-strong)',
              color: 'var(--text-primary)',
            }}
          />
        </label>

        <label className="flex flex-col gap-1 text-xs" style={{ color: 'var(--text-secondary)' }}>
          {t('launcher.shellLabel')}
          <input
            type="text"
            value={shell}
            onChange={(e) => setShell(e.target.value)}
            placeholder={t('launcher.shellPlaceholder')}
            disabled={busy}
            className="rounded-sm border px-2 py-1 text-sm outline-none"
            style={{
              backgroundColor: 'var(--bg-tertiary)',
              borderColor: 'var(--border-strong)',
              color: 'var(--text-primary)',
            }}
          />
        </label>

        {error ? (
          <p className="text-xs" style={{ color: 'var(--status-error)' }}>
            {error}
          </p>
        ) : null}

        <button
          type="submit"
          disabled={busy}
          className="mt-2 rounded-sm px-3 py-1.5 text-sm font-medium disabled:opacity-50"
          style={{
            backgroundColor: 'var(--accent)',
            color: 'var(--text-on-accent)',
          }}
        >
          {busy ? t('launcher.creating') : t('launcher.create')}
        </button>
      </form>
    </div>
  );
}
