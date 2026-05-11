//! WinMux IPC 프로토콜 v1.
//!
//! 이 크레이트는 `winmux-server`, `winmux-tray`, `winmux-cli`가 Named
//! Pipe로 주고받는 메시지의 와이어 포맷과, 그 위에서 검증·인코딩에
//! 필요한 최소한의 도우미만 제공한다. 플랫폼 의존성은 들이지 않으며,
//! 의존성은 `serde`(파생 매크로)와 `thiserror`(오류 enum)로 한정한다.
//!
//! 자세한 와이어 정의는 `docs/spec/01-ipc-protocol.md`,
//! 세션 모델은 `docs/spec/03-session-model.md`를 참고한다.

pub mod codec;
pub mod errors;
pub mod ids;
pub mod messages;
pub mod types;
pub mod version;

pub use crate::codec::{CodecError, MAX_MESSAGE_BYTES, decode_line, encode_line};
pub use crate::errors::{ErrorCode, ErrorPayload, ProtocolError};
pub use crate::ids::{ClientId, IdError, MessageId, PaneId, SessionId, WindowId};
pub use crate::messages::{
    AttachTarget, ClientMessage, EventMessage, KillSessionTarget, PaneSnapshot, ServerMessage,
};
pub use crate::types::{
    ClientKind, CommandRequest, CommandResultPayload, NewSessionRequest, PaneSize, PaneSummary,
    SelectDirection, SessionSummary, SplitDirection, WindowSummary,
};
pub use crate::version::{MAX_COMPATIBLE_VERSION, MIN_COMPATIBLE_VERSION, PROTOCOL_VERSION};
