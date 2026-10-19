//! The command catalog, built natively from the same files oclif reads at
//! startup — the root package's manifest, each bundled and installed plugin's
//! manifest, and the dynamic-command stores — instead of booting Node and
//! running `sdkck commands --json`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use serde::Serialize;
use serde_json::Value;

use crate::manifest::{self, Manifest, ManifestCommand, PackageJson};
use crate::paths::SdkckDirs;
use crate::permission::{self, Decision, PermissionConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagKind {
    Boolean,
    Option,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlagMeta {
    pub name: String,
    pub char: Option<String>,
    pub description: Option<String>,
    pub required: bool,
    pub multiple: bool,
    pub kind: FlagKind,
    pub allow_no: bool,
    pub options: Option<Vec<String>>,
    pub default: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArgMeta {
    pub name: String,
    pub description: Option<String>,
    pub required: bool,
    pub options: Option<Vec<String>>,
    pub default: Option<Value>,
}

/// Where a command came from, which decides how the UI presents it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandSource {
    /// Shipped in a plugin's manifest.
    Plugin,
    /// Declared by a JIT plugin that is not installed yet; the first run installs it.
    Jit,
    /// An imported API operation.
    Api,
    /// A cached tool of a connected MCP server.
    Mcp,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandMeta {
    /// oclif's colon-separated id (`jira:issue:get`).
    pub id: String,
    /// How the user types it (`jira issue get`).
    pub display_id: String,
    pub topic: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub args: Vec<ArgMeta>,
    pub flags: Vec<FlagMeta>,
    pub examples: Vec<String>,
    pub plugin_name: String,
    pub plugin_type: String,
    pub source: CommandSource,
    pub permission: Decision,
}

impl CommandMeta {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: String,
        summary: Option<String>,
        description: Option<String>,
        args: Vec<ArgMeta>,
        flags: Vec<FlagMeta>,
        plugin_name: &str,
        plugin_type: &str,
        source: CommandSource,
    ) -> Self {
        CommandMeta {
            display_id: id.replace(':', " "),
            topic: id.split(':').next().unwrap_or_default().to_string(),
            id,
            summary,
            description,
            aliases: vec![],
            args,
            flags,
            examples: vec![],
            plugin_name: plugin_name.to_string(),
            plugin_type: plugin_type.to_string(),
            source,
            permission: Decision { allowed: true, reason: permission::Reason::AllowRule, pattern: None },
        }
    }

    /// The argv a shell would pass for this command id.
    pub fn id_argv(&self) -> Vec<String> {
        self.id.split(':').map(str::to_string).collect()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    /// `root`, `core` (bundled), `user`, `link`, or `jit` (declared, not installed).
    pub kind: String,
    pub root: Option<PathBuf>,
    pub installed: bool,
    pub command_count: usize,
    /// The tag/version JIT installs pin, or a user plugin's install tag.
    pub tag: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Topic {
    pub id: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub bin: String,
    pub version: Option<String>,
    pub commands: Vec<CommandMeta>,
    pub plugins: Vec<PluginInfo>,
    pub topics: Vec<Topic>,
    pub warnings: Vec<String>,
    /// Commands hidden by the permission allowlist (excluded from `commands`).
    pub blocked: Vec<CommandMeta>,
    pub build_micros: u128,
    #[serde(skip)]
    pub fingerprint: Fingerprint,
}

/// Modification state of every file the catalog was built from, so a cached
/// catalog can be revalidated with a handful of `stat` calls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fingerprint(BTreeMap<PathBuf, Option<(SystemTime, u64)>>);

impl Fingerprint {
    fn add(&mut self, path: &Path) {
        self.0.insert(path.to_path_buf(), stat(path));
    }

    pub fn is_current(&self) -> bool {
        !self.0.is_empty() && self.0.iter().all(|(p, s)| stat(p) == *s)
    }
}

fn stat(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Inputs to a catalog build.
pub struct CatalogSource<'a> {
    /// The installed `sdkck` package directory (contains `package.json`).
    pub root: &'a Path,
    pub dirs: &'a SdkckDirs,
}

struct Pending {
    cmd: CommandMeta,
    rank: u8,
}

/// Priority when two plugins provide the same id: lower wins. Bundled beats
/// user-installed, which beats linked, which beats JIT placeholders.
fn rank(plugin_type: &str) -> u8 {
    match plugin_type {
        "core" => 0,
        "user" => 1,
        "link" | "dev" => 2,
        "jit" => 3,
        _ => 4,
    }
}

pub fn build(src: &CatalogSource) -> crate::Result<Catalog> {
    let started = Instant::now();
    let mut fp = Fingerprint::default();
    let mut warnings = vec![];
    let mut plugins: Vec<PluginInfo> = vec![];
    let mut commands: HashMap<String, Pending> = HashMap::new();
    let mut topics: BTreeMap<String, Option<String>> = BTreeMap::new();

    let root_pjson_path = src.root.join("package.json");
    fp.add(&root_pjson_path);
    let root_pjson: PackageJson = manifest::read_json(&root_pjson_path)?;
    let root_name = root_pjson.name.clone().unwrap_or_else(|| "sdkck".into());
    let bin = root_pjson.oclif.bin.clone().unwrap_or_else(|| root_name.clone());

    let mut insert = |cmd: CommandMeta, rank: u8| match commands.get(&cmd.id) {
        Some(existing) if existing.rank <= rank => {}
        _ => {
            commands.insert(cmd.id.clone(), Pending { cmd, rank });
        }
    };

    // User / linked plugins from the data dir.
    let data_pjson_path = src.dirs.data.join("package.json");
    fp.add(&data_pjson_path);
    let mut user_plugins: Vec<(String, String, PathBuf, Option<String>)> = vec![];
    if data_pjson_path.exists() {
        match manifest::read_json::<PackageJson>(&data_pjson_path) {
            Ok(pjson) => {
                for entry in &pjson.oclif.plugins {
                    let Some(name) = entry.get("name").and_then(Value::as_str) else { continue };
                    let kind = entry.get("type").and_then(Value::as_str).unwrap_or("user").to_string();
                    let tag = entry.get("tag").and_then(Value::as_str).map(str::to_string);
                    let dir = match entry.get("root").and_then(Value::as_str) {
                        Some(r) if kind == "link" => PathBuf::from(r),
                        _ => src.dirs.data.join("node_modules").join(name),
                    };
                    user_plugins.push((name.to_string(), kind, dir, tag));
                }
            }
            Err(e) => warnings.push(format!("Ignoring installed plugins: {e}")),
        }
    }
    let installed_names: BTreeSet<String> = user_plugins.iter().map(|(n, ..)| n.clone()).collect();

    // Root manifest (includes placeholder entries for every JIT plugin).
    let mut root_count = 0;
    let mut jit_counts: BTreeMap<String, usize> = BTreeMap::new();
    let root_manifest = src.root.join("oclif.manifest.json");
    fp.add(&root_manifest);
    match load_manifest(&root_manifest) {
        Some(Ok(m)) => {
            for (id, mc) in m.commands {
                let plugin = mc.plugin_name.clone().unwrap_or_else(|| root_name.clone());
                let ptype = mc.plugin_type.clone().unwrap_or_else(|| "core".into());
                if ptype == "jit" {
                    if installed_names.contains(&plugin) {
                        continue;
                    }
                    *jit_counts.entry(plugin.clone()).or_default() += 1;
                } else {
                    root_count += 1;
                }
                if mc.hidden {
                    continue;
                }
                let source = if ptype == "jit" { CommandSource::Jit } else { CommandSource::Plugin };
                insert(from_manifest(&id, mc, &plugin, &ptype, source, &bin), rank(&ptype));
            }
        }
        Some(Err(e)) => warnings.push(e.to_string()),
        None => warnings.push(format!("{} has no oclif.manifest.json; its own commands are not listed", root_name)),
    }
    collect_topics(&root_pjson, &mut topics);
    plugins.push(PluginInfo {
        name: root_name.clone(),
        version: root_pjson.version.clone(),
        description: root_pjson.description.clone(),
        kind: "root".into(),
        root: Some(src.root.to_path_buf()),
        installed: true,
        command_count: root_count,
        tag: None,
    });

    // Bundled (core) plugins.
    for spec in root_pjson.oclif.plugins.iter().filter_map(Value::as_str) {
        let dirs = resolve_package(src.root, spec);
        if dirs.is_empty() {
            warnings.push(format!("Bundled plugin {spec} could not be resolved"));
        }
        for dir in dirs {
            load_plugin(&dir, "core", None, &bin, &mut fp, &mut warnings, &mut plugins, &mut topics, &mut insert);
        }
    }

    // Installed plugins.
    for (name, kind, dir, tag) in &user_plugins {
        if !dir.join("package.json").exists() {
            warnings.push(format!("Plugin {name} is registered but missing from {}", dir.display()));
            continue;
        }
        load_plugin(dir, kind, tag.clone(), &bin, &mut fp, &mut warnings, &mut plugins, &mut topics, &mut insert);
    }

    // JIT plugins that are declared but not yet installed.
    for (name, tag) in &root_pjson.oclif.jit_plugins {
        if installed_names.contains(name) {
            continue;
        }
        plugins.push(PluginInfo {
            name: name.clone(),
            version: None,
            description: None,
            kind: "jit".into(),
            root: None,
            installed: false,
            command_count: jit_counts.get(name).copied().unwrap_or(0),
            tag: Some(tag.clone()),
        });
    }

    // Dynamic commands, registered only when absent (`if (_commands.has(id)) continue`).
    fp.add(&src.dirs.config);
    let mut dynamic = vec![crate::dynamic::api_commands(&src.dirs.config, &root_name)];
    // bin/run.js registers MCP tools only once @hesed/mcp-client is installed.
    let mcp_entry = src.dirs.data.join("node_modules/@hesed/mcp-client/dist/index.js");
    fp.add(&mcp_entry);
    if mcp_entry.exists() {
        dynamic.push(crate::dynamic::mcp_commands(&src.dirs.config, &root_name));
    }
    for set in dynamic {
        for f in &set.files {
            fp.add(f);
        }
        warnings.extend(set.warnings);
        for (id, desc) in set.topics {
            topics.entry(id).or_insert(desc);
        }
        // Ranked below everything, so an existing id always wins.
        for cmd in set.commands {
            insert(cmd, u8::MAX);
        }
    }

    // Permission allowlist, as the init hook applies it.
    let perm_path = permission::config_path(&src.dirs.config);
    fp.add(&perm_path);
    let perm = match permission::effective(&src.dirs.config) {
        Ok(p) => p,
        Err(e) => {
            // The hooks throw on a corrupt file, so every command fails; show that.
            warnings.push(e.to_string());
            PermissionConfig { allow_rules: vec![], deny_rules: vec![] }
        }
    };

    let mut all: Vec<CommandMeta> = commands.into_values().map(|p| p.cmd).collect();
    all.sort_by(|a, b| a.id.cmp(&b.id));
    let (mut visible, mut blocked) = (vec![], vec![]);
    for mut cmd in all {
        cmd.permission = permission::explain(&cmd.id, &perm);
        if cmd.permission.allowed {
            visible.push(cmd)
        } else {
            blocked.push(cmd)
        }
    }

    // Every command's top-level topic exists even when no plugin describes it.
    for cmd in &visible {
        topics.entry(cmd.topic.clone()).or_insert(None);
    }

    Ok(Catalog {
        bin,
        version: root_pjson.version,
        commands: visible,
        plugins,
        topics: topics
            .into_iter()
            .filter(|(id, _)| !id.contains(':'))
            .map(|(id, description)| Topic { id, description })
            .collect(),
        warnings,
        blocked,
        build_micros: started.elapsed().as_micros(),
        fingerprint: fp,
    })
}

fn load_manifest(path: &Path) -> Option<crate::Result<Manifest>> {
    path.exists().then(|| manifest::read_json(path))
}

#[allow(clippy::too_many_arguments)]
fn load_plugin(
    dir: &Path,
    kind: &str,
    tag: Option<String>,
    bin: &str,
    fp: &mut Fingerprint,
    warnings: &mut Vec<String>,
    plugins: &mut Vec<PluginInfo>,
    topics: &mut BTreeMap<String, Option<String>>,
    insert: &mut impl FnMut(CommandMeta, u8),
) {
    let pjson_path = dir.join("package.json");
    fp.add(&pjson_path);
    let pjson: PackageJson = match manifest::read_json(&pjson_path) {
        Ok(p) => p,
        Err(e) => {
            warnings.push(e.to_string());
            return;
        }
    };
    let name = pjson.name.clone().unwrap_or_else(|| dir.display().to_string());
    if plugins.iter().any(|p| p.name == name) {
        return;
    }
    let manifest_path = dir.join("oclif.manifest.json");
    fp.add(&manifest_path);
    let mut count = 0;
    match load_manifest(&manifest_path) {
        Some(Ok(m)) => {
            for (id, mc) in m.commands {
                count += 1;
                if mc.hidden {
                    continue;
                }
                insert(from_manifest(&id, mc, &name, kind, CommandSource::Plugin, bin), rank(kind));
            }
        }
        Some(Err(e)) => warnings.push(e.to_string()),
        None => warnings.push(format!(
            "Plugin {name} has no oclif.manifest.json; its commands still run but are not listed (rebuild it with `oclif manifest`)"
        )),
    }
    collect_topics(&pjson, topics);
    plugins.push(PluginInfo {
        name,
        version: pjson.version,
        description: pjson.description,
        kind: kind.to_string(),
        root: Some(dir.to_path_buf()),
        installed: true,
        command_count: count,
        tag,
    });
}

fn collect_topics(pjson: &PackageJson, topics: &mut BTreeMap<String, Option<String>>) {
    let mut flat = vec![];
    manifest::flatten_topics(&pjson.oclif.topics, "", &mut flat);
    for (id, desc) in flat {
        let entry = topics.entry(id).or_insert(None);
        if entry.is_none() {
            *entry = desc;
        }
    }
}

/// Render oclif's EJS-style placeholders the way help output shows them.
fn render(text: &str, bin: &str, display_id: &str) -> String {
    let mut out = text.to_string();
    for (pat, value) in [("config.bin", bin), ("command.id", display_id)] {
        for spaced in [format!("<%= {pat} %>"), format!("<%={pat}%>"), format!("<%- {pat} %>")] {
            out = out.replace(&spaced, value);
        }
    }
    out
}

fn from_manifest(id: &str, mc: ManifestCommand, plugin: &str, ptype: &str, source: CommandSource, bin: &str) -> CommandMeta {
    let display_id = id.replace(':', " ");
    let r = |s: Option<String>| s.map(|t| render(&t, bin, &display_id));
    let flags = mc
        .flags
        .into_iter()
        .filter_map(|(k, v)| serde_json::from_value::<manifest::ManifestFlag>(v).ok().map(|f| (k, f)))
        .filter(|(_, f)| !f.hidden)
        .map(|(key, f)| {
            let kind = if f.kind.as_deref() == Some("boolean") { FlagKind::Boolean } else { FlagKind::Option };
            FlagMeta {
                name: f.name.unwrap_or(key),
                char: f.char,
                description: r(f.summary.or(f.description)),
                required: f.required,
                multiple: kind == FlagKind::Option && f.multiple,
                kind,
                allow_no: f.allow_no,
                options: if kind == FlagKind::Boolean { None } else { f.options },
                default: f.default,
            }
        })
        .collect();
    let args = mc
        .args
        .into_iter()
        .filter_map(|(k, v)| serde_json::from_value::<manifest::ManifestArg>(v).ok().map(|a| (k, a)))
        .map(|(key, a)| ArgMeta {
            name: a.name.unwrap_or(key),
            description: r(a.description),
            required: a.required,
            options: a.options,
            default: a.default,
        })
        .collect();
    let examples = match mc.examples {
        Some(Value::Array(v)) => v
            .into_iter()
            .filter_map(|e| match e {
                Value::String(s) => Some(s),
                Value::Object(o) => o.get("command").and_then(Value::as_str).map(str::to_string),
                _ => None,
            })
            .map(|e| render(&e, bin, &display_id))
            .collect(),
        Some(Value::String(s)) => vec![render(&s, bin, &display_id)],
        _ => vec![],
    };
    CommandMeta {
        display_id: display_id.clone(),
        topic: id.split(':').next().unwrap_or_default().to_string(),
        id: id.to_string(),
        summary: r(mc.summary),
        description: r(mc.description),
        aliases: mc.aliases,
        args,
        flags,
        examples,
        plugin_name: plugin.to_string(),
        plugin_type: ptype.to_string(),
        source,
        permission: Decision { allowed: true, reason: permission::Reason::AllowRule, pattern: None },
    }
}

/// Node's module resolution for a bare specifier from `from`: every ancestor's
/// `node_modules/<spec>`. A `scope/prefix-*` glob expands to every match in the
/// nearest directory that has any.
pub fn resolve_package(from: &Path, spec: &str) -> Vec<PathBuf> {
    for ancestor in from.ancestors() {
        let nm = ancestor.join("node_modules");
        if let Some(prefix) = spec.strip_suffix('*') {
            let (scope_dir, name_prefix) = match prefix.rsplit_once('/') {
                Some((scope, p)) => (nm.join(scope), p),
                None => (nm.clone(), prefix),
            };
            let Ok(entries) = std::fs::read_dir(&scope_dir) else { continue };
            let mut found: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with(name_prefix))
                .map(|e| e.path())
                .filter(|p| p.join("package.json").exists())
                .collect();
            if !found.is_empty() {
                found.sort();
                return found;
            }
        } else {
            let candidate = nm.join(spec);
            if candidate.join("package.json").exists() {
                return vec![candidate];
            }
        }
    }
    vec![]
}
