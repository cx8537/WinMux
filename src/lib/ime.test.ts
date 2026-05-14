import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createImeManager } from '@/lib/ime';
import type { ImeAnchorMode, ImeManager } from '@/lib/ime';
import type { KeyboardAction, KeyboardState } from '@/lib/keyboard';
import { createKeyboardManager } from '@/lib/keyboard';

// vitest 기본 환경은 node라 DOM이 없다. ime 매니저는 textarea / 부모
// element에 listener를 걸고 overlay div를 자식으로 추가하므로, 테스트는
// "EventTarget 동작 + 자식 노드 보관 + querySelector" 정도만 만족하는
// 최소 stub을 만든다. 합성 이벤트는 객체 캐스팅으로 매니저에 직접 던진다.

/** EventTarget 베이스. addEventListener / dispatchEvent만 신뢰 가능. */
function makeTextareaStub(): HTMLTextAreaElement {
  const target = new EventTarget();
  // blur / focus 호출 카운트는 cancelComposition 검증에 사용.
  const calls = { blur: 0, focus: 0 };
  const stub = {
    addEventListener: target.addEventListener.bind(target),
    removeEventListener: target.removeEventListener.bind(target),
    dispatchEvent: target.dispatchEvent.bind(target),
    blur: () => {
      calls.blur += 1;
    },
    focus: () => {
      calls.focus += 1;
    },
    getBoundingClientRect: () => ({
      left: 100,
      top: 200,
      right: 200,
      bottom: 220,
      width: 100,
      height: 20,
      x: 100,
      y: 200,
      toJSON: () => ({}),
    }),
    style: {} as CSSStyleDeclaration,
    value: '',
    __calls: calls,
  };
  return stub as unknown as HTMLTextAreaElement;
}

interface ParentStubExtras {
  readonly children: HTMLElement[];
  appended: HTMLElement | null;
}

/** 자식 추가 / querySelector / getBoundingClientRect를 만족하는 최소 stub. */
function makeParentStub(): HTMLElement & ParentStubExtras {
  const children: HTMLElement[] = [];
  const stub = {
    appendChild(node: HTMLElement): HTMLElement {
      children.push(node);
      // 매니저는 div 생성 후 parentNode.removeChild를 부른다. removeChild가
      // children 배열에서도 빠지도록 parent를 주입한다.
      Object.defineProperty(node, 'parentNode', {
        configurable: true,
        get: () => stub,
      });
      stub.appended = node;
      return node;
    },
    removeChild(node: HTMLElement): HTMLElement {
      const i = children.indexOf(node);
      if (i >= 0) children.splice(i, 1);
      return node;
    },
    querySelector: (sel: string): HTMLElement | null => {
      // 본 모듈은 '.composition-view'만 조회한다. 기본은 없다고 응답.
      if (sel === '.composition-view') return null;
      return null;
    },
    getBoundingClientRect: () => ({
      left: 0,
      top: 0,
      right: 800,
      bottom: 600,
      width: 800,
      height: 600,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }),
    children,
    appended: null as HTMLElement | null,
  };
  return stub as unknown as HTMLElement & ParentStubExtras;
}

/** document.createElement('div')는 vitest의 node 환경에서 정의되지 않을 수
 *  있으므로, ime.ts가 호출하기 전에 globalThis.document를 얕은 stub으로
 *  덮어 둔다. 기존 값이 있으면 보존했다가 복원. */
interface ElementShim {
  className: string;
  textContent: string;
  style: Record<string, string>;
  parentNode: HTMLElement | null;
  // EventTarget으로 위임 — 본 테스트에서 overlay에 이벤트를 던질 일은 없다.
  setAttribute: (name: string, value: string) => void;
  attributes: Record<string, string>;
}

