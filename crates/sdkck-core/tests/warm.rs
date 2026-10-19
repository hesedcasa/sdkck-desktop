//! Warm-runner tests against a real sdkck install. They need the network-free
//! but real package (oclif, plugins), so they run only when `SDKCK_TEST_ROOT`
//! points at one, e.g. after `npm install --prefix /tmp/rt sdkck`:
//!
//!   SDKCK_TEST_ROOT=/tmp/rt/node_modules/sdkck cargo test -p sdkck-core --test warm

use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use sdkck_core::engine::{Engine, Settings};
use sdkck_core::env::{Env, Platform};
use sdkck_core::runner::{RunEvent, RunMode, WorkerState};

fn engine() -> Option<(Arc<Engine>, tempfile::TempDir)> {
    let root = std::env::var("SDKCK_TEST_ROOT").ok()?;
    let tmp = tempfile::tempdir().unwrap();
    let env = Env::new(
        std::env::vars().chain([
            ("SDKCK_CONFIG_DIR".into(), tmp.path().join("config").display().to_string()),
            ("SDKCK_DATA_DIR".into(), tmp.path().join("data").display().to_string()),
            ("SDKCK_CACHE_DIR".into(), tmp.path().join("cache").display().to_string()),
            ("SDKCK_OTEL_DISABLED".into(), "true".into()),
        ]),
        tmp.path(),
        Platform::current(),
    );
    let engine = Engine::new(tmp.path().join("app"), env);
    engine.save_settings(Settings { sdkck_root: Some(root.into()), login_shell_env: false, ..Settings::default() }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while engine.status().worker != WorkerState::Ready {
        assert!(Instant::now() < deadline, "worker never became ready: {:?}", engine.status().worker);
        std::thread::sleep(Duration::from_millis(100));
    }
    Some((engine, tmp))
}

fn run(engine: &Arc<Engine>, argv: &[&str]) -> (String, i32, RunMode) {
    let (tx, rx) = mpsc::channel();
    engine.run_raw(argv.iter().map(|s| s.to_string()).collect(), Arc::new(move |e| drop(tx.send(e)))).unwrap();
    let mut out = String::new();
    for e in rx.iter() {
        match e {
            RunEvent::Output { data, .. } => out.push_str(&data),
            RunEvent::Finished { code, mode, .. } => return (out, code, mode),
            RunEvent::Started { .. } => {}
        }
    }
    unreachable!()
}

#[test]
fn warm_runs_match_cold_runs_and_enforce_permissions() {
    let Some((engine, _tmp)) = engine() else {
        eprintln!("skipping: SDKCK_TEST_ROOT not set");
        return;
    };
    let (warm_out, code, mode) = run(&engine, &["version"]);
    assert_eq!((code, mode), (0, RunMode::Warm));
    assert!(warm_out.starts_with("sdkck/"), "{warm_out}");

    // Non-zero exits and their messages survive the in-process path.
    let (out, code, _) = run(&engine, &["permission", "check"]);
    assert_ne!(code, 0, "{out}");

    // Rules written natively are enforced by the plugin's prerun hook in the worker.
    engine.add_permission_rule(sdkck_core::permission::RuleKind::Deny, "version").unwrap();
    let (out, code, mode) = run(&engine, &["version"]);
    assert_eq!((code, mode), (2, RunMode::Warm), "{out}");
    assert!(out.contains("not permitted"), "{out}");

    // Plugin management always runs cold.
    let (_, _, mode) = run(&engine, &["plugins"]);
    assert_eq!(mode, RunMode::Cold);
}
