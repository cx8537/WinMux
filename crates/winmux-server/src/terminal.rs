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
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, Rgb};

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
    /// 형태다 (spec § snapshot()). M1.5에서 다음을 보존한다:
    ///
    /// - 가시 영역 전 셀의 글리프(UTF-8).
    /// - 최종 cursor 위치.
    /// - per-cell SGR 상태: foreground/background color (named 16색,
    ///   indexed 256색, truecolor RGB), bold, italic, underline,
    ///   reverse(inverse), dim, strikeout.
    ///
    /// 인접 셀 간 동일 스타일은 SGR을 재발행하지 않고 글리프만 흘려보낸다.
    /// 스타일이 바뀌는 경계에서는 `\x1b[0m`로 리셋한 뒤 새 스타일에
    /// 해당하는 SGR 코드만 발행해 단말의 active style을 결정론적으로 만든다
    /// (spec § snapshot() "SGR escapes + cell glyph. Adjacent cells with
    /// same style are merged").
    #[must_use]
    pub fn snapshot(&self) -> Vec<u8> {
        let grid = self.term.grid();
        let rows = grid.screen_lines();
        let cols = grid.columns();

        // 화면 클리어 + 홈 + SGR 초기화. wraparound는 명시 enable해서 라인
        // 끝에 도달했을 때 다음 라인으로 자동 넘어가게 한다 (xterm.js 기본값).
        // 평균 셀당 1바이트(글리프) + 가끔 SGR 전이를 가정해 conservative하게
        // 예약한다.
        let mut out: Vec<u8> = Vec::with_capacity(rows.saturating_mul(cols + 8) + 64);
        out.extend_from_slice(b"\x1b[2J\x1b[H\x1b[0m");

        // 직전에 emit한 스타일. 초기 상태는 SGR reset과 동치인 default.
        let mut prev_style = CellStyle::default();
        let mut glyph_buf = [0u8; 4];

        for line_idx in 0..rows {
            // 라인 시작으로 절대 이동 (1-based). 매 라인 시작 시 스타일은
            // 직전 라인 마지막 셀에서 그대로 이어진다 (CUP은 SGR을 리셋하지
            // 않는다); 따라서 prev_style은 행을 가로질러 유지한다.
            let _ = write!(&mut out, "\x1b[{};1H", line_idx + 1);
            let line = Line(i32::try_from(line_idx).unwrap_or(i32::MAX));
            for col_idx in 0..cols {
                let cell = &grid[line][Column(col_idx)];
                // WIDE_CHAR_SPACER는 직전 wide char가 차지한 두 번째 셀이므로
                // 글리프를 한 번만 emit한다 (spec § Wide characters).
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let style = CellStyle::from_cell(cell);
                if style != prev_style {
                    write_sgr_transition(&mut out, &style);
                    prev_style = style;
                }
                let glyph_bytes = cell.c.encode_utf8(&mut glyph_buf);
                out.extend_from_slice(glyph_bytes.as_bytes());
            }
        }

        // SGR 리셋 후 cursor 위치를 마지막에 적용. 리셋을 거치므로 reattach
        // 직후 사용자가 입력한 새 텍스트가 마지막 셀의 색을 상속하지 않는다.
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

    /// PTY 커서가 현재 표시 모드인지(DECTCEM). 초기값은 `true`이며,
    /// `ESC[?25l`로 false, `ESC[?25h`로 다시 true가 된다.
    ///
    /// 트레이가 IME composition overlay의 앵커를 결정할 때 쓰는 신호를
    /// 만들어 내는 1차 소스. 본 함수는 grid·flag 변화 없이 한 비트만
    /// 읽으므로 매 PTY 출력 chunk 직후 호출해도 비용이 사실상 0이다.
    #[must_use]
    pub fn cursor_visible(&self) -> bool {
        self.term.mode().contains(TermMode::SHOW_CURSOR)
    }
}

/// snapshot 직렬화에서 비교 대상이 되는 셀 스타일 부분집합.
///
/// `Flags`는 wide-char/wrapline 같은 layout-only 비트가 섞여 있으므로 비교에
/// 사용하면 색은 같지만 wrap 비트만 다른 인접 셀에서 불필요하게 SGR 전이가
/// 일어난다. 본 구조체는 시각적으로 의미 있는 비트만 추려낸다.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CellStyle {
    fg: Color,
    bg: Color,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    inverse: bool,
    strikeout: bool,
}

