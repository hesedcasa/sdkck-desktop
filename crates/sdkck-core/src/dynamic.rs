//! Commands sdkck registers at startup from on-disk stores rather than from
//! plugin manifests: imported API operations (`@hesed/api2cli`,
//! `<configDir>/api-<name>.json`) and cached MCP server tools
//! (`@hesed/mcp-client`, `<configDir>/mcp-client-<name>.json`).
//!
//! The arg/flag shapes mirror `createOperationCommand` and
//! `createMcpToolCommand` so the generated forms build the same argv the CLI
//! parses.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::catalog::{ArgMeta, CommandMeta, CommandSource, FlagKind, FlagMeta};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSpec {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    operations: Vec<StoredOperation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredOperation {
    operation_id: String,
    method: String,
    path: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    parameters: Vec<StoredParam>,
    #[serde(default)]
    body_params: BTreeMap<String, BodyParam>,
    #[serde(default)]
    raw_body_content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StoredParam {
    name: String,
    #[serde(rename = "in", default)]
    location: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BodyParam {
    #[serde(default)]
    required: bool,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpServerFile {
    #[serde(default)]
    cached_tools: Vec<McpTool>,
    config: McpServerConfig,
}

#[derive(Debug, Deserialize)]
struct McpServerConfig {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpTool {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    input_schema: Value,
}

pub struct DynamicSet {
    pub commands: Vec<CommandMeta>,
    pub topics: Vec<(String, Option<String>)>,
    pub warnings: Vec<String>,
    /// Store files read, for change detection.
    pub files: Vec<PathBuf>,
}

fn list_config_files(config_dir: &Path, matches: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(config_dir) else { return vec![] };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(&matches))
        .collect();
    files.sort();
    files
}

fn flag(name: &str, kind: FlagKind, description: impl Into<String>) -> FlagMeta {
    FlagMeta {
        name: name.to_string(),
        char: None,
        description: Some(description.into()),
        required: false,
        multiple: false,
        kind,
        allow_no: false,
        options: None,
        default: None,
    }
}

/// `readStore` + `registerApiCommands`.
pub fn api_commands(config_dir: &Path, plugin_name: &str) -> DynamicSet {
    let mut set = DynamicSet { commands: vec![], topics: vec![], warnings: vec![], files: vec![] };
    let is_spec_file = |n: &str| (n.starts_with("api-") || n.starts_with("openapi-")) && n.ends_with(".json") && n.len() > 9;
    let mut files = list_config_files(config_dir, is_spec_file);
    // api-<name>.json wins over the legacy openapi-<name>.json.
    files.sort_by_key(|p| !p.file_name().unwrap().to_string_lossy().starts_with("api-"));
    let mut seen = std::collections::BTreeSet::new();
    for file in files {
        let spec: StoredSpec = match crate::manifest::read_json(&file) {
            Ok(s) => s,
            Err(e) => {
                set.warnings.push(format!("Failed to load spec file: {e}"));
                continue;
            }
        };
        set.files.push(file);
        if !seen.insert(spec.name.clone()) {
            continue;
        }
        set.topics.push((spec.name.clone(), spec.description.clone().filter(|d| !d.is_empty()).or(spec.title.clone())));
        for op in &spec.operations {
            set.commands.push(api_operation(&spec.name, op, plugin_name));
        }
    }
    set
}

fn api_operation(spec_name: &str, op: &StoredOperation, plugin_name: &str) -> CommandMeta {
    let url_param_names: std::collections::BTreeSet<&str> = op.parameters.iter().map(|p| p.name.as_str()).collect();
    let output_shadowed = op.parameters.iter().any(|p| !p.required && p.name == "output")
        || (!url_param_names.contains("output") && op.body_params.get("output").is_some_and(|b| !b.required));

    let mut flags: Vec<FlagMeta> = vec![];
    let mut put = |f: FlagMeta| {
        // Later definitions overwrite same-named ones, as with a JS object literal.
        if let Some(existing) = flags.iter_mut().find(|e| e.name == f.name) {
            *existing = f;
        } else {
            flags.push(f);
        }
    };
    put(FlagMeta { multiple: true, ..flag("header", FlagKind::Option, "Extra request header as Key=Value (repeatable)") });
    put(FlagMeta { char: Some("p".into()), ..flag("profile", FlagKind::Option, "Authentication profile name") });
    put(flag("toon", FlagKind::Boolean, "Encode JSON output with TOON for token-efficient LLM consumption"));
    if !output_shadowed {
        put(FlagMeta {
            char: Some("o".into()),
            ..flag(
                "output",
                FlagKind::Option,
                "Write the raw response body to a file (required for binary responses such as zip archives)",
            )
        });
    }
    if let Some(ct) = &op.raw_body_content_type {
        put(flag("body", FlagKind::Option, format!("Raw request body ({ct}). Reads from stdin if omitted.")));
    }

    let mut args: Vec<ArgMeta> = vec![];
    for param in &op.parameters {
        let description = param.description.clone().unwrap_or_else(|| param.name.clone());
        if param.required {
            args.push(ArgMeta {
                name: param.name.clone(),
                description: Some(description),
                required: true,
                options: None,
                default: None,
            });
        } else {
            put(flag(&param.name, FlagKind::Option, format!("[{}] {description}", param.location)));
        }
    }
    for (name, body) in &op.body_params {
        let resolved = if url_param_names.contains(name.as_str()) { format!("body-{name}") } else { name.clone() };
        let description = format!(
            "{} ({})",
            body.description.clone().unwrap_or_else(|| name.clone()),
            body.kind.as_deref().unwrap_or("string")
        );
        if body.required {
            args.push(ArgMeta { name: resolved, description: Some(description), required: true, options: None, default: None });
        } else {
            put(flag(&resolved, FlagKind::Option, description));
        }
    }

    let description =
        op.description.clone().filter(|d| !d.is_empty()).unwrap_or_else(|| format!("{} {}", op.method.to_uppercase(), op.path));
    CommandMeta::new(
        format!("{spec_name}:{}", op.operation_id),
        Some(format!("{} {}", op.method.to_uppercase(), op.path)),
        Some(description),
        args,
        flags,
        plugin_name,
        "core",
        CommandSource::Api,
    )
}

/// `listServerFiles` + `registerMcpClientCommands`.
pub fn mcp_commands(config_dir: &Path, plugin_name: &str) -> DynamicSet {
    let mut set = DynamicSet { commands: vec![], topics: vec![], warnings: vec![], files: vec![] };
    let is_server_file = |n: &str| n.starts_with("mcp-client-") && n.ends_with(".json") && !n.ends_with("-oauth.json");
    for file in list_config_files(config_dir, is_server_file) {
        let server: McpServerFile = match crate::manifest::read_json(&file) {
            Ok(s) => s,
            Err(e) => {
                set.warnings.push(format!("Failed to load MCP server file: {e}"));
                continue;
            }
        };
        set.files.push(file);
        if server.cached_tools.is_empty() {
            continue;
        }
        let server_name = server.config.name;
        set.topics.push((server_name.clone(), Some(format!("MCP tools from {server_name}"))));
        for tool in &server.cached_tools {
            set.commands.push(mcp_tool(&server_name, tool, plugin_name));
        }
    }
    set
}

fn mcp_tool(server_name: &str, tool: &McpTool, plugin_name: &str) -> CommandMeta {
    let required: Vec<&str> = tool
        .input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut flags: Vec<FlagMeta> = vec![];
    if let Some(props) = tool.input_schema.get("properties").and_then(Value::as_object) {
        for (name, prop) in props {
            let is_required = required.contains(&name.as_str());
            let description = format!(
                "{}{}",
                if is_required { "(required) " } else { "" },
                prop.get("description").and_then(Value::as_str).unwrap_or(name)
            );
            let kind =
                if prop.get("type").and_then(Value::as_str) == Some("boolean") { FlagKind::Boolean } else { FlagKind::Option };
            let options = prop
                .get("enum")
                .and_then(Value::as_array)
                .map(|e| e.iter().filter_map(Value::as_str).map(str::to_string).collect());
            flags.push(FlagMeta { options, ..flag(name, kind, description) });
        }
    }
    flags.push(flag("json-args", FlagKind::Option, "Tool arguments as a JSON object (overrides individual flags)"));
    flags.push(flag("toon", FlagKind::Boolean, "Encode JSON output with TOON for token-efficient LLM consumption"));
    let description = tool.description.clone().unwrap_or_else(|| tool.name.clone());
    CommandMeta::new(
        format!("{server_name}:{}", tool.name),
        None,
        Some(description),
        vec![],
        flags,
        plugin_name,
        "core",
        CommandSource::Mcp,
    )
}
