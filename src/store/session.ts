// 어태치된 세션 상태를 보관하는 Zustand store.
//
// M0 PoC 단계에서는 한 클라이언트가 동시에 한 세션에만 어태치한다고
// 가정한다. M1에서 여러 세션을 빠르게 전환할 때 본 store가 sessionId 별
// 캐시로 확장된다.

import { create } from 'zustand';

import type { AttachOutcome, ServerStatus } from '@/lib/protocol';

interface SessionState {
  /** 현재 어태치된 세션의 결과. 없으면 `null`. */
  attached: AttachOutcome | null;
  /** 마지막으로 알려진 서버 상태. 초기값은 `connecting`. */
  status: ServerStatus;

  setAttached(outcome: AttachOutcome | null): void;
  setStatus(status: ServerStatus): void;
}

export const useSessionStore = create<SessionState>((set) => ({
  attached: null,
  status: { state: 'connecting' },
  setAttached: (outcome) => set({ attached: outcome }),
  setStatus: (status) => set({ status }),
}));
