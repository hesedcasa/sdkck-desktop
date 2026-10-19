//! Catalog and runner tests against a self-contained fake sdkck tree, so they
//! need neither the network nor a real sdkck install (cold-run tests need
//! `node` on PATH and are skipped without it).

use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use sdkck_core::catalog::{build, CatalogSource, CommandSource, FlagKind};
use sdkck_core::engine::{Engine, Settings};
use sdkck_core::env::{Env, Platform};
use sdkck_core::paths::SdkckDirs;
use sdkck_core::runner::{RunEvent, RunMode};

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
    dirs: SdkckDirs,
}

/// root package (one own command + JIT placeholders for @hesed/jira and
/// @hesed/bb), a bundled plugin found by glob, @hesed/jira installed as a user
/// plugin, one imported API spec, one MCP server, and a deny rule.
fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let root = base.join("prefix/node_modules/sdkck");
    write(
        &root.join("package.json"),
        r#"{"name":"sdkck","version":"9.9.9","oclif":{"bin":"sdkck","plugins":["@oclif/plugin-*"],
            "jitPlugins":{"@hesed/jira":"latest","@hesed/bb":"^2"},"topicSeparator":" "}}"#,
    );
    write(
        &root.join("oclif.manifest.json"),
        r#"{"version":"9.9.9","commands":{
          "hello":{"id":"hello","summary":"Say hello to <%= config.bin %>","pluginName":"sdkck","pluginType":"core",
            "args":{"zeta":{"name":"zeta","required":true},"alpha":{"name":"alpha"}},
            "flags":{"loud":{"name":"loud","type":"boolean","char":"l"},"secret":{"name":"secret","type":"option","hidden":true},
                     "tag":{"name":"tag","type":"option","multiple":true,"options":["a","b"],"default":"a"}},
            "examples":["<%= config.bin %> <%= command.id %> world"]},
          "jira:issue:get":{"id":"jira:issue:get","summary":"placeholder","pluginName":"@hesed/jira","pluginType":"jit"},
          "bb:repo:list":{"id":"bb:repo:list","summary":"List repos","pluginName":"@hesed/bb","pluginType":"jit"}
        }}"#,
    );
    // Bundled plugin, hoisted next to sdkck as npm does for global installs.
    let help = base.join("prefix/node_modules/@oclif/plugin-help");
    write(&help.join("package.json"), r#"{"name":"@oclif/plugin-help","version":"7.0.0"}"#);
    write(
        &help.join("oclif.manifest.json"),
        r#"{"commands":{"help":{"id":"help","summary":"Display help","pluginType":"core","pluginName":"@oclif/plugin-help"}}}"#,
    );

    let data = base.join("data");
    write(&data.join("package.json"), r#"{"oclif":{"plugins":[{"name":"@hesed/jira","type":"user","tag":"latest"}]}}"#);
    let jira = data.join("node_modules/@hesed/jira");
    write(
        &jira.join("package.json"),
        r#"{"name":"@hesed/jira","version":"1.4.0","oclif":{"topics":{"jira":{"description":"Jira Cloud"}}}}"#,
    );
    write(
        &jira.join("oclif.manifest.json"),
        r#"{"commands":{"jira:issue:get":{"id":"jira:issue:get","summary":"Get an issue","pluginName":"@hesed/jira","pluginType":"user",
            "args":{"key":{"name":"key","required":true}}},
          "jira:issue:delete":{"id":"jira:issue:delete","summary":"Delete","pluginName":"@hesed/jira","pluginType":"user"}}}"#,
    );
    write(&data.join("node_modules/@hesed/mcp-client/dist/index.js"), "");

    let config = base.join("config");
    write(
        &config.join("api-pets.json"),
        r#"{"name":"pets","title":"Pet Store","operations":[
          {"operationId":"getPet","method":"get","path":"/pets/{petId}","parameters":[
             {"name":"petId","in":"path","required":true},{"name":"output","in":"query","required":false}],"bodyParams":{}},
          {"operationId":"createPet","method":"post","path":"/pets","parameters":[{"name":"name","in":"query","required":false}],
           "bodyParams":{"name":{"required":true,"type":"string"},"age":{"required":false,"type":"integer"}}}]}"#,
    );
    write(
        &config.join("openapi-pets.json"),
        r#"{"name":"pets","operations":[{"operationId":"legacyOnly","method":"get","path":"/x"}]}"#,
    );
    write(
        &config.join("mcp-client-fixture.json"),
        r#"{"config":{"name":"fixture","transport":"stdio"},"cachedTools":[{"name":"echo","description":"Echo",
          "inputSchema":{"type":"object","required":["message"],"properties":{"message":{"type":"string"},"shout":{"type":"boolean"}}}}]}"#,
    );
    write(&config.join("mcp-client-fixture-oauth.json"), "not json at all");
    write(
        &config.join("permission.json"),
        r#"{"version":1,"allowRules":[{"pattern":"*"}],"denyRules":[{"pattern":"jira issue delete"}]}"#,
    );

    let dirs = SdkckDirs { config, data, cache: base.join("cache") };
    Fixture { _tmp: tmp, root, dirs }
}

