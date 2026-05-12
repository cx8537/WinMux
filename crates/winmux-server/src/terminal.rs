//! `alacritty_terminal`을 감싼 가상 터미널 어댑터.
//!
//! `docs/spec/02-pty-and-terminal.md` § Virtual Terminal 의 `VirtualTerm` API를
//! 1:1로 구현한다. 서버의 어떤 다른 모듈도 `alacritty_terminal`을 직접
//! 의존하지 않는다 — 본 모듈만 그 의존을 갖고, 나머지는 [`VirtualTerm`]을
//! 통해 grid에 접근한다 (spec § Wrapping vs depending directly).
//!
//! CLAUDE.md Rule 1: 본 모듈은 PTY 콘텐츠를 절대 로그에 남기지 않는다.
//! [`VirtualTerm::feed`]는 bytes의 길이만, [`VirtualTerm::snapshot`]은 출력
//! 길이만 메트릭으로 노출한다.

use std::io::Write as IoWrite;
use std::sync::{Arc, Mutex as StdMutex};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as AlacEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::Config;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::Processor;

/// 스크롤백 기본 라인 수. `winmux.toml`의 `scrollback.lines`가 들어오면
/// `with_scrollback`을 통해 덮어쓴다 (`docs/spec/09-config.md` § scrollback).
pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

/// `Term::new`에 넘기는 dimensions. `alacritty_terminal::grid::Dimensions`
/// trait을 만족시키는 최소 어댑터.
#[derive(Copy, Clone, Debug)]
struct TermSize {
    rows: u16,
    cols: u16,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        usize::from(self.rows)
    }
    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }
    fn columns(&self) -> usize {
        usize::from(self.cols)
    }
}

/// `Term`의 generic parameter에 들어가는 이벤트 싱크.
///
/// `Term::title` 필드는 0.25에서 private이라 직접 읽을 수 없다. 대신 listener의
/// [`AlacEvent::Title`] / [`AlacEvent::ResetTitle`] 콜백으로 latest title을 캡쳐해
/// [`VirtualTerm`]이 공유하는 슬롯에 저장한다.
#[derive(Clone)]
struct ServerListener {
    title: Arc<StdMutex<Option<String>>>,
}

impl EventListener for ServerListener {
    fn send_event(&self, event: AlacEvent) {
        match event {
            AlacEvent::Title(t) => {
                if let Ok(mut g) = self.title.lock() {
                    *g = Some(t);
                }
            }
            AlacEvent::ResetTitle => {
                if let Ok(mut g) = self.title.lock() {
                    *g = None;
                }
            }
            // 그 외 이벤트(MouseCursorDirty, Bell, ClipboardStore 등)는 server에서
            // 처리할 일이 없으므로 무시한다.
            _ => {}
        }
    }
}

/// 한 패널의 가상 터미널 상태.
///
/// 한 [`VirtualTerm`]은 한 ConPTY와 1:1 대응되며, 그 ConPTY의 stdout 바이트
/// 스트림을 누적 파싱해 in-memory grid를 유지한다. reattach 시 [`Self::snapshot`]을
/// 호출해 그 grid를 escape sequence로 재직렬화한다.
pub struct VirtualTerm {
    term: Term<ServerListener>,
    parser: Processor,
    title: Arc<StdMutex<Option<String>>>,
}

impl VirtualTerm {
    /// 지정 크기로 새 가상 터미널을 만든다. 스크롤백은 [`DEFAULT_SCROLLBACK_LINES`].
    #[must_use]
    pub fn new(rows: u16, cols: u16) -> Self {
        Self::with_scrollback(rows, cols, DEFAULT_SCROLLBACK_LINES)
    }

    /// 스크롤백 라인 수를 명시해 만든다 (설정 모듈에서 사용).
    #[must_use]
    pub fn with_scrollback(rows: u16, cols: u16, scrollback_lines: usize) -> Self {
        let config = Config {
            scrolling_history: scrollback_lines,
            ..Config::default()
        };
        let dim = TermSize { rows, cols };
        let title = Arc::new(StdMutex::new(None));
        let term = Term::new(
            config,
            &dim,
            ServerListener {
                title: title.clone(),
            },
        );
        Self {
            term,
            parser: Processor::new(),
            title,
        }
    }

