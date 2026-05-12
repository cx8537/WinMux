// 릴리스 빌드는 콘솔 창을 띄우지 않는다. 개발 빌드는 콘솔을 유지해
// `tracing`이나 보조 stdout/stderr 출력을 볼 수 있게 한다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! winmux-app — WinMux 트레이 프로세스를 감싸는 Tauri 호스트.
//!
//! 본 바이너리의 책임:
//! 1. `winmux-tray`의 `tracing` subscriber를 가장 먼저 설치한다.
//! 2. Tauri 런타임을 시작하고, 시스템 트레이 아이콘을 등록한다.
//! 3. `winmux-tray::start_ipc`로 Named Pipe IPC 매니저를 띄우고 그 핸들을
//!    Tauri State에 등록한다. webview가 `winmux_*` 명령을 호출하면 본
//!    매니저를 거쳐 server와 통신한다.
//! 4. 메인 윈도우의 닫기 버튼을 "숨기기"로 가로채 서버를 살려둔다
//!    (`docs/spec/00-overview.md` § Tray shutdown).
//! 5. 트레이 좌클릭과 "Open main window" 메뉴로 윈도우를 다시 띄운다.

use std::io::{self, Write};
use std::process::ExitCode;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

/// 메인 윈도우의 `label` (tauri.conf.json과 일치).
const MAIN_WINDOW_LABEL: &str = "main";
/// 트레이 메뉴 항목 ID.
const MENU_OPEN_MAIN: &str = "open_main";
const MENU_QUIT: &str = "quit";

fn main() -> ExitCode {
    winmux_tray::install_tracing();

    let result = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            winmux_tray::commands::winmux_server_status,
            winmux_tray::commands::winmux_ping,
            winmux_tray::commands::winmux_list_sessions,
            winmux_tray::commands::winmux_new_session,
            winmux_tray::commands::winmux_attach,
            winmux_tray::commands::winmux_detach,
            winmux_tray::commands::winmux_kill_session,
            winmux_tray::commands::winmux_pty_input,
            winmux_tray::commands::winmux_resize,
        ])
        .setup(setup)
        .on_window_event(handle_window_event)
        .run(tauri::generate_context!());

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(io::stderr(), "tauri runtime failed: {err}");
            ExitCode::from(1)
        }
    }
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // IPC 매니저를 백그라운드에서 시작하고, 그 핸들을 State로 등록한다.
    // 명령 핸들러는 `tauri::State<'_, IpcHandle>`로 꺼내 쓴다.
    let ipc_handle = winmux_tray::start_ipc(app.handle().clone())?;
    app.manage(ipc_handle);

    let open_main = MenuItem::with_id(app, MENU_OPEN_MAIN, "Open main window", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit WinMux", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open_main, &separator, &quit])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("default window icon is not available")?;

    TrayIconBuilder::with_id("winmux-tray")
        .tooltip("WinMux")
        .icon(icon)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN_MAIN => show_main_window(app),
            MENU_QUIT => app.exit(0),
            other => {
                // 등록되지 않은 메뉴 ID는 무시한다. 로그도 남기지 않는 게
                // 안전하다 — 사용자의 클릭 시점 정보는 행동학적 데이터에 해당.
                let _ = other;
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// 메인 윈도우를 다시 보이게 하고 포커스를 옮긴다.
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// 메인 윈도우의 닫기 요청을 가로채 "숨기기"로 바꾼다.
///
/// 서버 프로세스를 살려두기 위함 — 사용자는 트레이 메뉴의 "Quit WinMux"로만
/// 전체 종료를 요청할 수 있다 (`docs/spec/00-overview.md` § Tray shutdown).
fn handle_window_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW_LABEL {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let _ = window.hide();
    }
}
