//! Discovery and installation of the JavaScript side: a Node.js runtime and
//! the `sdkck` package whose plugins the app drives.
//!
//! Desktop apps launched from a dock or start menu do not inherit the PATH a
//! terminal has, so discovery also probes the usual install locations (Homebrew,
//! nvm, volta, fnm, the Windows installer) instead of relying on PATH alone.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::env::{Env, Platform};
use crate::manifest::{self, PackageJson};
use crate::{Error, Result};

/// sdkck's `engines.node`.
pub const MIN_NODE_MAJOR: u32 = 22;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    pub path: PathBuf,
    pub version: String,
    pub major: u32,
    pub supported: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SdkckOrigin {
    /// Chosen explicitly in settings.
    Custom,
    /// Installed by the app into its own runtime directory.
    Managed,
    /// Found on PATH / in the global npm prefix.
    Global,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SdkckInfo {
    pub root: PathBuf,
    pub version: Option<String>,
    pub origin: SdkckOrigin,
}

impl SdkckInfo {
    pub fn run_js(&self) -> PathBuf {
        self.root.join("bin").join("run.js")
    }
}

fn exe(name: &str, platform: Platform) -> String {
    if platform == Platform::Windows {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Directories that commonly hold a `node` binary but are often missing from
/// a GUI app's PATH.
fn extra_bin_dirs(env: &Env) -> Vec<PathBuf> {
    let home = env.home();
    let mut dirs = vec![];
    match env.platform() {
        Platform::Windows => {
            for var in ["ProgramFiles", "ProgramFiles(x86)"] {
                if let Some(p) = env.get(var) {
                    dirs.push(PathBuf::from(p).join("nodejs"));
                }
            }
            if let Some(p) = env.get("APPDATA") {
                dirs.push(PathBuf::from(p).join("npm"));
            }
        }
        _ => {
            dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
            dirs.push(home.join(".volta/bin"));
            dirs.push(home.join(".local/bin"));
            dirs.push(home.join(".npm-global/bin"));
        }
    }
    // Version managers: newest installed version first.
    for base in [
        home.join(".nvm/versions/node"),
        home.join(".local/share/fnm/node-versions"),
        home.join("Library/Application Support/fnm/node-versions"),
    ] {
        let Ok(entries) = std::fs::read_dir(&base) else { continue };
        let mut versions: Vec<(Vec<u32>, PathBuf)> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .map(|p| {
                let v = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .trim_start_matches('v')
                    .split('.')
                    .filter_map(|n| n.parse().ok())
                    .collect();
                (v, p)
            })
            .collect();
        versions.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, p) in versions {
            let bin = if p.join("installation/bin").exists() { p.join("installation/bin") } else { p.join("bin") };
            dirs.push(bin);
        }
    }
    dirs
}

/// `which` over PATH, then the extra directories.
pub fn find_executable(env: &Env, name: &str) -> Option<PathBuf> {
    let path = env.get("PATH").or_else(|| env.get("Path")).unwrap_or_default().to_string();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Ok(found) = which::which_in(name, Some(&path), &cwd) {
        return Some(found);
    }
    let file = exe(name, env.platform());
    extra_bin_dirs(env).into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

pub fn node_info(path: &Path) -> Result<NodeInfo> {
    let out = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::Runtime(format!("Could not run {}: {e}", path.display())))?;
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let major = version.trim_start_matches('v').split('.').next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Ok(NodeInfo { path: path.to_path_buf(), version, major, supported: major >= MIN_NODE_MAJOR })
}

/// The configured node, else the first supported one found.
pub fn find_node(env: &Env, configured: Option<&Path>) -> Option<NodeInfo> {
    if let Some(p) = configured {
        return node_info(p).ok();
    }
    let first = find_executable(env, "node").and_then(|p| node_info(&p).ok());
    if first.as_ref().is_some_and(|n| n.supported) {
        return first;
    }
    // PATH's node is too old (or missing): look for a newer one elsewhere.
    let file = exe("node", env.platform());
    extra_bin_dirs(env)
        .into_iter()
        .map(|d| d.join(&file))
        .filter(|p| p.is_file())
        .filter_map(|p| node_info(&p).ok())
        .find(|n| n.supported)
        .or(first)
}

fn sdkck_info(root: &Path, origin: SdkckOrigin) -> Option<SdkckInfo> {
    let pjson: PackageJson = manifest::read_json(&root.join("package.json")).ok()?;
    root.join("bin/run.js").is_file().then(|| SdkckInfo { root: root.to_path_buf(), version: pjson.version, origin })
}

/// Where the app installs its own copy of sdkck.
pub fn managed_package_root(managed_prefix: &Path) -> PathBuf {
    managed_prefix.join("node_modules").join("sdkck")
}

/// Settings override > the app-managed install > a global install.
pub fn find_sdkck(env: &Env, configured: Option<&Path>, managed_prefix: &Path) -> Option<SdkckInfo> {
    if let Some(root) = configured {
        return sdkck_info(root, SdkckOrigin::Custom);
    }
    if let Some(info) = sdkck_info(&managed_package_root(managed_prefix), SdkckOrigin::Managed) {
        return Some(info);
    }
    let shim = find_executable(env, "sdkck")?;
    // POSIX: a symlink to <root>/bin/run.js. Windows: an npm .cmd shim beside
    // node_modules/sdkck.
    let resolved = std::fs::canonicalize(&shim).unwrap_or(shim.clone());
    let candidates = [
        resolved.parent().and_then(Path::parent).map(Path::to_path_buf),
        shim.parent().map(|d| d.join("node_modules").join("sdkck")),
        shim.parent().and_then(Path::parent).map(|d| d.join("lib").join("node_modules").join("sdkck")),
    ];
    candidates.into_iter().flatten().find_map(|root| sdkck_info(&root, SdkckOrigin::Global))
}

/// npm's CLI entry next to a node binary, run through that same node so the
/// install does not depend on PATH or on Windows `.cmd` shims.
pub fn npm_cli(node: &Path) -> Option<PathBuf> {
    let dir = node.parent()?;
    let candidates = [
        dir.join("node_modules/npm/bin/npm-cli.js"),
        dir.join("../lib/node_modules/npm/bin/npm-cli.js"),
        dir.join("../libexec/lib/node_modules/npm/bin/npm-cli.js"),
    ];
    candidates.into_iter().find(|p| p.is_file()).map(|p| std::fs::canonicalize(&p).unwrap_or(p))
}

/// Build a `npm <args>` command run by `node`, falling back to `npm` on PATH.
pub fn npm_command(env: &Env, node: &Path, args: &[&str]) -> Result<Command> {
    let mut cmd = if let Some(cli) = npm_cli(node) {
        let mut c = Command::new(node);
        c.arg(cli);
        c
    } else if let Some(npm) = find_executable(env, if env.platform() == Platform::Windows { "npm.cmd" } else { "npm" }) {
        Command::new(npm)
    } else {
        return Err(Error::Runtime("npm was not found next to node or on PATH".into()));
    };
    cmd.args(args);
    Ok(cmd)
}

/// Install (or update) sdkck into the app-managed prefix, streaming npm's
/// output line by line.
pub fn install_sdkck(env: &Env, node: &Path, managed_prefix: &Path, version: &str, mut on_line: impl FnMut(&str)) -> Result<()> {
    std::fs::create_dir_all(managed_prefix).map_err(|e| Error::io(managed_prefix, e))?;
    let pjson = managed_prefix.join("package.json");
    if !pjson.exists() {
        std::fs::write(&pjson, "{\n  \"name\": \"sdkck-desktop-runtime\",\n  \"private\": true\n}\n")
            .map_err(|e| Error::io(&pjson, e))?;
    }
    let spec = format!("sdkck@{version}");
    let prefix = managed_prefix.to_string_lossy().to_string();
    let mut cmd =
        npm_command(env, node, &["install", "--prefix", &prefix, "--no-fund", "--no-audit", "--loglevel", "http", &spec])?;
    cmd.env_clear().envs(env.vars()).env("PATH", path_with_node(env, node));
    stream_command(cmd, &mut on_line)
}

/// PATH with node's directory first, so lifecycle scripts find the same node.
pub fn path_with_node(env: &Env, node: &Path) -> std::ffi::OsString {
    let existing = env.get("PATH").or_else(|| env.get("Path")).unwrap_or_default();
    let mut parts: Vec<PathBuf> = node.parent().map(Path::to_path_buf).into_iter().collect();
    parts.extend(std::env::split_paths(existing));
    std::env::join_paths(parts).unwrap_or_default()
}

pub(crate) fn stream_command(mut cmd: Command, on_line: &mut impl FnMut(&str)) -> Result<()> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Runtime(format!("Could not start {cmd:?}: {e}")))?;
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let tx2 = tx.clone();
    let t = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
            let _ = tx2.send(line);
        }
    });
    let stdout = child.stdout.take().unwrap();
    let t2 = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            let _ = tx.send(line);
        }
    });
    for line in rx {
        on_line(&crate::ansi::strip(&line));
    }
    let _ = t.join();
    let _ = t2.join();
    let status = child.wait().map_err(|e| Error::Runtime(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Runtime(format!("command exited with {status}")))
    }
}

