//! Runs an [`Engine`] on a dedicated thread. PoB's Lua state is single-threaded
//! and a full recalculation can take tens of milliseconds, so callers talk to
//! it through a channel and the UI thread never blocks on Lua.

use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::{Engine, EngineConfig, Error, Result};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EngineState {
    Booting,
    Ready,
    Error,
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    pub state: EngineState,
    pub message: Option<String>,
    pub boot_ms: Option<u128>,
    pub pob_root: String,
    pub user_dir: String,
}

enum Job {
    Call {
        method: String,
        params: Value,
        reply: SyncSender<Result<CallOutcome>>,
    },
    Eval {
        code: String,
        reply: SyncSender<Result<Value>>,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallOutcome {
    pub result: Value,
    /// Wall time spent inside Lua for this call.
    pub elapsed_ms: f64,
}

#[derive(Clone)]
pub struct EngineHandle {
    tx: Sender<Job>,
    status: Arc<Mutex<EngineStatus>>,
    _thread: Arc<Option<JoinHandle<()>>>,
}

/// The queue must stay empty this long before background work runs, so it never sits between the calls of one burst.
const IDLE_AFTER: Duration = Duration::from_millis(200);

impl EngineHandle {
    /// Spawns the engine thread and starts booting PoB immediately.
    pub fn spawn(cfg: EngineConfig) -> Self {
        Self::start(cfg, false)
    }

    /// As `spawn`, and the thread runs the bridge's background work (the item database parse) while no call is waiting.
    pub fn spawn_with_idle_work(cfg: EngineConfig) -> Self {
        Self::start(cfg, true)
    }

    fn start(cfg: EngineConfig, idle_work: bool) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let status = Arc::new(Mutex::new(EngineStatus {
            state: EngineState::Booting,
            message: None,
            boot_ms: None,
            pob_root: cfg.pob_root.to_string_lossy().to_string(),
            user_dir: cfg.user_dir.to_string_lossy().to_string(),
        }));
        let status2 = status.clone();
        let thread = std::thread::Builder::new()
            .name("pob-engine".into())
            // PoB's mod parser and calc code recurse deeply; give Lua headroom.
            .stack_size(64 * 1024 * 1024)
            .spawn(move || run(cfg, rx, status2, idle_work))
            .expect("spawn pob-engine thread");
        Self { tx, status, _thread: Arc::new(Some(thread)) }
    }

    pub fn status(&self) -> EngineStatus {
        self.status.lock().unwrap().clone()
    }

    /// Blocks until the engine has finished booting (or failed).
    pub fn wait_ready(&self) -> Result<()> {
        loop {
            let st = self.status();
            match st.state {
                EngineState::Booting => std::thread::sleep(std::time::Duration::from_millis(20)),
                EngineState::Ready => return Ok(()),
                EngineState::Error | EngineState::Stopped => {
                    return Err(Error::NotRunning(st.message.unwrap_or_default()))
                }
            }
        }
    }

    /// Blocking call; safe to invoke from any thread. Use inside `spawn_blocking`
    /// from async contexts.
    pub fn call(&self, method: &str, params: Value) -> Result<CallOutcome> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(Job::Call { method: method.to_string(), params, reply })
            .map_err(|_| Error::NotRunning("engine thread is gone".into()))?;
        rx.recv()
            .map_err(|_| Error::NotRunning("engine thread dropped the request".into()))?
    }

    pub fn eval(&self, code: &str) -> Result<Value> {
        let (reply, rx) = sync_channel(1);
        self.tx
            .send(Job::Eval { code: code.to_string(), reply })
            .map_err(|_| Error::NotRunning("engine thread is gone".into()))?;
        rx.recv()
            .map_err(|_| Error::NotRunning("engine thread dropped the request".into()))?
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(Job::Shutdown);
    }
}

enum Next {
    Job(Job),
    Idle,
    Closed,
}

fn next(rx: &Receiver<Job>, work_left: bool, quiet: bool) -> Next {
    if !work_left {
        return rx.recv().map_or(Next::Closed, Next::Job);
    }
    if quiet {
        return match rx.try_recv() {
            Ok(job) => Next::Job(job),
            Err(TryRecvError::Empty) => Next::Idle,
            Err(TryRecvError::Disconnected) => Next::Closed,
        };
    }
    match rx.recv_timeout(IDLE_AFTER) {
        Ok(job) => Next::Job(job),
        Err(RecvTimeoutError::Timeout) => Next::Idle,
        Err(RecvTimeoutError::Disconnected) => Next::Closed,
    }
}

fn run(cfg: EngineConfig, rx: Receiver<Job>, status: Arc<Mutex<EngineStatus>>, idle_work: bool) {
    let engine = match Engine::boot(cfg) {
        Ok(e) => {
            let mut st = status.lock().unwrap();
            st.state = EngineState::Ready;
            st.boot_ms = Some(e.boot_ms);
            drop(st);
            e
        }
        Err(e) => {
            let msg = e.to_string();
            log::error!("pob-engine boot failed: {msg}");
            let mut st = status.lock().unwrap();
            st.state = EngineState::Error;
            st.message = Some(msg.clone());
            drop(st);
            // Drain requests so callers get a clear error instead of a hang.
            for job in rx {
                match job {
                    Job::Call { reply, .. } => {
                        let _ = reply.send(Err(Error::NotRunning(msg.clone())));
                    }
                    Job::Eval { reply, .. } => {
                        let _ = reply.send(Err(Error::NotRunning(msg.clone())));
                    }
                    Job::Shutdown => break,
                }
            }
            return;
        }
    };

    let mut work_left = idle_work && engine.hold_idle_work();
    let mut quiet = false;
    let mut spent = Duration::ZERO;
    loop {
        let job = match next(&rx, work_left, quiet) {
            Next::Job(job) => job,
            Next::Idle => {
                quiet = true;
                let t0 = Instant::now();
                work_left = engine.idle_step();
                spent += t0.elapsed();
                if !work_left {
                    log::info!("pob-engine finished its background work in {} ms", spent.as_millis());
                }
                continue;
            }
            Next::Closed => break,
        };
        quiet = false;
        match job {
            Job::Call { method, params, reply } => {
                let t0 = Instant::now();
                let res = engine.call(&method, &params).map(|result| CallOutcome {
                    result,
                    elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0,
                });
                if let Err(e) = &res {
                    log::warn!("engine.{method} failed: {e}");
                }
                let _ = reply.send(res);
            }
            Job::Eval { code, reply } => {
                let _ = reply.send(engine.eval(&code));
            }
            Job::Shutdown => break,
        }
    }
    status.lock().unwrap().state = EngineState::Stopped;
}
