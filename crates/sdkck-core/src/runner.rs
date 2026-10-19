//! Command execution.
//!
//! * **Cold** — spawn `node <sdkck>/bin/run.js <argv…>`, exactly what typing
//!   `sdkck …` in a terminal does. Always correct; pays Node's boot and oclif's
//!   module loading on every run.
//! * **Warm** — hand the argv to a pre-booted worker (`worker/worker.mjs`) that
//!   keeps sdkck's module graph loaded and runs the invocation in-process via
//!   oclif's `run()`. Same hooks, same output, a fraction of the latency.
//!
//! The caller decides per run whether warm is allowed; a busy, booting or dead
//! worker silently falls back to cold, so a run is never refused.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::runtime::SdkckInfo;

const PROTOCOL_PREFIX: &str = "\u{1e}SDKCKD ";
pub const WORKER_SOURCE: &str = include_str!("../worker/worker.mjs");
const WORKER_BOOT_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    Warm,
    Cold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RunEvent {
    #[serde(rename_all = "camelCase")]
    Started { run_id: u64, mode: RunMode, argv: Vec<String> },
    #[serde(rename_all = "camelCase")]
    Output { run_id: u64, stream: Stream, data: String },
    #[serde(rename_all = "camelCase")]
    Finished { run_id: u64, code: i32, duration_ms: u64, mode: RunMode, cancelled: bool },
}

pub type Sink = Arc<dyn Fn(RunEvent) + Send + Sync>;

#[derive(Clone, Debug)]
pub struct RunnerConfig {
    pub node: PathBuf,
    pub sdkck: SdkckInfo,
    /// The complete environment children start with.
    pub env: BTreeMap<String, String>,
    /// Where to materialise the worker script.
    pub scratch_dir: PathBuf,
}

