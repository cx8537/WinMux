import { describe, expect, it } from 'vitest';

import { base64ToBytes, bytesToBase64, utf8ToBase64 } from '@/lib/codec';

describe('codec', () => {
  it('round-trips raw bytes', () => {
    const input = new Uint8Array([0, 1, 2, 0xff, 0xfe, 0xfd, 0x80, 0x7f]);
    const enc = bytesToBase64(input);
    const dec = base64ToBytes(enc);
    expect(Array.from(dec)).toEqual(Array.from(input));
  });

  it('encodes UTF-8 strings correctly', () => {
    // RFC 4648 §10에서 가져온 알려진 벡터.
    expect(utf8ToBase64('')).toBe('');
    expect(utf8ToBase64('f')).toBe('Zg==');
    expect(utf8ToBase64('foo')).toBe('Zm9v');
    expect(utf8ToBase64('foobar')).toBe('Zm9vYmFy');
  });

  it('handles multi-byte UTF-8 (Korean)', () => {
    const input = '한';
    const enc = utf8ToBase64(input);
    const dec = base64ToBytes(enc);
    const decoded = new TextDecoder().decode(dec);
    expect(decoded).toBe(input);
  });

  it('decodes Carriage Return / Line Feed', () => {
    const cr = base64ToBytes('DQo=');
    expect(Array.from(cr)).toEqual([0x0d, 0x0a]);
  });
});
