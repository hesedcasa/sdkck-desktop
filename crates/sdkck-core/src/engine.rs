//! The app-facing facade: one `Engine` owns settings, runtime discovery, the
//! cached catalog, the runner and run history. The Tauri layer is a thin
//! adapter over these methods, so everything here is testable without a UI.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};

use crate::agent_vault::{self, Interception};
use crate::catalog::{self, Catalog, CatalogSource, CommandMeta, CommandSource};
use crate::env::Env;
use crate::paths::SdkckDirs;
use crate::permission::{self, Decision, PermissionConfig, RuleKind};
use crate::runner::{RunEvent, RunMode, Runner, RunnerConfig, Sink, WorkerState};
use crate::runtime::{self, NodeInfo, SdkckInfo};
use crate::search::Searcher;
use crate::{Error, Result};

/// Command prefixes that always run cold: they mutate the plugin tree the
/// worker has loaded, or are long-lived servers that would pin the worker.
const COLD_ONLY: &[&str] = &["plugins", "mcp:start", "webui", "update", "autocomplete"];
const HISTORY_OUTPUT_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Use this node binary instead of discovering one.
    pub node_path: Option<PathBuf>,
    /// Use this sdkck package directory instead of the managed/global one.
    pub sdkck_root: Option<PathBuf>,
    /// Run commands in the pre-booted worker when safe.
    pub warm_runner: bool,
    /// Import the login shell's environment (GUI launches lack it on macOS/Linux).
    pub login_shell_env: bool,
    /// Extra environment variables for every command (API keys, SDKCK_* dirs…).
    pub env: BTreeMap<String, String>,
    pub history_limit: usize,
    /// Set once the setup wizard has been completed or skipped.
    pub onboarded: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            node_path: None,
            sdkck_root: None,
            warm_runner: true,
            login_shell_env: !cfg!(windows),
            env: BTreeMap::new(),
            history_limit: 200,
            onboarded: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub run_id: u64,
    pub command_id: Option<String>,
    pub argv: Vec<String>,
    pub started_at: u64,
    pub mode: Option<RunMode>,
    pub code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub cancelled: bool,
    pub output: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub node: Option<NodeInfo>,
    pub sdkck: Option<SdkckInfo>,
    pub dirs: SdkckDirs,
    pub managed_prefix: PathBuf,
    pub interception: Interception,
    pub worker: WorkerState,
    pub ready: bool,
    pub problems: Vec<String>,
    pub settings: Settings,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub commands: Vec<CommandMeta>,
    pub total: usize,
    pub search_micros: u128,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionView {
    pub path: PathBuf,
    pub exists: bool,
    pub config: PermissionConfig,
    pub error: Option<String>,
}

struct Discovered {
    env: Env,
    dirs: SdkckDirs,
    node: Option<NodeInfo>,
    sdkck: Option<SdkckInfo>,
}

