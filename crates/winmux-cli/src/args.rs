//! `winmux` CLI 인자 정의 (clap derive).
//!
//! `docs/spec/06-cli.md`의 명령 카탈로그를 1:1로 옮긴 골격이다.
//! M0 단계에서는 인자 파싱만 검증한다 — 실제 IPC 호출과 결과 출력은
//! 후속 작업에서 단계적으로 채운다. 모든 명령이 등록되어 있어야
//! `winmux --help`가 의미 있는 도움말을 낸다.
//!
//! 글로벌 플래그(`--quiet`, `--json`, `--no-color`)는 `propagate_version`과
//! `global = true`로 어느 위치(명령 앞/뒤)에서나 받는다.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// 최상위 파서. `winmux <command> [args...]`.
#[derive(Debug, Parser)]
#[command(
    name = "winmux",
    version,
    about = "Windows-native terminal multiplexer CLI",
    propagate_version = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// 모든 명령에 공통으로 받는 글로벌 플래그.
    #[command(flatten)]
    pub global: GlobalFlags,

    /// 실행할 명령.
    #[command(subcommand)]
    pub command: Command,
}

/// 어느 명령에서나 받는 글로벌 플래그.
#[derive(Args, Debug, Clone)]
pub struct GlobalFlags {
    /// 비필수 출력을 억제한다.
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// 결과를 JSON으로 출력한다(스크립트용).
    #[arg(long, global = true)]
    pub json: bool,

    /// ANSI 색상을 끈다(non-TTY에서는 자동).
    #[arg(long, global = true)]
    pub no_color: bool,
}

/// 명령 집합. spec § Commands와 1:1.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// 세션 목록.
    Ls(LsArgs),

    /// 새 세션 생성.
    NewSession(NewSessionArgs),

    /// 세션에 어태치.
    Attach(AttachArgs),

    /// 현재 클라이언트 디태치.
    Detach,

    /// 세션 종료.
    KillSession(KillSessionArgs),

    /// 윈도우 종료.
    KillWindow(TargetOnlyArgs),

    /// 패널 종료.
    KillPane(TargetOnlyArgs),

    /// 키 전송.
    SendKeys(SendKeysArgs),

    /// 윈도우 목록.
    ListWindows(OptionalTargetArgs),

    /// 패널 목록.
    ListPanes(OptionalTargetArgs),

    /// 패널 분할.
    SplitWindow(SplitWindowArgs),

    /// 활성 패널 변경.
    SelectPane(SelectPaneArgs),

    /// 패널 크기 조정.
    ResizePane(ResizePaneArgs),

    /// 패널 출력 캡처.
    CapturePane(CapturePaneArgs),

    /// 클라이언트에 메시지 표시.
    DisplayMessage(DisplayMessageArgs),

    /// 설정 파일 재로드.
    SourceFile(SourceFileArgs),

    /// 효과 옵션 출력.
    ShowOptions(ShowOptionsArgs),

    /// 키 바인딩 등록.
    BindKey(BindKeyArgs),

    /// 키 바인딩 해제.
    UnbindKey(UnbindKeyArgs),

    /// 서버 종료 신호 전송.
    KillServer,

    /// 서버 명시적 시작.
    StartServer,

    /// 버전 정보 출력.
    Version,
}

/// `ls`.
#[derive(Args, Debug)]
pub struct LsArgs {
    /// 디태치된 세션까지 포함해 모두 표시.
    #[arg(long)]
    pub all: bool,
}

/// `new-session`.
#[derive(Args, Debug)]
pub struct NewSessionArgs {
    /// 세션 이름(미지정 시 서버가 부여).
    #[arg(short = 's', long = "session")]
    pub session: Option<String>,

    /// 세션만 만들고 이 클라이언트는 어태치하지 않는다.
    #[arg(short = 'd', long)]
    pub detached: bool,

    /// 첫 패널의 작업 디렉터리.
    #[arg(short = 'c', long = "cwd")]
    pub cwd: Option<PathBuf>,

    /// 셸 별칭/경로(미지정 시 설정 기본값).
    #[arg(long)]
    pub shell: Option<String>,

    /// `--` 뒤에 오는 셸 + 인자.
    #[arg(last = true)]
    pub shell_argv: Vec<String>,
}

/// `attach`.
#[derive(Args, Debug)]
pub struct AttachArgs {
    /// 대상 세션.
    #[arg(short = 't', long = "target")]
    pub target: Option<String>,

