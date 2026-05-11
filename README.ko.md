# WinMux

> tmux for Windows, without WSL.

WSL 없이 Windows에서 동작하는 네이티브 터미널 멀티플렉서.
PowerShell, cmd 그리고 본인이 설치한 어떤 셸이든 tmux 스타일의
세션 영속성과 윈도우/페인 관리, 스크립팅 가능한 터미널 제어를 제공합니다.

[English README](README.md)

![License: MIT](https://img.shields.io/badge/License-MIT-yellow?style=flat-square)
![Platform: Windows 11](https://img.shields.io/badge/Platform-Windows_11-0078D6?style=flat-square&logo=windows11&logoColor=white)
![Tauri 2.x](https://img.shields.io/badge/Tauri-2.x-24C8DB?style=flat-square&logo=tauri&logoColor=white)
![Rust stable](https://img.shields.io/badge/Rust-stable-000000?style=flat-square&logo=rust&logoColor=white)
![Authored by Claude Code](https://img.shields.io/badge/Authored_by-Claude_Code-D97757?style=flat-square&logo=anthropic&logoColor=white)

---

## 왜 WinMux인가

tmux는 훌륭하지만 Windows에서는 WSL이나 Cygwin 뒤에서만 동작합니다.
PowerShell이나 cmd, 또는 직접 설치한 Git Bash로 일하는 사용자에게
tmux의 핵심 가치 — *터미널을 닫아도 작업이 죽지 않는다* — 는 그림의 떡입니다.

WinMux는 셸 세션을 소유하는 작은 백그라운드 프로세스로 동작합니다.
GUI는 켜졌다 꺼졌다 할 수 있습니다. 노트북을 슬립해도 됩니다.
페인은 계속 돌아갑니다. 돌아오면 다시 attach해서 멈춘 자리부터 이어갑니다.

WinMux는 tmux와 100% 호환을 목표로 하지 않습니다.
**tmux의 핵심 경험을 Windows답게** 구현하는 것을 목표로 합니다.

---

## 아키텍처 한눈에

```
사용자 ── 트레이 아이콘 ──► winmux-tray.exe ─┐
                                             │  Named Pipe
사용자 ── 셸 ────────────► winmux.exe (CLI) ─┤  \\.\pipe\winmux-<user>
                                             │
                                             ▼
                                    winmux-server.exe ── ConPTY ──► PowerShell / cmd / ...
                                    (백그라운드, PTY 소유)
```

세 개의 프로세스가 사용자별 Named Pipe로 통신합니다.
서버가 셸을 소유하고, 트레이가 GUI를 제공하고,
CLI는 스크립트와 임시 attach를 담당합니다.

전체 모델은 [`docs/spec/00-overview.md`](docs/spec/00-overview.md) 참조.

---

## 상태

Pre-alpha. 사양과 설계는 안정됐고 구현이 진행 중입니다.

| 마일스톤 | 동작 항목 |
| --- | --- |
| M0 — PoC | 서버 spawn, 단일 ConPTY, attach/detach, 화면 복원 |
| M1 — MVP | 다중 세션/윈도우/페인, prefix 키 바인딩, 기본 `.tmux.conf` |
| M2 — 호환성 | `winmux` CLI, copy mode, 추가 `.tmux.conf` 기능 |
| M3 — 영속성 | 세션 직렬화, 자동 시작, 트레이 마무리 |
| M4 — 고급 | 다중 클라이언트 attach, hooks, 플러그인 (TBD) |

마일스톤이 완료될 때마다 이 README가 업데이트됩니다.

---

## 저작 기록

> **이 프로젝트의 모든 코드와 문서는
> [Claude Code](https://www.anthropic.com/claude-code)가 작성·유지보수합니다.**
> 인간 협업자(`cx8537`)는 요구사항 정의, 사양 결정, 사용자 테스트,
> 방향 검토를 담당합니다.

| 역할 | 담당 |
| --- | --- |
| 코드, 리팩토링, 유지보수 | Claude Code |
| 모든 문서 (`README.md`, `docs/**`, `CLAUDE.md`) | Claude Code |
| 사양, 요구사항, UX 결정, 검수 | cx8537 (인간) |
| 라이선스 및 저작권 | cx8537 |

모든 커밋에 `Co-Authored-By: Claude` 트레일러가 포함됩니다.

---

## 주요 기능 (계획 및 구현)

- **Windows 네이티브.** WSL/Cygwin 불필요. PowerShell 7, Windows
  PowerShell, cmd, 사용자 지정 셸 모두 지원.
- **GUI 재시작에도 살아남는 세션.** 트레이를 닫았다 다시 열어도
  셸은 그대로 살아 있습니다.
- **tmux 스타일 prefix 키 바인딩.** 기본 `Ctrl+B`
  (또는 `.tmux.conf`에서 지정한 값).
- **다중 세션, 윈도우, 페인.** 분할, 리사이즈, 익숙한 키로 이동.
- **Windows 클립보드 통합.** 마우스로 텍스트 선택, `Ctrl+C`로 복사
  (선택 영역 있을 때) 또는 `SIGINT` (없을 때) — Windows 관례 그대로.
- **트레이 아이콘 백그라운드 데몬.** OneDrive 스타일. 윈도우를 닫아도
  작업은 계속됩니다.
- **`winmux` CLI로 자동화.** `winmux ls`, `winmux attach`,
  `winmux send-keys` — PowerShell에서 스크립팅 가능.
- **영어/한국어 UI.** 시스템 언어 자동 선택.
- **텔레메트리 없음. 자동 업데이트 없음.** 설치 후 본인 소유.

---

## 기술 스택

**앱 셸:** Tauri 2.x

**프론트엔드:** Vite, React 19, TypeScript, Tailwind, shadcn/ui,
Zustand, Zod, react-i18next, xterm.js v6

**백엔드 (Rust):** Tokio, `portable-pty`, `alacritty_terminal`,
`russh`, `tracing`, `rusqlite`

**IPC:** Windows Named Pipes, JSON Lines 프로토콜

전체 셋업은 [`docs/build/dev-setup.md`](docs/build/dev-setup.md) 참조.

---

## 빠른 시작

> WinMux는 pre-alpha 단계입니다. 아래는 소스에서 빌드하려는 개발자용 안내입니다.

```powershell
# 사전 요구사항: Node.js 20+, Rust stable, Windows 11
git clone https://github.com/cx8537/WinMux.git
cd WinMux
npm install
npm run tauri dev
```

릴리스 빌드와 인스톨러는 [`docs/build/release.md`](docs/build/release.md) 참조.

---

## 프로젝트 구조

```
WinMux/
├── CLAUDE.md                  # Claude Code 작업 원칙
├── README.md                  # 영어 README
├── README.ko.md               # 이 파일
├── SECURITY.md                # 보안 이슈 보고
├── CONTRIBUTING.md            # 기여 가이드
├── CHANGELOG.md               # 사용자 시각 변경 사항
├── LICENSE                    # MIT
├── crates/
│   ├── winmux-protocol/       # 공유 IPC 타입
│   ├── winmux-server/         # 백그라운드 서버 (GUI 의존성 없음)
│   ├── winmux-tray/           # 트레이 + Tauri GUI
│   └── winmux-cli/            # CLI 클라이언트
├── src/                       # 프론트엔드 (React + xterm.js)
├── src-tauri/                 # winmux-tray의 Tauri 셸
└── docs/                      # 모든 사양과 컨벤션
    ├── INDEX.md
    ├── decisions.md
    ├── known-issues.md
    ├── spec/                  # 기능 사양
    ├── conventions/           # 코드 스타일, 네이밍, 깃
    ├── nonfunctional/         # 보안, 성능 등
    ├── build/                 # 개발 셋업, 릴리스
    └── ops/                   # 트러블슈팅, 수동 테스트
```

---

## 보안

WinMux는 공격 표면이 적지 않은 개발자 도구입니다.
백그라운드 데몬, Named Pipe IPC, ConPTY 자식 프로세스, `.tmux.conf` 파싱 등.

보안 모델은 [`docs/nonfunctional/security.md`](docs/nonfunctional/security.md)에 정리돼 있습니다.

취약점 보고는 [SECURITY.md](SECURITY.md) 참조.

**범위 내:** 같은 PC의 다른 사용자 계정으로부터의 격리,
Named Pipe impersonation 방지, 안전한 `.tmux.conf` 처리.

**범위 외:** Administrator/SYSTEM 권한 공격자, 물리 접근, OS 차원 취약점.

---

## 문서

| 문서 | 용도 |
| --- | --- |
| [`docs/INDEX.md`](docs/INDEX.md) | 전체 문서 목차 |
| [`docs/spec/00-overview.md`](docs/spec/00-overview.md) | 아키텍처, 3-프로세스 모델 |
| [`docs/spec/05-tmux-compat.md`](docs/spec/05-tmux-compat.md) | 어떤 tmux 기능을 지원하는가 |
| [`docs/decisions.md`](docs/decisions.md) | 주요 설계 결정과 이유 |
| [`docs/known-issues.md`](docs/known-issues.md) | 알려진 한계와 우회 방법 |
| [`CLAUDE.md`](CLAUDE.md) | Claude Code 작업 원칙 |

---

## 기여

현재 1인 프로젝트이지만 이슈와 PR은 환영합니다.
먼저 [CONTRIBUTING.md](CONTRIBUTING.md)를 읽어주세요.

응답 시간은 불규칙합니다.

---

## 라이선스

MIT. [LICENSE](LICENSE) 참조.

WinMux는 셸을 spawn하고 명령을 실행하는 도구입니다.
**무보증(AS IS) 조항** 하에 제공되며, 사용에 대한 책임은 사용자에게 있습니다.
