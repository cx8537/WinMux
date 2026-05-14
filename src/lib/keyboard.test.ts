import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { KeyboardAction, KeyboardState } from '@/lib/keyboard';
import { createKeyboardManager } from '@/lib/keyboard';

interface KeyOptions {
  readonly type?: 'keydown' | 'keyup';
  readonly key: string;
  readonly ctrlKey?: boolean;
  readonly shiftKey?: boolean;
  readonly altKey?: boolean;
  readonly metaKey?: boolean;
}

/** 테스트용 합성 KeyboardEvent. xterm.js가 넘기는 native 이벤트와 같은
 *  shape을 흉내낸다 — 매니저가 보는 표면은 type/key/{*}Key 6 필드뿐이다. */
function key(opts: KeyOptions): KeyboardEvent {
  return {
    type: opts.type ?? 'keydown',
    key: opts.key,
    ctrlKey: opts.ctrlKey ?? false,
    shiftKey: opts.shiftKey ?? false,
    altKey: opts.altKey ?? false,
    metaKey: opts.metaKey ?? false,
  } as KeyboardEvent;
}

describe('createKeyboardManager', () => {
  let actions: KeyboardAction[];
  let states: KeyboardState[];

  beforeEach(() => {
    actions = [];
    states = [];
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function makeManager(prefixTimeoutMs = 3000) {
    return createKeyboardManager({
      prefixTimeoutMs,
      onAction: (a) => actions.push(a),
      onStateChange: (s) => states.push(s),
    });
  }

  it('passes plain keys through in idle', () => {
    const m = makeManager();
    expect(m.handle(key({ key: 'a' }))).toBe(true);
    expect(m.handle(key({ key: 'Enter' }))).toBe(true);
    expect(actions).toEqual([]);
    expect(states).toEqual([]);
    expect(m.getState()).toBe('idle');
  });

  it('keyup and modifier-only keys pass through untouched', () => {
    const m = makeManager();
    expect(m.handle(key({ type: 'keyup', key: 'a' }))).toBe(true);
    expect(m.handle(key({ key: 'Control', ctrlKey: true }))).toBe(true);
    expect(m.handle(key({ key: 'Shift', shiftKey: true }))).toBe(true);
    expect(m.getState()).toBe('idle');
    expect(actions).toEqual([]);
  });

  it('Ctrl+B transitions idle → awaiting and swallows the event', () => {
    const m = makeManager();
    expect(m.handle(key({ key: 'b', ctrlKey: true }))).toBe(false);
    expect(m.getState()).toBe('awaiting');
    expect(states).toEqual(['awaiting']);
  });

  it('Ctrl+B with extra modifier is NOT prefix', () => {
    const m = makeManager();
    expect(m.handle(key({ key: 'B', ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(m.getState()).toBe('idle');
  });

  it('detach binding fires on Prefix d', () => {
    const m = makeManager();
    m.handle(key({ key: 'b', ctrlKey: true }));
    expect(m.handle(key({ key: 'd' }))).toBe(false);
    expect(actions).toEqual([{ kind: 'detach' }]);
    expect(m.getState()).toBe('idle');
    expect(states).toEqual(['awaiting', 'idle']);
  });

  it('detach binding ignores Ctrl+d (modifier required to be none)', () => {
    const m = makeManager();
    m.handle(key({ key: 'b', ctrlKey: true }));
    expect(m.handle(key({ key: 'd', ctrlKey: true }))).toBe(false);
    // Ctrl+d는 매핑이 없으므로 unmatched로 처리 — prefix 취소, action 없음.
    expect(actions).toEqual([]);
    expect(m.getState()).toBe('idle');
  });

  it('Prefix Prefix sends literal 0x02 byte', () => {
    const m = makeManager();
    m.handle(key({ key: 'b', ctrlKey: true }));
    expect(m.handle(key({ key: 'b', ctrlKey: true }))).toBe(false);
    expect(actions).toHaveLength(1);
    const a = actions[0];
    if (a === undefined || a.kind !== 'send-bytes') {
      throw new Error(`expected send-bytes action, got ${JSON.stringify(a)}`);
    }
    expect(Array.from(a.bytes)).toEqual([0x02]);
    expect(m.getState()).toBe('idle');
  });

  it('unknown command key cancels prefix and is swallowed', () => {
    const m = makeManager();
    m.handle(key({ key: 'b', ctrlKey: true }));
    expect(m.handle(key({ key: 'q' }))).toBe(false);
    expect(actions).toEqual([]);
    expect(m.getState()).toBe('idle');
  });

  it('awaiting times out back to idle after the configured window', () => {
    const m = makeManager(500);
    m.handle(key({ key: 'b', ctrlKey: true }));
    expect(m.getState()).toBe('awaiting');

    vi.advanceTimersByTime(499);
    expect(m.getState()).toBe('awaiting');

    vi.advanceTimersByTime(1);
    expect(m.getState()).toBe('idle');
    expect(states).toEqual(['awaiting', 'idle']);
  });

  it('a second prefix while already awaiting is the literal-prefix path, not a re-entry', () => {
    const m = makeManager();
    m.handle(key({ key: 'b', ctrlKey: true }));
    m.handle(key({ key: 'b', ctrlKey: true }));
    expect(actions).toHaveLength(1);
    expect(actions[0]?.kind).toBe('send-bytes');
    expect(m.getState()).toBe('idle');
  });

  it('dispose clears the pending timer and resets state', () => {
    const m = makeManager(100);
    m.handle(key({ key: 'b', ctrlKey: true }));
    expect(m.getState()).toBe('awaiting');
    m.dispose();
    expect(m.getState()).toBe('idle');
    // 만료 시각을 지나도 onStateChange가 더 호출되어선 안 된다.
    vi.advanceTimersByTime(1000);
    // 'awaiting' 한 번만 보고됐는지 — dispose는 state change 없이 내부만 정리.
    expect(states).toEqual(['awaiting']);
  });

  it('case-insensitive matching for d and B', () => {
    const m = makeManager();
    // Shift+Ctrl+B 같이 modifier 더 붙으면 prefix 아님(앞에서 확인).
    // 여기선 prefix 진입 후 'D' (Shift 누른 경우)로 들어와도 detach 동작해야 함.
    m.handle(key({ key: 'b', ctrlKey: true }));
    expect(m.handle(key({ key: 'D', shiftKey: true }))).toBe(false);
    expect(actions).toEqual([{ kind: 'detach' }]);
  });
});
