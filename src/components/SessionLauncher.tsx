// 어태치된 세션이 없을 때 보여 주는 빈 상태 화면.
//
// 두 가지를 한 화면에 띄운다:
// 1. 서버에 살아 있는 세션 목록과 각 항목의 "어태치" 버튼
//    — 클라이언트가 끝났다 다시 떴을 때 직전 세션으로 돌아갈 수 있게.
// 2. 새 세션 만들기 폼. 사용자 이름이 비면 서버가 `untitled-N`을 부여하고
//    shell이 비면 server 측 `resolve_shell`이 COMSPEC 혹은 `powershell.exe`로
//    결정한다 (`docs/spec/02-pty-and-terminal.md`).

import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { logger } from '@/lib/logger';
import type { SessionId, SessionSummary } from '@/lib/protocol';
import { attachSession, listSessions, newSession } from '@/lib/server-client';
import { useSessionStore } from '@/store/session';

/** 클라이언트가 처음 attach할 때 임시로 알리는 셀 크기. PaneView 마운트 직후
 *  ResizeObserver+FitAddon이 정확한 크기로 winmux_resize를 한 번 더 보낸다. */
const FALLBACK_ROWS = 24;
const FALLBACK_COLS = 80;

export function SessionLauncher() {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [shell, setShell] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null);
  const [attachingId, setAttachingId] = useState<SessionId | null>(null);
  const setAttached = useSessionStore((s) => s.setAttached);
  const statusState = useSessionStore((s) => s.status.state);

  const refresh = useCallback(async () => {
    try {
      const list = await listSessions();
      setSessions(list);
    } catch (e: unknown) {
      const message = extractMessage(e);
      logger.warn('session-launcher.list_failed', { message });
      // 목록을 불러오지 못해도 새 세션 만들기 UI는 사용 가능해야 한다 — 그래서
      // 빈 배열로 두고 inline 안내만 띄운다.
      setSessions([]);
    }
  }, []);

  // 서버 연결이 connected로 바뀐 시점에 목록을 한 번 가져온다. connecting이나
  // disconnected에서는 listSessions가 의미가 없으므로 시도하지 않는다.
  useEffect(() => {
    if (statusState !== 'connected') {
      setSessions(null);
      return;
    }
    void refresh();
  }, [statusState, refresh]);

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
      } else {
        // detached 모드로 생성된 경우 (현재 UI는 호출하지 않지만 안전망).
        void refresh();
      }
    } catch (e: unknown) {
      const message = extractMessage(e);
      // 이름 충돌은 가장 흔한 케이스라 친절한 안내로 바꾼다.
      if (isAlreadyExistsError(message)) {
        setError(t('launcher.alreadyExists'));
      } else {
        setError(t('errors.newSessionFailed', { message }));
      }
      logger.warn('session-launcher.failed', { message });
    } finally {
      setBusy(false);
    }
  }

  async function attach(session: SessionSummary) {
    setError(null);
    setAttachingId(session.id);
    try {
      const outcome = await attachSession({
        session: { kind: 'id', id: session.id },
        rows: FALLBACK_ROWS,
        cols: FALLBACK_COLS,
      });
      setAttached(outcome);
    } catch (e: unknown) {
      const message = extractMessage(e);
      setError(t('errors.attachFailed', { message }));
      logger.warn('session-launcher.attach_failed', { message });
    } finally {
      setAttachingId(null);
    }
  }

  return (
    <div className="flex h-full w-full items-center justify-center overflow-auto py-8">
      <div className="flex w-[28rem] flex-col gap-6">
        {/* 기존 세션 목록 */}
        <section
          className="flex flex-col gap-3 rounded-md border p-6"
          style={{
            backgroundColor: 'var(--bg-secondary)',
            borderColor: 'var(--border-subtle)',
          }}
        >
          <h2 className="text-base font-semibold" style={{ color: 'var(--text-primary)' }}>
            {t('launcher.existingSessionsTitle')}
          </h2>

          {sessions === null ? (
            <p className="text-xs" style={{ color: 'var(--text-secondary)' }}>
              {t('launcher.loadingSessions')}
            </p>
          ) : sessions.length === 0 ? (
            <p className="text-xs" style={{ color: 'var(--text-secondary)' }}>
              {t('launcher.noSessions')}
            </p>
          ) : (
            <ul className="flex flex-col gap-2">
              {sessions.map((s) => {
                const isAttaching = attachingId === s.id;
                return (
                  <li
                    key={s.id}
                    className="flex items-center justify-between gap-3 rounded-sm border px-3 py-2"
                    style={{
                      backgroundColor: 'var(--bg-tertiary)',
                      borderColor: 'var(--border-strong)',
                    }}
                  >
                    <div className="flex flex-col">
                      <span className="text-sm" style={{ color: 'var(--text-primary)' }}>
                        {s.name}
                      </span>
                      <span className="text-xs" style={{ color: 'var(--text-secondary)' }}>
                        {t('launcher.sessionMeta', {
                          windows: s.windows,
                          attached: s.attached_clients,
                        })}
                      </span>
                    </div>
                    <button
                      type="button"
                      onClick={() => void attach(s)}
                      disabled={attachingId !== null}
                      className="rounded-sm px-3 py-1 text-xs font-medium disabled:opacity-50"
                      style={{
                        backgroundColor: 'var(--accent)',
                        color: 'var(--text-on-accent)',
                      }}
                    >
                      {isAttaching ? t('launcher.attaching') : t('launcher.attach')}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </section>

        {/* 새 세션 만들기 */}
        <form
          onSubmit={submit}
          className="flex flex-col gap-3 rounded-md border p-6"
          style={{
            backgroundColor: 'var(--bg-secondary)',
            borderColor: 'var(--border-subtle)',
          }}
        >
          <h2 className="text-base font-semibold" style={{ color: 'var(--text-primary)' }}>
            {t('launcher.newSessionTitle')}
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
    </div>
  );
}

function extractMessage(e: unknown): string {
  if (typeof e === 'object' && e !== null && 'message' in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}

/** 서버 `RegistryError::NameTaken`의 와이어 메시지(`session name `X` already exists`)를
 *  탐지한다. 정확한 문자열 비교 대신 substring으로 — 메시지 미세 변경에 견고. */
function isAlreadyExistsError(message: string): boolean {
  return message.includes('already exists');
}