impl RunnerConfig {
    fn command(&self, args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> Command {
        let mut cmd = Command::new(&self.node);
        cmd.args(args)
            .env_clear()
            .envs(&self.env)
            // Output is shown in a GUI, not a terminal.
            .env("FORCE_COLOR", "0")
            .env("NO_COLOR", "1")
            .current_dir(&self.sdkck.root);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        // Its own process group, so Stop can take down everything it spawned.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Kill a child and every process it started (plugins spawn MCP servers,
/// git, database clients…), not just the direct Node process.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        // The child leads its own process group (see `RunnerConfig::command`).
        // SAFETY: plain syscall on a pid we spawned; a stale group is a no-op.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// How long a finished command's output pipes may stay open — held by a
/// descendant that outlived it — before the run is reported finished anyway.
const PIPE_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerState {
    Off,
    Booting,
    Ready,
    Busy,
    Failed,
}

struct Active {
    run_id: u64,
    sink: Sink,
    started: Instant,
    cancelled: bool,
}

#[derive(Default)]
struct Shared {
    ready: bool,
    dead: bool,
    active: Option<Active>,
}

struct Worker {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    shared: Arc<(Mutex<Shared>, Condvar)>,
}

impl Worker {
    fn spawn(cfg: &RunnerConfig) -> std::io::Result<Arc<Worker>> {
        std::fs::create_dir_all(&cfg.scratch_dir)?;
        let script = cfg.scratch_dir.join("worker.mjs");
        if std::fs::read_to_string(&script).ok().as_deref() != Some(WORKER_SOURCE) {
            std::fs::write(&script, WORKER_SOURCE)?;
        }
        let mut child = cfg
            .command([script.as_os_str(), cfg.sdkck.root.as_os_str()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let shared = Arc::new((Mutex::new(Shared::default()), Condvar::new()));

        let s = shared.clone();
        std::thread::spawn(move || read_protocol(stdout, &s));
        let s = shared.clone();
        std::thread::spawn(move || {
            // Real stderr only carries output when something bypasses the
            // in-process capture (e.g. a subprocess inheriting it).
            for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
                let target = s.0.lock().unwrap().active.as_ref().map(|a| (a.sink.clone(), a.run_id));
                if let Some((sink, run_id)) = target {
                    sink(RunEvent::Output { run_id, stream: Stream::Stderr, data: format!("{}\n", crate::ansi::strip(&line)) });
                }
            }
        });

        Ok(Arc::new(Worker { child: Mutex::new(child), stdin: Mutex::new(stdin), shared }))
    }

    fn wait_ready(&self, timeout: Duration) -> bool {
        let (lock, cv) = &*self.shared;
        let guard = lock.lock().unwrap();
        let (guard, _) = cv.wait_timeout_while(guard, timeout, |s| !s.ready && !s.dead).unwrap();
        guard.ready && !guard.dead
    }

    fn state(&self) -> WorkerState {
        let guard = self.shared.0.lock().unwrap();
        match (guard.dead, guard.ready, guard.active.is_some()) {
            (true, ..) => WorkerState::Failed,
            (_, false, _) => WorkerState::Booting,
            (_, true, true) => WorkerState::Busy,
            _ => WorkerState::Ready,
        }
    }

    /// Claim the worker for one run. `false` when it is not idle.
    fn try_run(&self, run_id: u64, argv: &[String], sink: &Sink) -> bool {
        {
            let mut guard = self.shared.0.lock().unwrap();
            if !guard.ready || guard.dead || guard.active.is_some() {
                return false;
            }
            // Register before writing so no output can arrive unrouted, and emit
            // Started first so it precedes every Output event.
            sink(RunEvent::Started { run_id, mode: RunMode::Warm, argv: argv.to_vec() });
            guard.active = Some(Active { run_id, sink: sink.clone(), started: Instant::now(), cancelled: false });
        }
        let line = serde_json::json!({"id": run_id, "argv": argv}).to_string() + "\n";
        if self.stdin.lock().unwrap().write_all(line.as_bytes()).is_err() {
            // The reader thread sees EOF and finishes the run as failed.
            kill_tree(&mut self.child.lock().unwrap());
        }
        true
    }

    fn kill(&self) {
        if let Some(active) = self.shared.0.lock().unwrap().active.as_mut() {
            active.cancelled = true;
        }
        kill_tree(&mut self.child.lock().unwrap());
    }

    fn is_running(&self, run_id: u64) -> bool {
        self.shared.0.lock().unwrap().active.as_ref().is_some_and(|a| a.run_id == run_id)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Ok(c) = self.child.get_mut() {
            kill_tree(c);
        }
    }
}

fn read_protocol(stdout: std::process::ChildStdout, shared: &Arc<(Mutex<Shared>, Condvar)>) {
    let (lock, cv) = &**shared;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // Sinks run outside the lock: they may call back into the runner.
        let emit: Vec<(Sink, RunEvent)> = {
            let mut guard = lock.lock().unwrap();
            match line.strip_prefix(PROTOCOL_PREFIX) {
                // Raw stdout from something that bypassed capture.
                None => guard
                    .active
                    .as_ref()
                    .map(|a| {
                        (
                            a.sink.clone(),
                            RunEvent::Output { run_id: a.run_id, stream: Stream::Stdout, data: crate::ansi::strip(&line) },
                        )
                    })
                    .into_iter()
                    .collect(),
                Some(json) => {
                    let msg = serde_json::from_str::<serde_json::Value>(json).unwrap_or_default();
                    let matches = |a: &Active| Some(a.run_id) == msg["id"].as_u64();
                    match msg["t"].as_str() {
                        Some("ready") => {
                            guard.ready = true;
                            cv.notify_all();
                            vec![]
                        }
                        Some("out") => guard
                            .active
                            .as_ref()
                            .filter(|a| matches(a))
                            .map(|a| {
                                let stream = if msg["s"] == "stderr" { Stream::Stderr } else { Stream::Stdout };
                                let data = crate::ansi::strip(msg["d"].as_str().unwrap_or_default());
                                (a.sink.clone(), RunEvent::Output { run_id: a.run_id, stream, data })
                            })
                            .into_iter()
                            .collect(),
                        Some("exit") if guard.active.as_ref().is_some_and(matches) => {
                            let a = guard.active.take().unwrap();
                            let code = msg["code"].as_i64().unwrap_or(1) as i32;
                            let duration_ms = a.started.elapsed().as_millis() as u64;
                            vec![(
                                a.sink,
                                RunEvent::Finished { run_id: a.run_id, code, duration_ms, mode: RunMode::Warm, cancelled: false },
                            )]
                        }
                        Some("fatal") => {
                            guard.dead = true;
                            vec![]
                        }
                        _ => vec![],
                    }
                }
            }
        };
        for (sink, event) in emit {
            sink(event);
        }
    }
    let active = {
        let mut guard = lock.lock().unwrap();
        guard.dead = true;
        cv.notify_all();
        guard.active.take()
    };
    if let Some(a) = active {
        if !a.cancelled {
            (a.sink)(RunEvent::Output {
                run_id: a.run_id,
                stream: Stream::Stderr,
                data: "The warm runner exited unexpectedly; it will restart on the next run.\n".into(),
            });
        }
        let code = if a.cancelled { 130 } else { 1 };
        (a.sink)(RunEvent::Finished {
            run_id: a.run_id,
            code,
            duration_ms: a.started.elapsed().as_millis() as u64,
            mode: RunMode::Warm,
            cancelled: a.cancelled,
        });
    }
}

#[derive(Default)]
pub struct Runner {
    config: Mutex<Option<RunnerConfig>>,
    worker: Mutex<Option<Arc<Worker>>>,
    /// Recycled workers still finishing a run; kept reachable so Stop works.
    retiring: Arc<Mutex<Vec<Arc<Worker>>>>,
    cold: Arc<Mutex<HashMap<u64, Arc<Mutex<Child>>>>>,
    cancelled: Arc<Mutex<std::collections::HashSet<u64>>>,
}

impl Runner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the runtime configuration. Drops any worker started under the old one.
    pub fn configure(&self, config: Option<RunnerConfig>) {
        *self.config.lock().unwrap() = config;
        self.recycle();
    }

    pub fn is_configured(&self) -> bool {
        self.config.lock().unwrap().is_some()
    }

    /// Stop the worker; the next warm run (or `prewarm`) starts a fresh one,
    /// picking up plugins and stores that changed since it booted.
    pub fn recycle(&self) {
        if let Some(w) = self.worker.lock().unwrap().take() {
            if w.shared.0.lock().unwrap().active.is_none() {
                w.kill();
            } else {
                // Let the in-flight run finish; just stop routing new ones to it.
                self.retiring.lock().unwrap().push(w.clone());
                let retiring = self.retiring.clone();
                std::thread::spawn(move || {
                    while w.shared.0.lock().unwrap().active.is_some() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    retiring.lock().unwrap().retain(|r| !Arc::ptr_eq(r, &w));
                    w.kill();
                });
            }
        }
    }

    pub fn worker_state(&self) -> WorkerState {
        self.worker.lock().unwrap().as_ref().map_or(WorkerState::Off, |w| w.state())
    }

    /// Start the worker in the background so the first warm run is instant.
    pub fn prewarm(self: &Arc<Self>) {
        let this = self.clone();
        std::thread::spawn(move || {
            this.ensure_worker();
        });
    }

    fn ensure_worker(&self) -> Option<Arc<Worker>> {
        let cfg = self.config.lock().unwrap().clone()?;
        let worker = {
            let mut slot = self.worker.lock().unwrap();
            match slot.as_ref() {
                Some(w) if !w.shared.0.lock().unwrap().dead => w.clone(),
                _ => {
                    let w = Worker::spawn(&cfg).ok()?;
                    *slot = Some(w.clone());
                    w
                }
            }
        };
        worker.wait_ready(WORKER_BOOT_TIMEOUT).then_some(worker)
    }

    /// Start `argv` (the full sdkck argv, command id parts first). Events go to
    /// `sink` from a background thread; this returns once the run is started.
    pub fn run(self: &Arc<Self>, run_id: u64, argv: Vec<String>, allow_warm: bool, sink: Sink) -> crate::Result<()> {
        let cfg = self.config.lock().unwrap().clone().ok_or_else(|| crate::Error::Runtime("sdkck is not set up yet".into()))?;
        if allow_warm {
            // Only use a worker that is already up; never make a run wait on a boot.
            let ready = self.worker.lock().unwrap().clone().filter(|w| w.state() == WorkerState::Ready);
            if let Some(w) = ready {
                if w.try_run(run_id, &argv, &sink) {
                    return Ok(());
                }
            } else if self.worker_state() == WorkerState::Off || self.worker_state() == WorkerState::Failed {
                self.prewarm();
            }
        }
        self.run_cold(cfg, run_id, argv, sink)
    }

    fn run_cold(&self, cfg: RunnerConfig, run_id: u64, argv: Vec<String>, sink: Sink) -> crate::Result<()> {
        let mut child = cfg
            .command(std::iter::once(cfg.sdkck.run_js().into_os_string()).chain(argv.iter().map(Into::into)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| crate::Error::Runtime(format!("Could not start node: {e}")))?;
        let started = Instant::now();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let child = Arc::new(Mutex::new(child));
        self.cold.lock().unwrap().insert(run_id, child.clone());
        sink(RunEvent::Started { run_id, mode: RunMode::Cold, argv });

        let cold = self.cold.clone();
        let cancelled = self.cancelled.clone();
        std::thread::spawn(move || {
            // Output that arrives after Finished (from a lingering descendant) is dropped.
            let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let gated: Sink = {
                let (sink, closed) = (sink.clone(), closed.clone());
                Arc::new(move |e| {
                    if !closed.load(std::sync::atomic::Ordering::SeqCst) {
                        sink(e)
                    }
                })
            };
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            for (pipe, stream) in [(Box::new(stdout) as Box<dyn Read + Send>, Stream::Stdout), (Box::new(stderr), Stream::Stderr)]
            {
                let (sink, done) = (gated.clone(), done_tx.clone());
                std::thread::spawn(move || {
                    pump(pipe, run_id, stream, &sink);
                    let _ = done.send(());
                });
            }
            let status = loop {
                if let Some(status) = child.lock().unwrap().try_wait().ok().flatten() {
                    break Some(status);
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            // Drain what the command wrote, but never wait on a descendant that
            // inherited the pipes and is still running.
            let deadline = Instant::now() + PIPE_GRACE;
            for _ in 0..2 {
                if done_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).is_err() {
                    break;
                }
            }
            closed.store(true, std::sync::atomic::Ordering::SeqCst);
            cold.lock().unwrap().remove(&run_id);
            let was_cancelled = cancelled.lock().unwrap().remove(&run_id);
            let code = status.map_or(1, exit_code);
            sink(RunEvent::Finished {
                run_id,
                code,
                duration_ms: started.elapsed().as_millis() as u64,
                mode: RunMode::Cold,
                cancelled: was_cancelled,
            });
        });
        Ok(())
    }

    /// Stop a run. A warm run is stopped by killing the worker (it restarts on demand).
    pub fn cancel(&self, run_id: u64) -> bool {
        if let Some(child) = self.cold.lock().unwrap().get(&run_id) {
            self.cancelled.lock().unwrap().insert(run_id);
            kill_tree(&mut child.lock().unwrap());
            return true;
        }
        let mut slot = self.worker.lock().unwrap();
        if slot.as_ref().is_some_and(|w| w.is_running(run_id)) {
            slot.take().unwrap().kill();
            return true;
        }
        drop(slot);
        let retiring = self.retiring.lock().unwrap().iter().find(|w| w.is_running(run_id)).cloned();
        if let Some(w) = retiring {
            w.kill();
            return true;
        }
        false
    }
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return 128 + sig;
        }
    }
    1
}

/// Forward a pipe as it arrives (not line-buffered, so progress output shows
/// immediately), keeping multi-byte UTF-8 sequences intact across reads.
fn pump(mut pipe: impl Read, run_id: u64, stream: Stream, sink: &Sink) {
    let mut buf = [0u8; 8192];
    let mut carry: Vec<u8> = vec![];
    loop {
        let n = match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        carry.extend_from_slice(&buf[..n]);
        let valid = match std::str::from_utf8(&carry) {
            Ok(_) => carry.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => carry.len(),
        };
        if valid == 0 {
            continue;
        }
        let text = String::from_utf8_lossy(&carry[..valid]).into_owned();
        carry.drain(..valid);
        sink(RunEvent::Output { run_id, stream, data: crate::ansi::strip(&text) });
    }
    if !carry.is_empty() {
        sink(RunEvent::Output { run_id, stream, data: String::from_utf8_lossy(&carry).into_owned() });
    }
}