    /// PTY 출력 바이트를 파서에 흘려보내 grid 상태를 갱신한다.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// 가상 터미널 크기를 변경한다. ConPTY와 같은 값으로 동기화되어야 한다.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.term.resize(TermSize { rows, cols });
    }

    /// OSC 0/2로 마지막에 설정된 타이틀.
    #[must_use]
    pub fn title(&self) -> Option<String> {
        match self.title.lock() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    /// 현재 grid를 escape sequence로 직렬화한다.
    ///
    /// 출력은 "신선한 xterm 호환 단말에 그대로 써넣으면 화면이 복원되는"
    /// 형태다 (spec § snapshot()). M0 PoC 단계에서는 다음을 보존한다:
    ///
    /// - 가시 영역 전 셀의 글리프(UTF-8).
    /// - 최종 cursor 위치.
    ///
    /// 색상/속성 직렬화는 spec § Wide chars/Alt screen와 함께 후속 단계
    /// (`docs/decisions.md` § VT-2)로 이연한다. 본 PoC 출력으로도 reattach
    /// 화면이 직전과 동일한 글리프 배열로 복원되므로 M0 정의 "Detach and
    /// reattach show the previous screen"을 충족한다.
    #[must_use]
    pub fn snapshot(&self) -> Vec<u8> {
        let grid = self.term.grid();
        let rows = grid.screen_lines();
        let cols = grid.columns();

        // 화면 클리어 + 홈 + SGR 초기화. wraparound는 명시 enable해서 라인
        // 끝에 도달했을 때 다음 라인으로 자동 넘어가게 한다 (xterm.js 기본값).
        let mut out: Vec<u8> = Vec::with_capacity(rows.saturating_mul(cols + 4) + 64);
        out.extend_from_slice(b"\x1b[2J\x1b[H\x1b[0m");

        let mut glyph_buf = [0u8; 4];
        for line_idx in 0..rows {
            // 라인 시작으로 절대 이동 (1-based).
            let _ = write!(&mut out, "\x1b[{};1H", line_idx + 1);
            let line = Line(i32::try_from(line_idx).unwrap_or(i32::MAX));
            for col_idx in 0..cols {
                let cell = &grid[line][Column(col_idx)];
                // WIDE_CHAR_SPACER는 직전 wide char가 차지한 두 번째 셀이므로
                // 글리프를 한 번만 emit한다 (spec § Wide characters).
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let glyph_bytes = cell.c.encode_utf8(&mut glyph_buf);
                out.extend_from_slice(glyph_bytes.as_bytes());
            }
        }

        // SGR 리셋 후 cursor 위치를 마지막에 적용.
        out.extend_from_slice(b"\x1b[0m");
        let cursor = grid.cursor.point;
        let cursor_line = cursor.line.0.max(0).saturating_add(1);
        let cursor_col = cursor.column.0.saturating_add(1);
        let _ = write!(&mut out, "\x1b[{cursor_line};{cursor_col}H");
        out
    }

    /// 현재 grid의 (행, 열). 테스트와 메트릭에서 사용.
    #[must_use]
    pub fn dimensions(&self) -> (u16, u16) {
        let g = self.term.grid();
        let rows = u16::try_from(g.screen_lines()).unwrap_or(u16::MAX);
        let cols = u16::try_from(g.columns()).unwrap_or(u16::MAX);
        (rows, cols)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn new_terminal_starts_blank() {
        let vt = VirtualTerm::new(10, 20);
        assert_eq!(vt.dimensions(), (10, 20));
        let snap = vt.snapshot();
        // 적어도 초기화 시퀀스는 들어있어야 한다.
        assert!(snap.starts_with(b"\x1b[2J\x1b[H\x1b[0m"));
        // 마지막에는 cursor 1;1로 이동하는 시퀀스가 있다.
        assert!(snap.ends_with(b"\x1b[1;1H"));
    }

    #[test]
    fn feed_plain_ascii_round_trips_into_snapshot() {
        let mut vt = VirtualTerm::new(5, 20);
        vt.feed(b"hello world");
        let snap = vt.snapshot();
        // 글리프가 snapshot 안에 들어있는지만 확인 (위치 검증은 grid가 보증).
        assert!(
            snap.windows(b"hello world".len())
                .any(|w| w == b"hello world"),
            "expected glyphs not present in snapshot"
        );
        // cursor가 11번째 컬럼으로 이동했어야 한다 (0-based 10, 1-based 11).
        assert!(snap.ends_with(b"\x1b[1;12H"));
    }

    #[test]
    fn feed_then_clear_then_text_only_keeps_latest() {
        let mut vt = VirtualTerm::new(5, 20);
        vt.feed(b"first");
        vt.feed(b"\x1b[2J\x1b[H");
        vt.feed(b"latest");
        let snap = vt.snapshot();
        assert!(snap.windows(6).any(|w| w == b"latest"));
        // "first"는 지워졌으므로 등장하지 않는다.
        assert!(
            !snap.windows(5).any(|w| w == b"first"),
            "stale text leaked into snapshot"
        );
    }

    #[test]
    fn resize_changes_dimensions() {
        let mut vt = VirtualTerm::new(10, 20);
        vt.resize(24, 80);
        assert_eq!(vt.dimensions(), (24, 80));
    }

    #[test]
    fn title_captured_from_osc() {
        let mut vt = VirtualTerm::new(5, 10);
        // OSC 0;title BEL — set both icon name and window title.
        vt.feed(b"\x1b]0;my-title\x07");
        assert_eq!(vt.title().as_deref(), Some("my-title"));
    }
}