function installDocumentShim(): () => void {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  const prev = g.document;
  g.document = {
    createElement: (tag: string): ElementShim => {
      const attributes: Record<string, string> = {};
      return {
        className: '',
        textContent: '',
        style: {},
        parentNode: null,
        attributes,
        setAttribute: (name: string, value: string) => {
          attributes[name] = value;
        },
      } satisfies ElementShim;
      void tag;
    },
  };
  return () => {
    g.document = prev;
  };
}

/** 합성 이벤트 객체 생성. 매니저가 보는 표면은 `data` 한 필드뿐이다. */
function compEvent(type: string, data: string): CompositionEvent {
  // EventTarget.dispatchEvent는 Event 인스턴스를 요구하지만 매니저는
  // listener에서 type / data 만 본다. 안전한 합성을 위해 Event를 기반으로
  // data를 덧붙인다.
  const ev = new Event(type) as Event & { data: string };
  ev.data = data;
  return ev as unknown as CompositionEvent;
}

interface ImeFixture {
  readonly textarea: HTMLTextAreaElement & { __calls: { blur: number; focus: number } };
  readonly parent: HTMLElement & ParentStubExtras;
  readonly ime: ImeManager;
  readonly events: { starts: number; updates: string[]; ends: string[] };
  readonly restoreDocument: () => void;
}

function makeIme(): ImeFixture {
  const restoreDocument = installDocumentShim();
  const textarea = makeTextareaStub() as HTMLTextAreaElement & {
    __calls: { blur: number; focus: number };
  };
  const parent = makeParentStub();
  const events = { starts: 0, updates: [] as string[], ends: [] as string[] };
  const ime = createImeManager({
    textarea,
    overlayParent: parent,
    fontFamily: 'monospace',
    fontSize: 14,
    onCompositionStart: () => {
      events.starts += 1;
    },
    onCompositionUpdate: (data) => {
      events.updates.push(data);
    },
    onCompositionEnd: (composed) => {
      events.ends.push(composed);
    },
  });
  return { textarea, parent, ime, events, restoreDocument };
}