pub struct Engine {
    app_dir: PathBuf,
    settings_path: PathBuf,
    history_path: PathBuf,
    process_env: Env,
    shell_env: Mutex<Option<Env>>,
    settings: RwLock<Settings>,
    state: RwLock<Discovered>,
    catalog: Mutex<Option<Arc<Catalog>>>,
    searcher: Mutex<Searcher>,
    runner: Arc<Runner>,
    history: Arc<Mutex<VecDeque<HistoryEntry>>>,
    next_run_id: AtomicU64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Engine {
    /// `app_dir` holds the app's own state: settings, history, and the
    /// managed sdkck install under `runtime/`.
    pub fn new(app_dir: impl Into<PathBuf>, process_env: Env) -> Arc<Self> {
        let app_dir = app_dir.into();
        let settings_path = app_dir.join("settings.json");
        let history_path = app_dir.join("history.json");
        let settings: Settings = crate::manifest::read_json(&settings_path).unwrap_or_default();
        let history: VecDeque<HistoryEntry> = crate::manifest::read_json(&history_path).unwrap_or_default();
        let next = history.iter().map(|h| h.run_id).max().unwrap_or(0) + 1;
        let dirs = SdkckDirs::resolve(&process_env);
        let engine = Arc::new(Engine {
            app_dir,
            settings_path,
            history_path,
            process_env: process_env.clone(),
            shell_env: Mutex::new(None),
            settings: RwLock::new(settings),
            state: RwLock::new(Discovered { env: process_env, dirs, node: None, sdkck: None }),
            catalog: Mutex::new(None),
            searcher: Mutex::new(Searcher::default()),
            runner: Arc::new(Runner::new()),
            history: Arc::new(Mutex::new(history)),
            next_run_id: AtomicU64::new(next),
        });
        engine.rediscover();
        engine
    }

    pub fn managed_prefix(&self) -> PathBuf {
        self.app_dir.join("runtime")
    }

    fn effective_env(&self, settings: &Settings) -> Env {
        let base = if settings.login_shell_env {
            let mut cached = self.shell_env.lock().unwrap();
            cached.get_or_insert_with(|| runtime::merge_login_shell_env(&self.process_env)).clone()
        } else {
            self.process_env.clone()
        };
        if settings.env.is_empty() {
            return base;
        }
        let mut vars = base.vars().clone();
        vars.extend(settings.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        Env::new(vars, base.home(), base.platform())
    }

    /// Re-resolve environment, directories, node and sdkck from settings, and
    /// reconfigure the runner. Cheap enough to call after any settings change.
    pub fn rediscover(self: &Arc<Self>) {
        let settings = self.settings.read().unwrap().clone();
        let env = self.effective_env(&settings);
        let dirs = SdkckDirs::resolve(&env);
        let node = runtime::find_node(&env, settings.node_path.as_deref());
        let sdkck = runtime::find_sdkck(&env, settings.sdkck_root.as_deref(), &self.managed_prefix());
        let runner_config = match (&node, &sdkck) {
            (Some(n), Some(s)) if n.supported => {
                let mut vars = env.vars().clone();
                // Windows spells it `Path`, and env keys there are case-insensitive:
                // drop every spelling so the node-first PATH is the only one.
                vars.retain(|k, _| !k.eq_ignore_ascii_case("path"));
                vars.insert("PATH".into(), runtime::path_with_node(&env, &n.path).to_string_lossy().into_owned());
                Some(RunnerConfig { node: n.path.clone(), sdkck: s.clone(), env: vars, scratch_dir: self.app_dir.join("worker") })
            }
            _ => None,
        };
        *self.state.write().unwrap() = Discovered { env, dirs, node, sdkck };
        *self.catalog.lock().unwrap() = None;
        self.runner.configure(runner_config);
        if settings.warm_runner && self.runner.is_configured() && !self.interception().forces_cold_runs() {
            self.runner.prewarm();
        }
    }

    pub fn interception(&self) -> Interception {
        let state = self.state.read().unwrap();
        agent_vault::detect(&state.env, &state.dirs.config)
    }

    pub fn status(&self) -> Status {
        let state = self.state.read().unwrap();
        let settings = self.settings.read().unwrap().clone();
        let mut problems = vec![];
        match &state.node {
            None => problems.push(format!(
                "Node.js {}+ was not found. Install it from https://nodejs.org or set its path in Settings.",
                runtime::MIN_NODE_MAJOR
            )),
            Some(n) if !n.supported => {
                problems.push(format!("Node.js {} is too old; sdkck needs {}+.", n.version, runtime::MIN_NODE_MAJOR))
            }
            _ => {}
        }
        if state.sdkck.is_none() {
            problems.push("sdkck is not installed yet.".into());
        }
        let interception = agent_vault::detect(&state.env, &state.dirs.config);
        Status {
            ready: problems.is_empty(),
            node: state.node.clone(),
            sdkck: state.sdkck.clone(),
            dirs: state.dirs.clone(),
            managed_prefix: self.managed_prefix(),
            interception,
            worker: self.runner.worker_state(),
            problems,
            settings,
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    pub fn save_settings(self: &Arc<Self>, settings: Settings) -> Result<Status> {
        let login_changed = self.settings.read().unwrap().login_shell_env != settings.login_shell_env;
        std::fs::create_dir_all(&self.app_dir).map_err(|e| Error::io(&self.app_dir, e))?;
        std::fs::write(&self.settings_path, serde_json::to_string_pretty(&settings)?)
            .map_err(|e| Error::io(&self.settings_path, e))?;
        if login_changed {
            *self.shell_env.lock().unwrap() = None;
        }
        *self.settings.write().unwrap() = settings;
        self.rediscover();
        Ok(self.status())
    }

    /// The catalog, rebuilt only when a file it was built from changed.
    pub fn catalog(&self) -> Result<Arc<Catalog>> {
        let mut slot = self.catalog.lock().unwrap();
        if let Some(c) = slot.as_ref().filter(|c| c.fingerprint.is_current()) {
            return Ok(c.clone());
        }
        let state = self.state.read().unwrap();
        let sdkck = state.sdkck.as_ref().ok_or_else(|| Error::NotFound("sdkck is not installed yet".into()))?;
        let built = Arc::new(catalog::build(&CatalogSource { root: &sdkck.root, dirs: &state.dirs })?);
        *slot = Some(built.clone());
        Ok(built)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<SearchResult> {
        let catalog = self.catalog()?;
        let started = std::time::Instant::now();
        let hits = self.searcher.lock().unwrap().search(&catalog.commands, query, limit);
        Ok(SearchResult {
            total: hits.len(),
            commands: hits.into_iter().map(|i| catalog.commands[i].clone()).collect(),
            search_micros: started.elapsed().as_micros(),
        })
    }

    pub fn command(&self, id: &str) -> Result<CommandMeta> {
        let catalog = self.catalog()?;
        let id = id.replace(' ', ":");
        catalog
            .commands
            .iter()
            .chain(&catalog.blocked)
            .find(|c| c.id == id || c.aliases.contains(&id))
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("command {id} not found")))
    }

    fn allow_warm(&self, argv: &[String], command: Option<&CommandMeta>) -> bool {
        if !self.settings.read().unwrap().warm_runner || self.interception().forces_cold_runs() {
            return false;
        }
        // A JIT command's first run installs its plugin, which the worker's
        // already-loaded config would never see.
        if command.is_some_and(|c| c.source == CommandSource::Jit) {
            return false;
        }
        let id = argv.iter().take_while(|a| !a.starts_with('-')).cloned().collect::<Vec<_>>().join(":");
        !COLD_ONLY.iter().any(|p| id == *p || id.starts_with(&format!("{p}:")))
    }

    /// Run a catalog command with already-built arguments/flags.
    pub fn run_command(self: &Arc<Self>, id: &str, args: Vec<String>, sink: Sink) -> Result<u64> {
        let command = self.command(id).ok();
        let mut argv: Vec<String> = id.replace(' ', ":").split(':').map(str::to_string).collect();
        argv.extend(args);
        self.run_argv(argv, command, sink)
    }

    /// Run a raw sdkck argv, as typed after `sdkck`.
    pub fn run_raw(self: &Arc<Self>, argv: Vec<String>, sink: Sink) -> Result<u64> {
        self.run_argv(argv, None, sink)
    }

    fn run_argv(self: &Arc<Self>, argv: Vec<String>, command: Option<CommandMeta>, sink: Sink) -> Result<u64> {
        let run_id = self.next_run_id.fetch_add(1, Ordering::SeqCst);
        let warm = self.allow_warm(&argv, command.as_ref());
        self.history_start(run_id, command.as_ref().map(|c| c.id.clone()), argv.clone());
        let engine = Arc::downgrade(self);
        let history = self.history.clone();
        let wrapped: Sink = Arc::new(move |event: RunEvent| {
            record(&history, &event);
            let finished = matches!(event, RunEvent::Finished { .. });
            sink(event);
            if finished {
                if let Some(engine) = engine.upgrade() {
                    engine.after_run();
                }
            }
        });
        self.runner.run(run_id, argv, warm, wrapped)?;
        Ok(run_id)
    }

    /// After a run: persist history, and if the run changed what the CLI
    /// loads (a plugin install, `api import`, `mcp client add`, …) recycle the
    /// worker so its next run sees the new commands.
    fn after_run(self: &Arc<Self>) {
        self.persist_history();
        let stale = self.catalog.lock().unwrap().as_ref().is_some_and(|c| !c.fingerprint.is_current());
        if stale {
            self.runner.recycle();
            if self.settings.read().unwrap().warm_runner && !self.interception().forces_cold_runs() {
                self.runner.prewarm();
            }
        }
    }

    pub fn cancel(&self, run_id: u64) -> bool {
        self.runner.cancel(run_id)
    }

    /// Restart the warm worker (e.g. after editing a plugin by hand).
    pub fn restart_worker(self: &Arc<Self>) {
        self.runner.recycle();
        if self.settings.read().unwrap().warm_runner && !self.interception().forces_cold_runs() {
            self.runner.prewarm();
        }
    }

    // ── Plugins ────────────────────────────────────────────────────────────

    pub fn install_plugin(self: &Arc<Self>, spec: &str, sink: Sink) -> Result<u64> {
        validate_spec(spec)?;
        self.run_raw(vec!["plugins".into(), "install".into(), spec.to_string()], sink)
    }

    pub fn uninstall_plugin(self: &Arc<Self>, name: &str, sink: Sink) -> Result<u64> {
        validate_spec(name)?;
        self.run_raw(vec!["plugins".into(), "uninstall".into(), name.to_string()], sink)
    }

    pub fn update_plugins(self: &Arc<Self>, sink: Sink) -> Result<u64> {
        self.run_raw(vec!["plugins".into(), "update".into()], sink)
    }

    pub fn search_registry(&self, text: &str) -> Result<Vec<crate::registry::RegistryPackage>> {
        let registry = self.state.read().unwrap().env.get("SDKCK_NPM_REGISTRY").map(str::to_string);
        crate::registry::search(registry.as_deref(), text, 40)
    }

    /// Install or update sdkck itself into the managed prefix.
    pub fn install_sdkck(self: &Arc<Self>, version: &str, on_line: impl FnMut(&str)) -> Result<Status> {
        let (env, node) = {
            let state = self.state.read().unwrap();
            let node = state.node.clone().filter(|n| n.supported).ok_or_else(|| {
                Error::Runtime(format!("Install Node.js {}+ first (https://nodejs.org).", runtime::MIN_NODE_MAJOR))
            })?;
            (state.env.clone(), node)
        };
        runtime::install_sdkck(&env, &node.path, &self.managed_prefix(), version, on_line)?;
        self.rediscover();
        Ok(self.status())
    }

    // ── Permissions ────────────────────────────────────────────────────────

    fn config_dir(&self) -> PathBuf {
        self.state.read().unwrap().dirs.config.clone()
    }

    pub fn permissions(&self) -> PermissionView {
        let dir = self.config_dir();
        let path = permission::config_path(&dir);
        match permission::read(&dir) {
            Ok(Some(config)) => PermissionView { path, exists: true, config, error: None },
            Ok(None) => PermissionView { path, exists: false, config: PermissionConfig::allow_all(), error: None },
            Err(e) => PermissionView {
                path,
                exists: true,
                config: PermissionConfig { allow_rules: vec![], deny_rules: vec![] },
                error: Some(e.to_string()),
            },
        }
    }

    pub fn add_permission_rule(&self, kind: RuleKind, pattern: &str) -> Result<PermissionView> {
        let dir = self.config_dir();
        // Creating the file from scratch would flip "no file = allow all" to
        // default-deny; seed the allow-all rule so adding a deny rule only denies.
        if permission::read(&dir)?.is_none() {
            permission::write(&dir, &PermissionConfig::allow_all())?;
        }
        permission::add_rule(&dir, kind, pattern)?;
        Ok(self.permissions())
    }

    pub fn remove_permission_rule(&self, kind: RuleKind, pattern: &str) -> Result<PermissionView> {
        permission::remove_rule(&self.config_dir(), kind, pattern)?;
        Ok(self.permissions())
    }

    pub fn reset_permissions(&self) -> Result<PermissionView> {
        permission::write(&self.config_dir(), &PermissionConfig::allow_all())?;
        Ok(self.permissions())
    }

    pub fn check_permission(&self, id: &str) -> Result<Decision> {
        Ok(permission::explain(id, &permission::effective(&self.config_dir())?))
    }

    // ── History ────────────────────────────────────────────────────────────

    fn history_start(&self, run_id: u64, command_id: Option<String>, argv: Vec<String>) {
        let limit = self.settings.read().unwrap().history_limit.max(1);
        let mut h = self.history.lock().unwrap();
        h.push_front(HistoryEntry {
            run_id,
            command_id,
            argv,
            started_at: now_ms(),
            mode: None,
            code: None,
            duration_ms: None,
            cancelled: false,
            output: String::new(),
            truncated: false,
        });
        h.truncate(limit);
    }

    pub fn history(&self) -> Vec<HistoryEntry> {
        self.history.lock().unwrap().iter().cloned().collect()
    }

    pub fn clear_history(&self) {
        self.history.lock().unwrap().clear();
        self.persist_history();
    }

    fn persist_history(&self) {
        let snapshot: Vec<HistoryEntry> = self.history.lock().unwrap().iter().filter(|h| h.code.is_some()).cloned().collect();
        if let Ok(body) = serde_json::to_string(&snapshot) {
            let _ = std::fs::create_dir_all(&self.app_dir);
            let _ = std::fs::write(&self.history_path, body);
        }
    }

    pub fn app_dir(&self) -> &Path {
        &self.app_dir
    }
}

fn record(history: &Mutex<VecDeque<HistoryEntry>>, event: &RunEvent) {
    let mut h = history.lock().unwrap();
    match event {
        RunEvent::Started { run_id, mode, .. } => {
            if let Some(e) = h.iter_mut().find(|e| e.run_id == *run_id) {
                e.mode = Some(*mode);
            }
        }
        RunEvent::Output { run_id, data, .. } => {
            if let Some(e) = h.iter_mut().find(|e| e.run_id == *run_id) {
                if e.output.len() + data.len() <= HISTORY_OUTPUT_LIMIT {
                    e.output.push_str(data);
                } else {
                    e.truncated = true;
                }
            }
        }
        RunEvent::Finished { run_id, code, duration_ms, mode, cancelled } => {
            if let Some(e) = h.iter_mut().find(|e| e.run_id == *run_id) {
                e.code = Some(*code);
                e.duration_ms = Some(*duration_ms);
                e.mode = Some(*mode);
                e.cancelled = *cancelled;
            }
        }
    }
}

/// An npm package spec as `plugins install` takes it — never a flag.
fn validate_spec(spec: &str) -> Result<()> {
    let ok = !spec.is_empty() && !spec.starts_with('-') && !spec.chars().any(char::is_whitespace);
    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(format!("{spec:?} is not a valid plugin name")))
    }
}
