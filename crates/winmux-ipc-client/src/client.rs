//! 고수준 IPC 클라이언트: Hello 핸드셰이크 + JSON Lines 송수신.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufStream};
use tokio::net::windows::named_pipe::NamedPipeClient;
use tokio::time::timeout;
use tracing::info;
use winmux_protocol::{
    ClientKind, ClientMessage, MessageId, PROTOCOL_VERSION, ServerMessage, decode_line, encode_line,
};

/// 일반 요청의 응답 대기 한도 (`docs/spec/01-ipc-protocol.md` § Timeouts).
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Hello 응답에서 클라이언트가 알게 되는 서버 정보.
#[derive(Clone, Debug)]
pub struct HelloAckInfo {
    /// 서버 빌드 버전 (`CARGO_PKG_VERSION`).
    pub server_version: String,
    /// 서버가 검증한 사용자 이름.
    pub user: String,
    /// HelloAck의 `v` 필드.
    pub protocol_version: u32,
}

/// 양방향 IPC 클라이언트. 한 [`NamedPipeClient`]를 BufStream으로 감싼다.
pub struct Client {
    stream: BufStream<NamedPipeClient>,
    next_msg_id: u64,
}

impl Client {
    /// 이미 연결된 파이프 핸들을 감싼다.
    #[must_use]
    pub fn new(pipe: NamedPipeClient) -> Self {
        Self {
            stream: BufStream::new(pipe),
            // 클라이언트 한 세션 안에서 단조 증가하는 id.
            next_msg_id: 1,
        }
    }

    /// 다음 요청용 [`MessageId`]를 만든다. 포맷: 0-padded 20자리 hex.
    ///
    /// 같은 `Client` 인스턴스 안에서 단조 증가하므로 한 연결 위에서는
    /// `id`가 절대 중복되지 않는다. tray/cli가 자체적으로 id를 발급할
    /// 필요가 없도록 `pub`로 노출한다.
    pub fn next_message_id(&mut self) -> MessageId {
        let body = format!("{:020x}", self.next_msg_id);
        self.next_msg_id = self.next_msg_id.wrapping_add(1);
        // body는 항상 20자라 절대 빈 문자열이 아니다. fallback은 도달 불가.
        MessageId::from_body(&body).unwrap_or_else(|_| MessageId::from_raw(format!("msg-{body}")))
    }

    /// `Hello` 송신 후 `HelloAck`(또는 `Error`)을 받는다.
    ///
    /// `version`은 호출자(보통 `env!("CARGO_PKG_VERSION")`)가 알린다.
    pub async fn hello(&mut self, kind: ClientKind, version: &str) -> Result<HelloAckInfo> {
        let id = self.next_message_id();
        let hello = ClientMessage::Hello {
            v: PROTOCOL_VERSION,
            id,
            client: kind,
            pid: std::process::id(),
            version: version.to_owned(),
        };
        self.send(&hello).await.context("send Hello")?;

        let response = timeout(DEFAULT_REQUEST_TIMEOUT, self.recv())
            .await
            .context("HelloAck timed out")??;
        match response {
            ServerMessage::HelloAck {
                server_version,
                user,
                v,
                ..
            } => {
                info!(server_version = %server_version, user = %user, "ipc.client.authenticated");
                Ok(HelloAckInfo {
                    server_version,
                    user,
                    protocol_version: v,
                })
            }
            ServerMessage::Error { payload, .. } => bail!(
                "server rejected Hello: {} ({})",
                payload.message,
                payload.code.as_str()
            ),
            other => bail!("expected HelloAck, got {other:?}"),
        }
    }

    /// 한 요청을 보내고 한 응답을 받는다. 타임아웃은 [`DEFAULT_REQUEST_TIMEOUT`].
    ///
    /// 스트리밍 메시지(`PtyInput`)에는 쓰지 말 것 — 응답이 없으므로 영원히
    /// 멈춘다. 그런 경우엔 [`Self::send`]만 호출한다.
    pub async fn request(&mut self, msg: &ClientMessage) -> Result<ServerMessage> {
        self.send(msg).await?;
        timeout(DEFAULT_REQUEST_TIMEOUT, self.recv())
            .await
            .context("request timed out")?
    }

    /// 한 [`ClientMessage`]를 JSON Lines 한 줄로 보낸다.
    pub async fn send(&mut self, msg: &ClientMessage) -> Result<()> {
        let json = serde_json::to_vec(msg).context("serialize ClientMessage")?;
        let line = encode_line(&json).context("encode line")?;
        self.stream.write_all(&line).await.context("write line")?;
        self.stream.flush().await.context("flush line")?;
        Ok(())
    }

    /// 다음 한 줄을 읽어 [`ServerMessage`]로 디코딩한다. EOF면 `Err`.
    pub async fn recv(&mut self) -> Result<ServerMessage> {
        let mut buf = Vec::with_capacity(512);
        let n = self
            .stream
            .read_until(b'\n', &mut buf)
            .await
            .context("read line")?;
        if n == 0 {
            bail!("server closed connection");
        }
        let body = decode_line(&buf).context("frame error")?;
        serde_json::from_str::<ServerMessage>(body).context("deserialize ServerMessage")
    }

    /// `Bye` 송신 후 연결을 닫는다. Best-effort — 서버가 이미 끊었으면 무시.
    pub async fn close(mut self) -> Result<()> {
        let bye = ClientMessage::Bye {
            v: PROTOCOL_VERSION,
        };
        let _ = self.send(&bye).await;
        let _ = self.stream.flush().await;
        Ok(())
    }
}