#[test]
fn builds_the_same_command_set_oclif_would() {
    let f = fixture();
    let catalog = build(&CatalogSource { root: &f.root, dirs: &f.dirs }).unwrap();
    let ids: Vec<&str> = catalog.commands.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["bb:repo:list", "fixture:echo", "hello", "help", "jira:issue:get", "pets:createPet", "pets:getPet"]);
    assert!(catalog.warnings.is_empty(), "{:?}", catalog.warnings);

    // The installed plugin replaces its JIT placeholder; an uninstalled one stays JIT.
    let get = catalog.commands.iter().find(|c| c.id == "jira:issue:get").unwrap();
    assert_eq!(
        (get.source, get.summary.as_deref(), get.args[0].name.as_str()),
        (CommandSource::Plugin, Some("Get an issue"), "key")
    );
    let bb = catalog.commands.iter().find(|c| c.id == "bb:repo:list").unwrap();
    assert_eq!(bb.source, CommandSource::Jit);
    let bb_plugin = catalog.plugins.iter().find(|p| p.name == "@hesed/bb").unwrap();
    assert!(!bb_plugin.installed);
    assert_eq!((bb_plugin.command_count, bb_plugin.tag.as_deref()), (1, Some("^2")));

    // The deny rule hides the command, as the permission init hook does.
    assert_eq!(catalog.blocked.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["jira:issue:delete"]);
    assert_eq!(catalog.topics.iter().find(|t| t.id == "jira").unwrap().description.as_deref(), Some("Jira Cloud"));
}

#[test]
fn keeps_declaration_order_and_renders_templates() {
    let f = fixture();
    let catalog = build(&CatalogSource { root: &f.root, dirs: &f.dirs }).unwrap();
    let hello = catalog.commands.iter().find(|c| c.id == "hello").unwrap();
    assert_eq!(hello.args.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["zeta", "alpha"]);
    assert_eq!(hello.flags.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["loud", "tag"]);
    assert_eq!(hello.summary.as_deref(), Some("Say hello to sdkck"));
    assert_eq!(hello.examples, ["sdkck hello world"]);
    let tag = &hello.flags[1];
    assert!(tag.multiple && tag.kind == FlagKind::Option && tag.default == Some("a".into()));
}

#[test]
fn mirrors_the_dynamic_command_factories() {
    let f = fixture();
    let catalog = build(&CatalogSource { root: &f.root, dirs: &f.dirs }).unwrap();
    let names = |id: &str| {
        let c = catalog.commands.iter().find(|c| c.id == id).unwrap();
        (c.args.iter().map(|a| a.name.clone()).collect::<Vec<_>>(), c.flags.iter().map(|f| f.name.clone()).collect::<Vec<_>>())
    };
    // An optional `output` param shadows the file-output flag.
    assert_eq!(
        names("pets:getPet"),
        (vec!["petId".into()], vec!["header".into(), "profile".into(), "toon".into(), "output".into()])
    );
    let get = catalog.commands.iter().find(|c| c.id == "pets:getPet").unwrap();
    assert_eq!(get.flags[3].description.as_deref(), Some("[query] output"));
    // Body params colliding with URL params get a body- prefix.
    assert_eq!(
        names("pets:createPet"),
        (
            vec!["body-name".into()],
            vec!["header".into(), "profile".into(), "toon".into(), "output".into(), "name".into(), "age".into()]
        )
    );
    assert_eq!(names("fixture:echo").1, ["message", "shout", "json-args", "toon"]);
    let echo = catalog.commands.iter().find(|c| c.id == "fixture:echo").unwrap();
    assert_eq!(echo.flags[0].description.as_deref(), Some("(required) message"));
    assert_eq!(echo.source, CommandSource::Mcp);
}

