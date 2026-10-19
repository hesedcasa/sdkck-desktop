//! Compare sdkck Desktop's native paths with the CLI they replace.
//!
//!   cargo run --release -p sdkck-core --example bench -- [argv for a run…]
//!
//! Uses the same discovery as the app (settings override via SDKCK_ROOT).

use std::sync::{mpsc, Arc};
use std::time::Instant;

use sdkck_core::engine::{Engine, Settings};
use sdkck_core::env::Env;
use sdkck_core::runner::RunEvent;

fn main() {
    let app_dir = std::env::temp_dir().join("sdkck-desktop-bench");
    let engine = Engine::new(&app_dir, Env::from_process());
    if let Ok(root) = std::env::var("SDKCK_ROOT") {
        engine.save_settings(Settings { sdkck_root: Some(root.into()), login_shell_env: false, ..engine.settings() }).unwrap();
    }
    let status = engine.status();
    assert!(status.ready, "not ready: {:?}", status.problems);
    println!(
        "node {} · sdkck {:?} at {}",
        status.node.unwrap().version,
        status.sdkck.as_ref().unwrap().version,
        status.sdkck.as_ref().unwrap().root.display()
    );

    let t = Instant::now();
    let catalog = engine.catalog().unwrap();
    println!(
        "catalog: {} commands, {} plugins, {} blocked — built in {:.2} ms (wall {:.2} ms)",
        catalog.commands.len(),
        catalog.plugins.len(),
        catalog.blocked.len(),
        catalog.build_micros as f64 / 1000.0,
        t.elapsed().as_secs_f64() * 1000.0
    );
    for w in &catalog.warnings {
        println!("  warning: {w}");
    }
    let t = Instant::now();
    for _ in 0..100 {
        engine.catalog().unwrap();
    }
    println!("catalog revalidation: {:.3} ms avg", t.elapsed().as_secs_f64() * 10.0);
    let r = engine.search("jira issue", 5).unwrap();
    println!(
        "search 'jira issue' in {} µs → {:?}",
        r.search_micros,
        r.commands.iter().map(|c| c.display_id.as_str()).collect::<Vec<_>>()
    );

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let argv = if argv.is_empty() { vec!["version".to_string()] } else { argv };
    // Let the worker boot (the app does this at startup).
    std::thread::sleep(std::time::Duration::from_secs(4));
    for i in 0..4 {
        let (tx, rx) = mpsc::channel();
        let t = Instant::now();
        let sink = Arc::new(move |e: RunEvent| {
            let _ = tx.send(e);
        });
        engine.run_raw(argv.clone(), sink).unwrap();
        let mut out = String::new();
        for e in rx {
            match e {
                RunEvent::Output { data, .. } => out.push_str(&data),
                RunEvent::Finished { code, mode, .. } => {
                    println!(
                        "run #{i} {:?} {mode:?} exit {code} in {:.0} ms: {}",
                        argv,
                        t.elapsed().as_secs_f64() * 1000.0,
                        out.trim().lines().next().unwrap_or("")
                    );
                    break;
                }
                _ => {}
            }
        }
    }
}