describe('createImeManager', () => {
  let fx: ImeFixture | null = null;

  afterEach(() => {
    if (fx) {
      fx.ime.dispose();
      fx.restoreDocument();
      fx = null;
    }
  });

  it('overlay is appended to parent on creation', () => {
    fx = makeIme();
    expect(fx.parent.appended).not.toBeNull();
    expect(fx.parent.children.length).toBe(1);
  });

  it('compositionstart enters composing state and fires the callback', () => {
    fx = makeIme();
    expect(fx.ime.isComposing()).toBe(false);
    fx.textarea.dispatchEvent(compEvent('compositionstart', ''));
    expect(fx.ime.isComposing()).toBe(true);
    expect(fx.events.starts).toBe(1);
  });

  it('compositionupdate forwards data and keeps composing true', () => {
    fx = makeIme();
    fx.textarea.dispatchEvent(compEvent('compositionstart', ''));
    fx.textarea.dispatchEvent(compEvent('compositionupdate', 'ㄱ'));
    fx.textarea.dispatchEvent(compEvent('compositionupdate', '가'));
    fx.textarea.dispatchEvent(compEvent('compositionupdate', '간'));
    expect(fx.events.updates).toEqual(['ㄱ', '가', '간']);
    expect(fx.ime.isComposing()).toBe(true);
  });

  it('compositionend fires onCompositionEnd exactly once with final string', () => {
    fx = makeIme();
    fx.textarea.dispatchEvent(compEvent('compositionstart', ''));
    fx.textarea.dispatchEvent(compEvent('compositionupdate', '간'));
    fx.textarea.dispatchEvent(compEvent('compositionend', '간'));
    expect(fx.events.ends).toEqual(['간']);
    expect(fx.ime.isComposing()).toBe(false);
  });

  it('cancelComposition during composition calls blur/focus and clears state', () => {
    fx = makeIme();
    fx.textarea.dispatchEvent(compEvent('compositionstart', ''));
    fx.textarea.dispatchEvent(compEvent('compositionupdate', 'ㄱ'));
    expect(fx.ime.isComposing()).toBe(true);
    fx.ime.cancelComposition();
    expect(fx.ime.isComposing()).toBe(false);
    expect(fx.textarea.__calls.blur).toBe(1);
    expect(fx.textarea.__calls.focus).toBe(1);
  });

  it('cancelComposition is a no-op when not composing', () => {
    fx = makeIme();
    fx.ime.cancelComposition();
    expect(fx.textarea.__calls.blur).toBe(0);
    expect(fx.textarea.__calls.focus).toBe(0);
  });

  it('overlay anchor switches between textarea and pane-bottom-left across layout passes', () => {
    // 본 테스트는 spec § 04 §§ 191-198의 후속 작업(TUI caret-aware anchor)을 검증한다.
    // 일반 셸이면 helper-textarea rect(=PTY cursor 셀)를 따라가고, TUI 앱이
    // `ESC[?25l`로 PTY cursor를 숨기면 패널 bottom-left 고정 좌표로 전환한다.
    const restoreDocument = installDocumentShim();
    try {
      const textarea = makeTextareaStub();
      const parent = makeParentStub();
      let mode: ImeAnchorMode = 'textarea';
      const ime = createImeManager({
        textarea,
        overlayParent: parent,
        fontFamily: 'monospace',
        fontSize: 14,
        getAnchorMode: () => mode,
      });
      try {
        // 1) textarea 모드 — taRect(left=100, top=200), parentRect(left=0, top=0)
        //    overlay.left = 100, overlay.top = 200.
        textarea.dispatchEvent(compEvent('compositionstart', ''));
        textarea.dispatchEvent(compEvent('compositionupdate', '가'));
        const overlay = parent.appended;
        expect(overlay).not.toBeNull();
        if (overlay === null) throw new Error('overlay must be appended');
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const style = (overlay as any).style as Record<string, string>;
        expect(style.left).toBe('100px');
        expect(style.top).toBe('200px');

        // 2) PTY cursor 숨김 전이 — anchor 모드 전환 후 다음 update에서
        //    overlay는 parent의 bottom-left + inset으로 이동해야 한다.
        //    parent.height = 600, fontSize = 14 → lineHeight = 18,
        //    inset = 8 → top = 600 - 18 - 8 = 574, left = 8.
        mode = 'pane-bottom-left';
        textarea.dispatchEvent(compEvent('compositionupdate', '간'));
        expect(style.left).toBe('8px');
        expect(style.top).toBe('574px');

        // 3) 다시 가시 상태로 — overlay는 즉시 textarea rect로 복귀.
        mode = 'textarea';
        textarea.dispatchEvent(compEvent('compositionupdate', '강'));
        expect(style.left).toBe('100px');
        expect(style.top).toBe('200px');
      } finally {
        ime.dispose();
      }
    } finally {
      restoreDocument();
    }
  });

  it('overlay defaults to textarea anchor when getAnchorMode is not supplied', () => {
    const restoreDocument = installDocumentShim();
    try {
      const textarea = makeTextareaStub();
      const parent = makeParentStub();
      const ime = createImeManager({
        textarea,
        overlayParent: parent,
        fontFamily: 'monospace',
        fontSize: 14,
      });
      try {
        textarea.dispatchEvent(compEvent('compositionstart', ''));
        textarea.dispatchEvent(compEvent('compositionupdate', '가'));
        const overlay = parent.appended;
        if (overlay === null) throw new Error('overlay must be appended');
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const style = (overlay as any).style as Record<string, string>;
        // taRect(100,200) - parentRect(0,0) = (100,200).
        expect(style.left).toBe('100px');
        expect(style.top).toBe('200px');
      } finally {
        ime.dispose();
      }
    } finally {
      restoreDocument();
    }
  });

  it('dispose removes the overlay and detaches listeners', () => {
    fx = makeIme();
    fx.ime.dispose();
    expect(fx.parent.children.length).toBe(0);
    // dispose 후 dispatch가 들어와도 상태는 변하지 않아야 한다.
    fx.textarea.dispatchEvent(compEvent('compositionstart', ''));
    expect(fx.ime.isComposing()).toBe(false);
    expect(fx.events.starts).toBe(0);
  });
});