impl Default for CellStyle {
    fn default() -> Self {
        Self {
            fg: Color::Named(NamedColor::Foreground),
            bg: Color::Named(NamedColor::Background),
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            inverse: false,
            strikeout: false,
        }
    }
}

impl CellStyle {
    fn from_cell(cell: &Cell) -> Self {
        let f = cell.flags;
        // `Flags::ALL_UNDERLINES`는 단/이중/curl/dotted/dashed를 모두 묶은
        // 비트셋이다. 본 단계에서는 그중 어떤 underline이든 단일 `\x1b[4m`로
        // 환원한다 (spec은 "underline" 보존만 요구).
        Self {
            fg: cell.fg,
            bg: cell.bg,
            bold: f.contains(Flags::BOLD),
            dim: f.contains(Flags::DIM),
            italic: f.contains(Flags::ITALIC),
            underline: f.intersects(Flags::ALL_UNDERLINES),
            inverse: f.contains(Flags::INVERSE),
            strikeout: f.contains(Flags::STRIKEOUT),
        }
    }
}

/// `\x1b[0m`로 리셋한 뒤 `style`을 그대로 표현하는 SGR 시퀀스를 `out`에 기록.
///
/// "diff" 방식 대신 항상 리셋 + 재발행을 택한 이유:
/// 1. xterm.js / 실제 단말의 SGR 상태 머신은 켜기/끄기 코드가 비대칭이라
///    (예: `22`가 bold 단독 해제가 아닌 bold+dim 동시 해제) diff를 안전하게
///    내려면 케이스가 많아진다.
/// 2. snapshot은 가끔 한 번 발행되는 path지 hot path가 아니다
///    (`docs/nonfunctional/performance.md` § Hot Paths "Snapshot on attach").
///    수십 바이트 늘어나도 200 ms p95 reattach SLO에는 의미 없는 비용이다.
fn write_sgr_transition(out: &mut Vec<u8>, style: &CellStyle) {
    out.extend_from_slice(b"\x1b[0m");
    if *style == CellStyle::default() {
        return;
    }

    // SGR 파라미터를 `;`로 모아 한 번의 `\x1b[...m`로 보낸다.
    let mut sgr: Vec<u8> = Vec::with_capacity(32);
    sgr.extend_from_slice(b"\x1b[");
    let mut first = true;
    let push_param = |sgr: &mut Vec<u8>, first: &mut bool, s: &str| {
        if !*first {
            sgr.push(b';');
        }
        sgr.extend_from_slice(s.as_bytes());
        *first = false;
    };

    if style.bold {
        push_param(&mut sgr, &mut first, "1");
    }
    if style.dim {
        push_param(&mut sgr, &mut first, "2");
    }
    if style.italic {
        push_param(&mut sgr, &mut first, "3");
    }
    if style.underline {
        push_param(&mut sgr, &mut first, "4");
    }
    if style.inverse {
        push_param(&mut sgr, &mut first, "7");
    }
    if style.strikeout {
        push_param(&mut sgr, &mut first, "9");
    }

    if style.fg != Color::Named(NamedColor::Foreground) {
        let mut buf = String::with_capacity(20);
        append_color_params(&mut buf, style.fg, ColorRole::Foreground);
        push_param(&mut sgr, &mut first, &buf);
    }
    if style.bg != Color::Named(NamedColor::Background) {
        let mut buf = String::with_capacity(20);
        append_color_params(&mut buf, style.bg, ColorRole::Background);
        push_param(&mut sgr, &mut first, &buf);
    }

    sgr.push(b'm');
    out.extend_from_slice(&sgr);
}

#[derive(Copy, Clone)]
enum ColorRole {
    Foreground,
    Background,
}

