//! Lenient serde model of oclif's `oclif.manifest.json` and the `oclif`
//! section of a plugin's `package.json`. Only the fields the catalog needs are
//! modelled; everything defaults so manifests from any oclif major parse.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::{Error, Result};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub version: Option<String>,
    pub commands: BTreeMap<String, ManifestCommand>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ManifestCommand {
    pub id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub hidden: bool,
    pub plugin_name: Option<String>,
    pub plugin_type: Option<String>,
    pub usage: Option<Value>,
    pub examples: Option<Value>,
    /// Kept as raw JSON maps (insertion-ordered via `preserve_order`) because
    /// positional args are matched by declaration order.
    pub flags: serde_json::Map<String, Value>,
    pub args: serde_json::Map<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ManifestFlag {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub char: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub required: bool,
    pub multiple: bool,
    pub hidden: bool,
    pub allow_no: bool,
    pub options: Option<Vec<String>>,
    pub default: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ManifestArg {
    pub name: Option<String>,
    pub description: Option<String>,
    pub required: bool,
    pub options: Option<Vec<String>>,
    pub default: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PackageJson {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub oclif: OclifSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OclifSection {
    pub bin: Option<String>,
    /// Bundled plugins (root) — plain names, possibly globs like `@oclif/plugin-*`.
    /// In the data dir's package.json, objects `{name, type, root?}` instead.
    pub plugins: Vec<Value>,
    pub jit_plugins: BTreeMap<String, String>,
    pub topic_separator: Option<String>,
    pub topics: BTreeMap<String, Value>,
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let raw = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    serde_json::from_str(&raw).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
}

/// Flatten `oclif.topics` (`{jira: {description, subtopics: {...}}}`) into
/// colon-separated ids.
pub fn flatten_topics(topics: &BTreeMap<String, Value>, prefix: &str, out: &mut Vec<(String, Option<String>)>) {
    for (name, value) in topics {
        let id = if prefix.is_empty() { name.clone() } else { format!("{prefix}:{name}") };
        let description = value.get("description").and_then(Value::as_str).map(str::to_string);
        if value.get("hidden").and_then(Value::as_bool) != Some(true) {
            out.push((id.clone(), description));
        }
        if let Some(sub) = value.get("subtopics").and_then(Value::as_object) {
            let sub: BTreeMap<String, Value> = sub.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            flatten_topics(&sub, &id, out);
        }
    }
}