    /// 다른 클라이언트를 디태치시키고 자신만 남긴다.
    #[arg(short = 'd', long = "detach-others")]
    pub detach_others: bool,
}

/// `kill-session`.
#[derive(Args, Debug)]
pub struct KillSessionArgs {
    /// 종료할 세션.
    #[arg(short = 't', long = "target")]
    pub target: String,
}

/// `kill-window` / `kill-pane` 공통.
#[derive(Args, Debug)]
pub struct TargetOnlyArgs {
    /// 대상(`session:window` 또는 `session:window.pane`).
    #[arg(short = 't', long = "target")]
    pub target: String,
}

/// `list-windows` / `list-panes` 공통. 타깃은 선택.
#[derive(Args, Debug)]
pub struct OptionalTargetArgs {
    /// 대상 세션/윈도우. 미지정 시 "현재" 컨텍스트.
    #[arg(short = 't', long = "target")]
    pub target: Option<String>,
}

/// `send-keys`.
#[derive(Args, Debug)]
pub struct SendKeysArgs {
    /// 대상 패널.
    #[arg(short = 't', long = "target")]
    pub target: Option<String>,

    /// 보낼 키(여러 인자 가능). tmux 스타일 키 이름(`Enter`, `C-c`, …).
    #[arg(required = true)]
    pub keys: Vec<String>,
}

/// `split-window`.
///
/// tmux와 spec에 따라 `-h`는 horizontal split이다. clap이 자동 생성하는
/// `-h`(=`--help`) short와 충돌하므로 이 명령에서만 자동 help flag를 끄고
/// `--help` long만 명시적으로 다시 추가한다.
#[derive(Args, Debug)]
#[command(disable_help_flag = true)]
pub struct SplitWindowArgs {
    /// 대상 패널.
    #[arg(short = 't', long = "target")]
    pub target: Option<String>,

    /// 가로 분할(왼쪽·오른쪽). `-v`와 동시 사용 불가.
    #[arg(short = 'h', long, conflicts_with = "vertical")]
    pub horizontal: bool,

    /// 세로 분할(위·아래). 기본값.
    #[arg(short = 'v', long, conflicts_with = "horizontal")]
    pub vertical: bool,

    /// 새 패널의 크기 비율(1..=99).
    #[arg(short = 'p', long = "percent")]
    pub percent: Option<u8>,

    /// 새 패널의 초기 cwd.
    #[arg(short = 'c', long)]
    pub cwd: Option<PathBuf>,

    /// 도움말을 출력하고 종료한다.
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,
}

/// `select-pane`.
#[derive(Args, Debug)]
pub struct SelectPaneArgs {
    /// 명시적 패널 타깃. 방향 플래그와 동시에 줄 수 있지만 우선순위는 타깃.
    #[arg(short = 't', long = "target")]
    pub target: Option<String>,

    /// 왼쪽 패널로 이동.
    #[arg(short = 'L', long)]
    pub left: bool,

    /// 오른쪽 패널로 이동.
    #[arg(short = 'R', long)]
    pub right: bool,

    /// 위 패널로 이동.
    #[arg(short = 'U', long)]
    pub up: bool,

    /// 아래 패널로 이동.
    #[arg(short = 'D', long)]
    pub down: bool,
}

/// `resize-pane`. 방향 + N, 또는 zoom toggle.
#[derive(Args, Debug)]
pub struct ResizePaneArgs {
    /// 대상 패널.
    #[arg(short = 't', long = "target")]
    pub target: Option<String>,

    /// 왼쪽으로 N셀 줄임(또는 늘림).
    #[arg(short = 'L', long, value_name = "N")]
    pub left: Option<u16>,

    /// 오른쪽으로 N셀.
    #[arg(short = 'R', long, value_name = "N")]
    pub right: Option<u16>,

    /// 위로 N셀.
    #[arg(short = 'U', long, value_name = "N")]
    pub up: Option<u16>,

    /// 아래로 N셀.
    #[arg(short = 'D', long, value_name = "N")]
    pub down: Option<u16>,

    /// Zoom 토글.
    #[arg(short = 'Z', long)]
    pub zoom: bool,
}

/// `capture-pane`.
#[derive(Args, Debug)]
pub struct CapturePaneArgs {
    /// 대상 패널.
    #[arg(short = 't', long = "target")]
    pub target: Option<String>,

    /// stdout에 즉시 출력.
    #[arg(short = 'p', long)]
    pub print: bool,

