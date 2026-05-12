//! WinMux Named Pipe IPC 클라이언트.
//!
//! `winmux-tray`와 `winmux-cli`가 공유하는 와이어 어댑터. `winmux-server`의
//! 핸드셰이크·메시지 dispatch 절반과 직접 짝이 된다 — protocol crate의
//! 같은 enum을 양쪽이 사용한다.
//!
//! 본 crate는 PTY나 GUI 의존성을 갖지 않으며, `winmux-server`처럼 Win32
//! `unsafe`도 쓰지 않는다. Tokio 비동기 `NamedPipeClient` 위에 얇은 JSON
//! Lines 어댑터를 둔다.

pub mod client;
pub mod connect;

pub use client::{Client, HelloAckInfo};
pub use connect::{ConnectError, connect, connect_with_retry};
