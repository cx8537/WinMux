// 브라우저 측 JSON-Lines / base64 헬퍼.
//
// WinMux IPC는 `PtyInput`·`PtyOutput`의 페이로드를 base64로 실어 나른다
// (`docs/spec/01-ipc-protocol.md`). 본 모듈은 그 인코딩을 webview에서
// 깔끔하게 다루기 위한 얇은 래퍼다 — 브라우저 내장 `atob`/`btoa`를
// 그대로 쓰되, 임의 바이트 입력에 대해 안전한 시그니처를 제공한다.

/**
 * 원시 바이트(`Uint8Array`)를 표준 base64 문자열로 인코딩한다.
 *
 * `btoa`는 Latin-1 문자열만 받는다. 따라서 각 바이트를 문자 코드로
 * 매핑한 뒤 호출한다.
 */
export function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i] ?? 0);
  }
  return btoa(binary);
}

/**
 * UTF-8 문자열(키 입력 등)을 base64로 인코딩한다.
 *
 * 키 입력은 한글 IME 결과처럼 멀티바이트 시퀀스가 들어올 수 있으므로
 * `TextEncoder`로 UTF-8 바이트를 만든 뒤 base64로 감싼다.
 */
export function utf8ToBase64(text: string): string {
  return bytesToBase64(new TextEncoder().encode(text));
}

/**
 * 표준 base64 문자열을 원시 바이트로 디코딩한다.
 *
 * 잘못된 입력에 대해 `atob`은 `DOMException`을 던진다. 호출자가 catch
 * 할 책임을 진다.
 */
export function base64ToBytes(text: string): Uint8Array {
  const binary = atob(text);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    out[i] = binary.charCodeAt(i);
  }
  return out;
}