/// Variables a login shell exports (`$SHELL -lic env`), for apps launched
/// outside a terminal. Keys already in `env` win, except PATH, which is taken
/// from the shell so tools installed there are found.
pub fn merge_login_shell_env(env: &Env) -> Env {
    if env.platform() == Platform::Windows {
        return env.clone();
    }
    let shell = env.get("SHELL").unwrap_or("/bin/sh").to_string();
    const MARK: &str = "__SDKCK_DESKTOP_ENV__";
    let script = format!("echo {MARK}; env -0 2>/dev/null || env");
    let child = Command::new(&shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return env.clone() };
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buf);
        buf
    });
    // A slow or interactive rc file must not hang startup.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return env.clone();
            }
        }
    }
    let out = reader.join().unwrap_or_default();
    let text = String::from_utf8_lossy(&out);
    let Some((_, body)) = text.split_once(&format!("{MARK}\n")) else { return env.clone() };
    let entries: Vec<&str> = if body.contains('\0') { body.split('\0').collect() } else { body.lines().collect() };
    let mut vars = env.vars().clone();
    for entry in entries {
        if let Some((k, v)) = entry.split_once('=') {
            let k = k.trim_start_matches('\n');
            if k.is_empty() || k.contains(char::is_whitespace) {
                continue;
            }
            if k == "PATH" || !vars.contains_key(k) {
                vars.insert(k.to_string(), v.to_string());
            }
        }
    }
    Env::new(vars, env.home(), env.platform())
}