impl ColorRole {
    /// 8색 base offset (fg=30, bg=40).
    fn standard_base(self) -> u8 {
        match self {
            Self::Foreground => 30,
            Self::Background => 40,
        }
    }
    /// 8색 bright offset (fg=90, bg=100).
    fn bright_base(self) -> u8 {
        match self {
            Self::Foreground => 90,
            Self::Background => 100,
        }
    }
    /// 확장 색 선두 코드 (fg=38, bg=48).
    fn extended_lead(self) -> u8 {
        match self {
            Self::Foreground => 38,
            Self::Background => 48,
        }
    }
    /// "default color" 코드 (fg=39, bg=49). 본 구현은 reset(`0m`) 이후 default
    /// 색을 명시적으로 다시 쓰지 않으므로 사용처는 없지만, 향후 diff 방식
    /// 전환에 대비해 enum의 일원으로 보존한다.
    #[allow(dead_code)]
    fn default_code(self) -> u8 {
        match self {
            Self::Foreground => 39,
            Self::Background => 49,
        }
    }
}

/// `color`를 SGR 파라미터 문자열로 변환해 `out`에 push.
///
/// 매핑 규칙:
/// - `Named(Black..=White)` → 30+n / 40+n.
/// - `Named(BrightBlack..=BrightWhite)` → 90+(n-8) / 100+(n-8).
/// - `Named(Dim*)` → 베이스 색 + (caller 측에서) `DIM` flag로 표현. 본 함수는
///   베이스 색만 발행한다.
/// - `Named(Foreground|Background)` → caller가 호출하기 전에 default와 비교해
///   skip하므로 도달하지 않는다. 도달 시 default 코드(39/49)로 fallback.
/// - `Named(BrightForeground|DimForeground|Cursor)` → xterm에 대응 코드가
///   없으므로 default(39/49)로 fallback. 사용자의 시각 정보는 잃지만 안전한
///   degradation이다.
/// - `Indexed(0..=7)` → 30+n / 40+n.
/// - `Indexed(8..=15)` → 90+(n-8) / 100+(n-8).
/// - `Indexed(16..=255)` → 38;5;n / 48;5;n.
/// - `Spec(Rgb { r, g, b })` → 38;2;r;g;b / 48;2;r;g;b.
fn append_color_params(out: &mut String, color: Color, role: ColorRole) {
    use std::fmt::Write as _;
    // String에 대한 write!는 io 에러를 낼 수 없으므로 unwrap이 필요 없다.
    // `let _ =`로 명시적으로 결과를 버린다.
    match color {
        Color::Named(name) => match name {
            NamedColor::Black
            | NamedColor::Red
            | NamedColor::Green
            | NamedColor::Yellow
            | NamedColor::Blue
            | NamedColor::Magenta
            | NamedColor::Cyan
            | NamedColor::White => {
                let n = name as u8;
                let _ = write!(out, "{}", role.standard_base() + n);
            }
            NamedColor::BrightBlack
            | NamedColor::BrightRed
            | NamedColor::BrightGreen
            | NamedColor::BrightYellow
            | NamedColor::BrightBlue
            | NamedColor::BrightMagenta
            | NamedColor::BrightCyan
            | NamedColor::BrightWhite => {
                // NamedColor::BrightBlack = 8, ..., BrightWhite = 15.
                let n = (name as u8).saturating_sub(8);
                let _ = write!(out, "{}", role.bright_base() + n);
            }
            NamedColor::DimBlack => {
                let _ = write!(out, "{}", role.standard_base());
            }
            NamedColor::DimRed => {
                let _ = write!(out, "{}", role.standard_base() + 1);
            }
            NamedColor::DimGreen => {
                let _ = write!(out, "{}", role.standard_base() + 2);
            }
            NamedColor::DimYellow => {
                let _ = write!(out, "{}", role.standard_base() + 3);
            }
            NamedColor::DimBlue => {
                let _ = write!(out, "{}", role.standard_base() + 4);
            }
            NamedColor::DimMagenta => {
                let _ = write!(out, "{}", role.standard_base() + 5);
            }
            NamedColor::DimCyan => {
                let _ = write!(out, "{}", role.standard_base() + 6);
            }
            NamedColor::DimWhite => {
                let _ = write!(out, "{}", role.standard_base() + 7);
            }
            NamedColor::Foreground
            | NamedColor::Background
            | NamedColor::Cursor
            | NamedColor::BrightForeground
            | NamedColor::DimForeground => {
                // 정상 경로에서는 default와 일치해 skip되거나, 대응 SGR 코드가
                // 없는 alacritty 내부용이다. default 코드로 fallback.
                let _ = write!(out, "{}", role.default_code());
            }
        },
        Color::Indexed(idx) => {
            if idx < 8 {
                let _ = write!(out, "{}", role.standard_base() + idx);
            } else if idx < 16 {
                let _ = write!(out, "{}", role.bright_base() + (idx - 8));
            } else {
                let _ = write!(out, "{};5;{}", role.extended_lead(), idx);
            }
        }
        Color::Spec(Rgb { r, g, b }) => {
            let _ = write!(out, "{};2;{};{};{}", role.extended_lead(), r, g, b);
        }
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

    /// 초기 상태에서 `SHOW_CURSOR`는 alacritty의 default mode에 포함되므로 true.
    /// `ESC[?25l`(DECTCEM disable)을 흘리면 false로 전이하고, `ESC[?25h`로 다시
    /// true로 복원되는지 검증한다.
    #[test]
    fn cursor_visibility_tracks_dectcem_transitions() {
        let mut vt = VirtualTerm::new(5, 10);
        assert!(
            vt.cursor_visible(),
            "fresh VirtualTerm should start with cursor visible"
        );

        vt.feed(b"\x1b[?25l");
        assert!(
            !vt.cursor_visible(),
            "ESC[?25l should hide the cursor (TUI apps like claude/lazygit do this)"
        );

        // ESC[?25l이 두 번 들어와도 상태는 false 유지.
        vt.feed(b"\x1b[?25l");
        assert!(!vt.cursor_visible());

        vt.feed(b"\x1b[?25h");
        assert!(
            vt.cursor_visible(),
            "ESC[?25h should restore cursor visibility"
        );
    }

    #[test]
    fn title_captured_from_osc() {
        let mut vt = VirtualTerm::new(5, 10);
        // OSC 0;title BEL — set both icon name and window title.
        vt.feed(b"\x1b]0;my-title\x07");
        assert_eq!(vt.title().as_deref(), Some("my-title"));
    }

    /// SGR-rich byte stream을 첫 번째 `VirtualTerm`에 흘려 넣고, 그 `snapshot()`
    /// 출력을 두 번째 `VirtualTerm`에 그대로 다시 흘려 넣은 뒤 두 단말의
    /// 같은 좌표에서 fg/bg/attrs가 일치하는지 검증한다. CLAUDE.md rule 15
    /// (실제 환경, 모의 금지) — 두 단말 모두 진짜 `alacritty_terminal`이다.
    #[test]
    fn snapshot_round_trips_sgr_through_fresh_vterm() {
        // 5x20, 3색 + bold + underline + reverse를 섞은 짧은 시퀀스.
        // 의도된 그리드:
        //   row 0, col 0..=2: 'R','E','D'   fg=Red,  bold
        //   row 0, col 3    : ' '          default
        //   row 0, col 4..=6: 'G','R','N'   bg=Green
        //   row 0, col 7    : ' '          default
        //   row 0, col 8..=11: 'U','N','D','L'  underline
        //   row 0, col 12   : ' '
        //   row 0, col 13..=15: 'R','V','S'   reverse
        let stream: &[u8] =
            b"\x1b[31;1mRED\x1b[0m \x1b[42mGRN\x1b[0m \x1b[4mUNDL\x1b[0m \x1b[7mRVS\x1b[0m";

        let mut a = VirtualTerm::new(5, 20);
        a.feed(stream);
        let snap = a.snapshot();

        let mut b = VirtualTerm::new(5, 20);
        b.feed(&snap);

        // 헬퍼: 양 단말의 같은 좌표에서 셀 동치를 확인.
        let assert_cells_match = |row: usize, col: usize, label: &str| {
            let line = Line(i32::try_from(row).unwrap());
            let ca = &a.term.grid()[line][Column(col)];
            let cb = &b.term.grid()[line][Column(col)];
            assert_eq!(ca.c, cb.c, "{label}: glyph mismatch at ({row},{col})");
            assert_eq!(
                ca.fg, cb.fg,
                "{label}: fg mismatch at ({row},{col}): a={:?} b={:?}",
                ca.fg, cb.fg
            );
            assert_eq!(
                ca.bg, cb.bg,
                "{label}: bg mismatch at ({row},{col}): a={:?} b={:?}",
                ca.bg, cb.bg
            );
            // layout-only 비트는 두 단말의 셀 이력이 달라 자연스레 다를 수
            // 있다(WRAPLINE 등). 시각적 SGR 비트만 비교한다.
            let mask = Flags::BOLD
                | Flags::DIM
                | Flags::ITALIC
                | Flags::ALL_UNDERLINES
                | Flags::INVERSE
                | Flags::STRIKEOUT;
            assert_eq!(
                ca.flags & mask,
                cb.flags & mask,
                "{label}: visual flags mismatch at ({row},{col}): a={:?} b={:?}",
                ca.flags,
                cb.flags
            );
        };

        // RED: fg=Red, bold.
        for (i, ch) in "RED".chars().enumerate() {
            assert_cells_match(0, i, "RED");
            let cell = &a.term.grid()[Line(0)][Column(i)];
            assert_eq!(cell.c, ch);
            assert_eq!(cell.fg, Color::Named(NamedColor::Red));
            assert!(cell.flags.contains(Flags::BOLD));
        }
        // 공백 후 GRN: bg=Green.
        for (i, ch) in "GRN".chars().enumerate() {
            let col = 4 + i;
            assert_cells_match(0, col, "GRN");
            let cell = &a.term.grid()[Line(0)][Column(col)];
            assert_eq!(cell.c, ch);
            assert_eq!(cell.bg, Color::Named(NamedColor::Green));
        }
        // UNDL: underline.
        for (i, ch) in "UNDL".chars().enumerate() {
            let col = 8 + i;
            assert_cells_match(0, col, "UNDL");
            let cell = &a.term.grid()[Line(0)][Column(col)];
            assert_eq!(cell.c, ch);
            assert!(cell.flags.intersects(Flags::ALL_UNDERLINES));
        }
        // RVS: reverse.
        for (i, ch) in "RVS".chars().enumerate() {
            let col = 13 + i;
            assert_cells_match(0, col, "RVS");
            let cell = &a.term.grid()[Line(0)][Column(col)];
            assert_eq!(cell.c, ch);
            assert!(cell.flags.contains(Flags::INVERSE));
        }

        // 빈 칸 한 좌표(스타일 default)도 확인.
        assert_cells_match(0, 3, "gap-1");
        let gap = &a.term.grid()[Line(0)][Column(3)];
        assert_eq!(gap.c, ' ');
        assert_eq!(gap.fg, Color::Named(NamedColor::Foreground));
        assert_eq!(gap.bg, Color::Named(NamedColor::Background));
    }

    /// 256 indexed와 truecolor RGB 색상도 round-trip되는지 별도 검증.
    #[test]
    fn snapshot_round_trips_indexed_and_truecolor() {
        // 256색 indexed 200을 fg로, truecolor (10,20,30)을 bg로.
        let stream: &[u8] = b"\x1b[38;5;200mX\x1b[0m\x1b[48;2;10;20;30mY\x1b[0m";

        let mut a = VirtualTerm::new(5, 10);
        a.feed(stream);
        let snap = a.snapshot();

        let mut b = VirtualTerm::new(5, 10);
        b.feed(&snap);

        let ca0 = &a.term.grid()[Line(0)][Column(0)];
        let cb0 = &b.term.grid()[Line(0)][Column(0)];
        assert_eq!(ca0.c, 'X');
        assert_eq!(cb0.c, 'X');
        assert_eq!(ca0.fg, Color::Indexed(200));
        assert_eq!(cb0.fg, ca0.fg);

        let ca1 = &a.term.grid()[Line(0)][Column(1)];
        let cb1 = &b.term.grid()[Line(0)][Column(1)];
        assert_eq!(ca1.c, 'Y');
        assert_eq!(cb1.c, 'Y');
        assert_eq!(
            ca1.bg,
            Color::Spec(Rgb {
                r: 10,
                g: 20,
                b: 30
            })
        );
        assert_eq!(cb1.bg, ca1.bg);
    }
}