// ────────────────────────────────────────────────────────────────────
// keyboard 매니저 통합: prefix가 합성 중에 들어오면 합성을 취소하고
// AwaitingCommand로 진입해야 한다. (spec § 04 §§ 183-185.)
// ────────────────────────────────────────────────────────────────────

interface KeyOptions {
  readonly type?: 'keydown' | 'keyup';
  readonly key: string;
  readonly ctrlKey?: boolean;
  readonly shiftKey?: boolean;
  readonly altKey?: boolean;
  readonly metaKey?: boolean;
}

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

describe('keyboard + ime integration', () => {
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

  it('Ctrl+B during composition cancels IME and enters awaiting', () => {
    const restoreDocument = installDocumentShim();
    try {
      const textarea = makeTextareaStub() as HTMLTextAreaElement & {
        __calls: { blur: number; focus: number };
      };
      const parent = makeParentStub();
      const ime = createImeManager({
        textarea,
        overlayParent: parent,
        fontFamily: 'monospace',
        fontSize: 14,
      });
      try {
        const km = createKeyboardManager({
          onAction: (a) => actions.push(a),
          onStateChange: (s) => states.push(s),
          isComposing: () => ime.isComposing(),
          onCancelComposition: () => ime.cancelComposition(),
        });
        // 합성 시작.
        textarea.dispatchEvent(compEvent('compositionstart', ''));
        textarea.dispatchEvent(compEvent('compositionupdate', 'ㄱ'));
        expect(ime.isComposing()).toBe(true);

        // 합성 도중 Ctrl+B → 매니저는 합성 취소하고 awaiting으로.
        const handled = km.handle(key({ key: 'b', ctrlKey: true }));
        expect(handled).toBe(false); // prefix는 매니저가 소비.
        expect(ime.isComposing()).toBe(false);
        expect(textarea.__calls.blur).toBe(1);
        expect(textarea.__calls.focus).toBe(1);
        expect(km.getState()).toBe('awaiting');
        expect(states).toEqual(['awaiting']);

        // 이어서 d 키 → detach action 발사.
        expect(km.handle(key({ key: 'd' }))).toBe(false);
        expect(actions).toEqual([{ kind: 'detach' }]);
        expect(km.getState()).toBe('idle');

        km.dispose();
      } finally {
        ime.dispose();
      }
    } finally {
      restoreDocument();
    }
  });

  it('plain keys during composition are passed through (returned true)', () => {
    const restoreDocument = installDocumentShim();
    try {
      const textarea = makeTextareaStub();
      const parent = makeParentStub();
      const ime = createImeManager({
        textarea,
        overlayParent: parent,
        fontFamily: 'monospace',
        fontSize: 14,
      });
      try {
        const km = createKeyboardManager({
          onAction: (a) => actions.push(a),
          isComposing: () => ime.isComposing(),
          onCancelComposition: () => ime.cancelComposition(),
        });
        textarea.dispatchEvent(compEvent('compositionstart', ''));
        // 합성 중 입력된 일반 키: 매니저는 통과(true).
        expect(km.handle(key({ key: 'a' }))).toBe(true);
        expect(km.handle(key({ key: 'd' }))).toBe(true);
        expect(actions).toEqual([]);
        // 합성 종료 후엔 다시 정상 흐름.
        textarea.dispatchEvent(compEvent('compositionend', '가'));
        expect(km.handle(key({ key: 'a' }))).toBe(true);

        km.dispose();
      } finally {
        ime.dispose();
      }
    } finally {
      restoreDocument();
    }
  });
});
