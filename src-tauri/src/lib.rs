//! Tauri adapter over `sdkck_core::engine::Engine`. Every command runs on a
//! blocking thread (discovery and installs shell out), and run output is
//! pushed to the webview as `run-event` events.

use std::sync::{Arc, Condvar, Mutex};

use sdkck_core::catalog::{CommandMeta, PluginInfo, Topic};
use sdkck_core::engine::{Engine, HistoryEntry, PermissionView, SearchResult, Settings, Status};
use sdkck_core::env::Env;
use sdkck_core::permission::{Decision, RuleKind};
use sdkck_core::registry::RegistryPackage;
use sdkck_core::runner::{RunEvent, Sink};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// The engine is created off the main thread (login-shell env import and
/// runtime discovery can take a moment); commands wait for it.
#[derive(Default, Clone)]
struct EngineCell(Arc<(Mutex<Option<Arc<Engine>>>, Condvar)>);

impl EngineCell {
    fn set(&self, engine: Arc<Engine>) {
        *self.0 .0.lock().unwrap() = Some(engine);
        self.0 .1.notify_all();
    }

    fn get(&self) -> Arc<Engine> {
        let guard = self.0 .0.lock().unwrap();
        let guard = self.0 .1.wait_while(guard, |e| e.is_none()).unwrap();
        guard.clone().unwrap()
    }
}

type CmdResult<T> = Result<T, String>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

fn sink(app: &AppHandle) -> Sink {
    let app = app.clone();
    Arc::new(move |event: RunEvent| {
        let _ = app.emit("run-event", event);
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogSummary {
    bin: String,
    version: Option<String>,
    command_count: usize,
    blocked_count: usize,
    plugins: Vec<PluginInfo>,
    topics: Vec<Topic>,
    warnings: Vec<String>,
    build_micros: u128,
}

#[tauri::command]
async fn status(cell: State<'_, EngineCell>) -> CmdResult<Status> {
    let cell = cell.inner().clone();
    blocking(move || Ok(cell.get().status())).await
}

#[tauri::command]
async fn save_settings(cell: State<'_, EngineCell>, settings: Settings) -> CmdResult<Status> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().save_settings(settings).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn rediscover(cell: State<'_, EngineCell>) -> CmdResult<Status> {
    let cell = cell.inner().clone();
    blocking(move || {
        let engine = cell.get();
        engine.rediscover();
        Ok(engine.status())
    })
    .await
}

#[tauri::command]
async fn install_sdkck(app: AppHandle, cell: State<'_, EngineCell>, version: Option<String>) -> CmdResult<Status> {
    let cell = cell.inner().clone();
    blocking(move || {
        let version = version.unwrap_or_else(|| "latest".into());
        cell.get()
            .install_sdkck(&version, |line| {
                let _ = app.emit("setup-log", line);
            })
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn catalog(cell: State<'_, EngineCell>) -> CmdResult<CatalogSummary> {
    let cell = cell.inner().clone();
    blocking(move || {
        let c = cell.get().catalog().map_err(|e| e.to_string())?;
        Ok(CatalogSummary {
            bin: c.bin.clone(),
            version: c.version.clone(),
            command_count: c.commands.len(),
            blocked_count: c.blocked.len(),
            plugins: c.plugins.clone(),
            topics: c.topics.clone(),
            warnings: c.warnings.clone(),
            build_micros: c.build_micros,
        })
    })
    .await
}

#[tauri::command]
async fn search(cell: State<'_, EngineCell>, query: String, limit: Option<usize>) -> CmdResult<SearchResult> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().search(&query, limit.unwrap_or(500)).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn command(cell: State<'_, EngineCell>, id: String) -> CmdResult<CommandMeta> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().command(&id).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn blocked_commands(cell: State<'_, EngineCell>) -> CmdResult<Vec<CommandMeta>> {
    let cell = cell.inner().clone();
    blocking(move || Ok(cell.get().catalog().map_err(|e| e.to_string())?.blocked.clone())).await
}

#[tauri::command]
async fn run_command(app: AppHandle, cell: State<'_, EngineCell>, id: String, args: Vec<String>) -> CmdResult<u64> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().run_command(&id, args, sink(&app)).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn run_raw(app: AppHandle, cell: State<'_, EngineCell>, argv: Vec<String>) -> CmdResult<u64> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().run_raw(argv, sink(&app)).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn cancel_run(cell: State<'_, EngineCell>, run_id: u64) -> CmdResult<bool> {
    let cell = cell.inner().clone();
    blocking(move || Ok(cell.get().cancel(run_id))).await
}

#[tauri::command]
async fn restart_worker(cell: State<'_, EngineCell>) -> CmdResult<()> {
    let cell = cell.inner().clone();
    blocking(move || {
        cell.get().restart_worker();
        Ok(())
    })
    .await
}

#[tauri::command]
async fn history(cell: State<'_, EngineCell>) -> CmdResult<Vec<HistoryEntry>> {
    let cell = cell.inner().clone();
    blocking(move || Ok(cell.get().history())).await
}

#[tauri::command]
async fn clear_history(cell: State<'_, EngineCell>) -> CmdResult<()> {
    let cell = cell.inner().clone();
    blocking(move || {
        cell.get().clear_history();
        Ok(())
    })
    .await
}

#[tauri::command]
async fn install_plugin(app: AppHandle, cell: State<'_, EngineCell>, spec: String) -> CmdResult<u64> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().install_plugin(&spec, sink(&app)).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn uninstall_plugin(app: AppHandle, cell: State<'_, EngineCell>, name: String) -> CmdResult<u64> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().uninstall_plugin(&name, sink(&app)).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn update_plugins(app: AppHandle, cell: State<'_, EngineCell>) -> CmdResult<u64> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().update_plugins(sink(&app)).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn search_registry(cell: State<'_, EngineCell>, text: String) -> CmdResult<Vec<RegistryPackage>> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().search_registry(&text).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn permissions(cell: State<'_, EngineCell>) -> CmdResult<PermissionView> {
    let cell = cell.inner().clone();
    blocking(move || Ok(cell.get().permissions())).await
}

#[tauri::command]
async fn add_permission_rule(cell: State<'_, EngineCell>, kind: RuleKind, pattern: String) -> CmdResult<PermissionView> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().add_permission_rule(kind, &pattern).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn remove_permission_rule(cell: State<'_, EngineCell>, kind: RuleKind, pattern: String) -> CmdResult<PermissionView> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().remove_permission_rule(kind, &pattern).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn reset_permissions(cell: State<'_, EngineCell>) -> CmdResult<PermissionView> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().reset_permissions().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn check_permission(cell: State<'_, EngineCell>, id: String) -> CmdResult<Decision> {
    let cell = cell.inner().clone();
    blocking(move || cell.get().check_permission(&id).map_err(|e| e.to_string())).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let cell = EngineCell::default();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(cell.clone())
        .setup(move |app| {
            let app_dir = app.path().app_data_dir()?;
            std::thread::spawn(move || cell.set(Engine::new(app_dir, Env::from_process())));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status,
            save_settings,
            rediscover,
            install_sdkck,
            catalog,
            search,
            command,
            blocked_commands,
            run_command,
            run_raw,
            cancel_run,
            restart_worker,
            history,
            clear_history,
            install_plugin,
            uninstall_plugin,
            update_plugins,
            search_registry,
            permissions,
            add_permission_rule,
            remove_permission_rule,
            reset_permissions,
            check_permission,
        ])
        .run(tauri::generate_context!())
        .expect("error while running sdkck Desktop");
}
