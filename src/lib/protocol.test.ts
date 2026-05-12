import { describe, expect, it } from 'vitest';

import type { CommandError } from '@/lib/protocol';
import { isCommandError, paneId, sessionId, windowId } from '@/lib/protocol';

describe('protocol', () => {
  it('brands ids without altering the underlying string', () => {
    const pid = paneId('pane-ABCD');
    expect(pid).toBe('pane-ABCD');
    const sid = sessionId('ses-XYZ');
    expect(sid).toBe('ses-XYZ');
    const wid = windowId('win-1');
    expect(wid).toBe('win-1');
  });

  it('isCommandError accepts the canonical shape', () => {
    const err: CommandError = {
      message: 'session not found',
      code: 'SESSION_NOT_FOUND',
      recoverable: true,
    };
    expect(isCommandError(err)).toBe(true);
  });

  it('isCommandError rejects strings and nulls', () => {
    expect(isCommandError('boom')).toBe(false);
    expect(isCommandError(null)).toBe(false);
    expect(isCommandError(undefined)).toBe(false);
    expect(isCommandError(42)).toBe(false);
  });

  it('isCommandError rejects objects missing required fields', () => {
    expect(isCommandError({ message: 'x' })).toBe(false);
    expect(isCommandError({ recoverable: true })).toBe(false);
  });
});
