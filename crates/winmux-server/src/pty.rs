//! `portable-pty` 위의 얇은 비동기 어댑터.
//!
//! 한 [`Pty`]는 한 셸 자식을 가지며, 다음 백그라운드 task를 함께 띄운다:
//!
//! - **reader**: ConPTY stdout/stderr → broadcast `Output(bytes)`.
//! - **writer**: mpsc `Vec<u8>` 큐 → ConPTY stdin.
//! - **waiter**: child wait → broadcast `Exited { code }`.
//!
//! [`Pty::subscribe`]를 호출한 모든 task는 동일한 [`PtyEvent`] 스트림을 받는다.
//! Drop 시 자식은 best-effort kill되고, server-wide Job Object가 잔여 자식의
//! 정리를 OS 레벨에서 보증한다 (`docs/spec/02-pty-and-terminal.md` § Child lifetime).

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::jobobj::JobObject;

/// 한 번 spawn에 필요한 모든 입력.
#[derive(Clone, Debug)]
pub struct SpawnRequest {
    /// 셸 실행 파일 경로 (예: `pwsh.exe`).
    pub shell: PathBuf,
    /// 셸 인자.
    pub args: Vec<String>,
    /// 초기 작업 디렉터리. None이면 server 작업 디렉터리 상속.
    pub cwd: Option<PathBuf>,
    /// 추가 환경 변수(부모 환경 위에 덧붙임).
    pub env: std::collections::BTreeMap<String, String>,
    /// 초기 행 수.
    pub rows: u16,
    /// 초기 열 수.
    pub cols: u16,
}

/// pane이 발생시키는 비동기 이벤트.
#[derive(Clone, Debug)]
pub enum PtyEvent {
    /// 셸 stdout/stderr에서 읽은 바이트.
    Output(Vec<u8>),
    /// 셸이 종료됐다. 매우 짧은 시점에 한 번 발송된 뒤 broadcast가 닫힌다.
    Exited {
        /// 자식이 보고한 종료 코드. wait 실패 시 None.
        code: Option<i32>,
    },
}

const READ_BUF: usize = 64 * 1024;
const INPUT_QUEUE: usize = 64;
const EVENT_QUEUE: usize = 256;

/// 한 셸 자식 + 그 자식과 통신하는 task 모음.
pub struct Pty {
    /// resize 시 짧게 잠근다.
    master: Arc<StdMutex<Box<dyn MasterPty + Send>>>,
    /// stdin 큐 송신부. mpsc — backpressure로 부하 제한.
    input_tx: mpsc::Sender<Vec<u8>>,
    /// PtyEvent broadcast 송신부.
    event_tx: broadcast::Sender<PtyEvent>,
    /// 자식 OS PID. 알 수 없으면 None.
    pub pid: Option<u32>,
    /// Drop 시 abort할 backing task 핸들.
    tasks: Vec<JoinHandle<()>>,
    /// 자식 kill helper. Drop 또는 명시적 kill에서 한 번 호출된다.
    killer: StdMutex<Option<Box<dyn ChildKiller + Send + Sync>>>,
}

impl Pty {
    /// 새 셸을 spawn한다. `job`이 주어지면 자식 PID를 그 Job에 부여한다.
    pub fn spawn(req: SpawnRequest, job: Option<&JobObject>) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: req.rows,
                cols: req.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("openpty")?;

        let mut cmd = CommandBuilder::new(&req.shell);
        for a in &req.args {
            cmd.arg(a);
        }
        if let Some(cwd) = &req.cwd {
            cmd.cwd(cwd);
        }
        for (k, v) in &req.env {
            cmd.env(k, v);
        }

        let mut child = pair.slave.spawn_command(cmd).context("spawn_command")?;
        // slave handle은 부모에 더 이상 필요 없다.
        drop(pair.slave);

        let pid = child.process_id();
        if let (Some(pid), Some(job)) = (pid, job)
            && let Err(e) = job.assign_pid(pid)
        {
            warn!(error = %e, pid, "pty.job_assign.failed");
        }

        let writer = pair.master.take_writer().context("take_writer")?;
        let reader = pair.master.try_clone_reader().context("try_clone_reader")?;
        let killer = child.clone_killer();
        let master = Arc::new(StdMutex::new(pair.master));

        let (event_tx, _) = broadcast::channel::<PtyEvent>(EVENT_QUEUE);
        let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(INPUT_QUEUE);

        // Reader task — `MasterPty::read`는 blocking이므로 spawn_blocking.
        let reader_event_tx = event_tx.clone();
        let reader_task = tokio::task::spawn_blocking(move || {
            let mut reader = reader;
            let mut buf = vec![0u8; READ_BUF];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        debug!(error = %e, "pty.reader.error");
                        break;
                    }
                };
                if reader_event_tx
                    .send(PtyEvent::Output(buf[..n].to_vec()))
                    .is_err()
                {
                    break;
                }
            }
            debug!("pty.reader.exiting");
        });

        // Writer task — write_all 역시 blocking이다.
        let writer_task = tokio::task::spawn_blocking(move || {
            let mut writer = writer;
            while let Some(bytes) = input_rx.blocking_recv() {
                if let Err(e) = writer.write_all(&bytes) {
                    warn!(error = %e, "pty.writer.write_error");
                    break;
                }
                if let Err(e) = writer.flush() {
                    warn!(error = %e, "pty.writer.flush_error");
                    break;
                }
            }
            debug!("pty.writer.exiting");
        });

        // Waiter task — Child::wait도 blocking.
        let wait_event_tx = event_tx.clone();
        let wait_task = tokio::task::spawn_blocking(move || match child.wait() {
            Ok(status) => {
                let code = i32::try_from(status.exit_code()).unwrap_or(-1);
                info!(code, "pty.child.exited");
                let _ = wait_event_tx.send(PtyEvent::Exited { code: Some(code) });
            }
            Err(e) => {
                warn!(error = %e, "pty.child.wait_failed");
                let _ = wait_event_tx.send(PtyEvent::Exited { code: None });
            }
        });

        Ok(Self {
            master,
            input_tx,
            event_tx,
            pid,
            tasks: vec![reader_task, writer_task, wait_task],
            killer: StdMutex::new(Some(killer)),
        })
    }

    /// PtyEvent broadcast의 새 receiver. 한 번 받으면 다음 send부터 본다.
    pub fn subscribe(&self) -> broadcast::Receiver<PtyEvent> {
        self.event_tx.subscribe()
    }

    /// stdin에 바이트를 큐잉한다. mpsc backpressure 안에서만 await한다.
    pub async fn write_input(&self, bytes: Vec<u8>) -> Result<()> {
        self.input_tx
            .send(bytes)
            .await
            .map_err(|_| anyhow::anyhow!("pty input channel closed"))
    }

    /// ConPTY 크기를 변경한다.
    pub fn try_resize(&self, rows: u16, cols: u16) -> Result<()> {
        let m = self.master.lock().unwrap_or_else(|e| e.into_inner());
        m.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("resize")
    }

    /// 자식을 강제 종료한다. 멱등 — 두 번째 호출은 무시된다.
    pub fn kill(&self) {
        let mut guard = self.killer.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut k) = guard.take()
            && let Err(e) = k.kill()
        {
            debug!(error = %e, "pty.kill.error");
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.kill();
        for t in &self.tasks {
            t.abort();
        }
    }
}