#[test]
fn revalidates_on_change() {
    let f = fixture();
    let catalog = build(&CatalogSource { root: &f.root, dirs: &f.dirs }).unwrap();
    assert!(catalog.fingerprint.is_current());
    std::thread::sleep(Duration::from_millis(20));
    write(&f.dirs.config.join("api-more.json"), r#"{"name":"more","operations":[]}"#);
    assert!(!catalog.fingerprint.is_current(), "a new spec file must invalidate the catalog");

    // A malformed store file is tracked too, so repairing it in place is noticed.
    write(&f.dirs.config.join("api-more.json"), "{not json");
    let broken = build(&CatalogSource { root: &f.root, dirs: &f.dirs }).unwrap();
    assert!(broken.warnings.iter().any(|w| w.contains("api-more.json")), "{:?}", broken.warnings);
    std::thread::sleep(Duration::from_millis(20));
    write(&f.dirs.config.join("api-more.json"), r#"{"name":"more","operations":[]}"#);
    assert!(!broken.fingerprint.is_current(), "repairing a store file must invalidate the catalog");
}

fn node_available() -> bool {
    std::process::Command::new("node").arg("--version").output().is_ok_and(|o| o.status.success())
}

#[test]
fn cold_runs_stream_output_and_exit_codes() {
    if !node_available() {
        eprintln!("skipping: node not on PATH");
        return;
    }
    let f = fixture();
    write(
        &f.root.join("bin/run.js"),
        "process.stdout.write('args:' + process.argv.slice(2).join(',') + '\\n');\
         process.stderr.write('\\u001b[31mwarn\\u001b[0m\\n');\
         const mode = process.argv[2];         if (mode === 'tree' || mode === 'orphan') {           const c = require('child_process').spawn(process.execPath, ['-e', 'setTimeout(() => {}, 60000)'], {stdio: 'inherit'});           require('fs').writeFileSync(process.argv[3], String(c.pid)); c.unref();         }         if (mode === 'sleep' || mode === 'tree') setTimeout(() => {}, 60000);         else if (mode !== 'orphan') process.exit(Number(process.argv[3] ?? 0));",
    );
    let env = Env::new(
        std::env::vars().chain([
            ("SDKCK_CONFIG_DIR".to_string(), f.dirs.config.display().to_string()),
            ("SDKCK_DATA_DIR".to_string(), f.dirs.data.display().to_string()),
        ]),
        std::env::temp_dir(),
        Platform::current(),
    );
    let app = tempfile::tempdir().unwrap();
    let engine = Engine::new(app.path(), env);
    engine
        .save_settings(Settings {
            sdkck_root: Some(f.root.clone()),
            warm_runner: false,
            login_shell_env: false,
            ..Settings::default()
        })
        .unwrap();
    assert!(engine.status().ready, "{:?}", engine.status().problems);

    let run = |argv: Vec<&str>| {
        let (tx, rx) = mpsc::channel();
        let id = engine.run_raw(argv.into_iter().map(String::from).collect(), Arc::new(move |e| drop(tx.send(e)))).unwrap();
        (id, rx)
    };
    let collect = |rx: mpsc::Receiver<RunEvent>| {
        let mut out = String::new();
        for e in rx.iter() {
            match e {
                RunEvent::Output { data, .. } => out.push_str(&data),
                RunEvent::Finished { code, mode, cancelled, .. } => return (out, code, mode, cancelled),
                RunEvent::Started { .. } => {}
            }
        }
        unreachable!()
    };

    let (_, rx) = run(vec!["exit", "3"]);
    let (out, code, mode, _) = collect(rx);
    assert_eq!((code, mode), (3, RunMode::Cold));
    assert!(out.contains("args:exit,3") && out.contains("warn") && !out.contains('\u{1b}'), "{out:?}");

    let (id, rx) = run(vec!["sleep"]);
    std::thread::sleep(Duration::from_millis(300));
    assert!(engine.cancel(id));
    let (_, _, _, cancelled) = collect(rx);
    assert!(cancelled);

    let history = engine.history();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].code, Some(3));

    // Stop takes down everything the command spawned, not just node.
    std::fs::create_dir_all(&f.dirs.cache).unwrap();
    let pid_file = f.dirs.cache.join("tree.pid");
    let (id, rx) = run(vec!["tree", pid_file.to_str().unwrap()]);
    let pid = wait_for_pid(&pid_file);
    assert!(engine.cancel(id));
    let (_, _, _, cancelled) = collect(rx);
    assert!(cancelled);
    assert_process_gone(pid);

    // A descendant that outlives the command and holds its pipes must not keep
    // the run open.
    let pid_file = f.dirs.cache.join("orphan.pid");
    let started = std::time::Instant::now();
    let (_, rx) = run(vec!["orphan", pid_file.to_str().unwrap()]);
    let (_, code, _, _) = collect(rx);
    assert_eq!(code, 0);
    assert!(started.elapsed() < Duration::from_secs(10), "finished only after {:?}", started.elapsed());
    kill_pid(wait_for_pid(&pid_file));

    // Settings and history (API keys, command output) are owner-only.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&app.path().join("settings.json")), 0o600);
        assert_eq!(mode(&app.path().join("history.json")), 0o600);
        assert_eq!(mode(app.path()), 0o700);
    }
}

fn wait_for_pid(file: &Path) -> u32 {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(pid) = std::fs::read_to_string(file).ok().and_then(|s| s.trim().parse().ok()) {
            return pid;
        }
        assert!(std::time::Instant::now() < deadline, "child never started");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().is_ok_and(|s| s.success())
}

#[cfg(unix)]
fn assert_process_gone(pid: u32) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while process_alive(pid) {
        assert!(std::time::Instant::now() < deadline, "grandchild {pid} survived Stop");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(not(unix))]
fn assert_process_gone(_pid: u32) {}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
    #[cfg(windows)]
    let _ = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).status();
}