    /// 시작 라인(음수 = 스크롤백 위쪽 상대).
    #[arg(short = 'S', long, value_name = "START")]
    pub start: Option<i32>,

    /// 끝 라인(음수 = 아래쪽 상대).
    #[arg(short = 'E', long, value_name = "END")]
    pub end: Option<i32>,
}

/// `display-message`.
#[derive(Args, Debug)]
pub struct DisplayMessageArgs {
    /// 대상 클라이언트.
    #[arg(short = 't', long = "target")]
    pub target: Option<String>,

    /// 표시할 메시지 본문.
    pub message: String,
}

/// `source-file`.
#[derive(Args, Debug)]
pub struct SourceFileArgs {
    /// 읽을 설정 파일 경로.
    pub path: PathBuf,
}

/// `show-options`.
#[derive(Args, Debug)]
pub struct ShowOptionsArgs {
    /// 글로벌 옵션을 표시.
    #[arg(short = 'g', long)]
    pub global: bool,
}

/// `bind-key`.
#[derive(Args, Debug)]
pub struct BindKeyArgs {
    /// 바인딩할 키(예: `C-b c`).
    pub key: String,

    /// 실행할 명령과 인자.
    #[arg(required = true)]
    pub command: Vec<String>,
}

/// `unbind-key`.
#[derive(Args, Debug)]
pub struct UnbindKeyArgs {
    /// 해제할 키.
    pub key: String,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn parse(argv: &[&str]) -> Cli {
        Cli::try_parse_from(argv).expect("parse")
    }

    #[test]
    fn ls_default_has_all_false() {
        let cli = parse(&["winmux", "ls"]);
        match cli.command {
            Command::Ls(LsArgs { all }) => assert!(!all),
            _ => panic!("expected Ls"),
        }
    }

    #[test]
    fn ls_with_all_flag() {
        let cli = parse(&["winmux", "ls", "--all"]);
        match cli.command {
            Command::Ls(LsArgs { all }) => assert!(all),
            _ => panic!("expected Ls"),
        }
    }

    #[test]
    fn new_session_parses_name_and_inline_shell() {
        let cli = parse(&[
            "winmux",
            "new-session",
            "-s",
            "wsl",
            "-d",
            "--",
            "wsl.exe",
            "-d",
            "Ubuntu",
        ]);
        match cli.command {
            Command::NewSession(args) => {
                assert_eq!(args.session.as_deref(), Some("wsl"));
                assert!(args.detached);
                assert_eq!(args.shell_argv, vec!["wsl.exe", "-d", "Ubuntu"]);
            }
            _ => panic!("expected NewSession"),
        }
    }

    #[test]
    fn send_keys_collects_multiple_keys() {
        let cli = parse(&["winmux", "send-keys", "-t", "work:0.0", "echo hi", "Enter"]);
        match cli.command {
            Command::SendKeys(args) => {
                assert_eq!(args.target.as_deref(), Some("work:0.0"));
                assert_eq!(args.keys, vec!["echo hi", "Enter"]);
            }
            _ => panic!("expected SendKeys"),
        }
    }

    #[test]
    fn send_keys_requires_at_least_one_key() {
        let r = Cli::try_parse_from(["winmux", "send-keys", "-t", "work:0.0"]);
        assert!(r.is_err(), "must require at least one key");
    }

    #[test]
    fn split_window_horizontal_and_vertical_conflict() {
        let r = Cli::try_parse_from(["winmux", "split-window", "-h", "-v"]);
        assert!(r.is_err(), "-h and -v must conflict");
    }

    #[test]
    fn resize_pane_zoom_only() {
        let cli = parse(&["winmux", "resize-pane", "-Z"]);
        match cli.command {
            Command::ResizePane(args) => {
                assert!(args.zoom);
                assert_eq!(args.left, None);
            }
            _ => panic!("expected ResizePane"),
        }
    }

    #[test]
    fn no_subcommand_is_an_error() {
        let r = Cli::try_parse_from(["winmux"]);
        assert!(r.is_err(), "subcommand required");
    }

    #[test]
    fn global_flags_propagate_after_subcommand() {
        let cli = parse(&["winmux", "ls", "--json", "--quiet", "--no-color"]);
        assert!(cli.global.json);
        assert!(cli.global.quiet);
        assert!(cli.global.no_color);
        assert!(matches!(cli.command, Command::Ls(_)));
    }
}
