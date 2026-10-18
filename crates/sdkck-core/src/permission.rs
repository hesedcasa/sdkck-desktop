//! Native port of `@hesed/permission`'s rule engine and `permission.json`
//! store, so the app can filter the catalog and edit rules without a Node
//! round-trip. Semantics (matching, evaluation order, legacy migration, the
//! on-disk v1 schema) must stay byte-for-byte with the plugin's
//! `permission-config.js`; the plugin's prerun hook remains the enforcement
//! point for every command that actually runs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const CONFIG_VERSION: u64 = 1;
const PERMISSION_TOPIC: &str = "permission";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub pattern: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionConfig {
    pub allow_rules: Vec<Rule>,
    pub deny_rules: Vec<Rule>,
}

impl PermissionConfig {
    /// What the plugin's hooks assume when no file exists: allow everything.
    pub fn allow_all() -> Self {
        PermissionConfig { allow_rules: vec![Rule { pattern: "*".into() }], deny_rules: vec![] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    PermissionCommand,
    DisallowRule,
    AllowRule,
    NoAllowRule,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Decision {
    pub allowed: bool,
    pub reason: Reason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    Allow,
    Deny,
}

pub fn config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("permission.json")
}

/// `readPermissionConfig`: `None` when the file does not exist; an error for
/// malformed JSON or an unsupported version, never a silent allow-all.
pub fn read(config_dir: &Path) -> Result<Option<PermissionConfig>> {
    let path = config_path(config_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(&path, e)),
    };
    let parsed: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| Error::Invalid(format!("Permission config at {} is not valid JSON: {e}", path.display())))?;
    let rules = |key: &str| -> Option<Vec<Rule>> {
        match parsed.get(key) {
            None | Some(serde_json::Value::Null) => None,
            Some(v) => Some(serde_json::from_value(v.clone()).unwrap_or_default()),
        }
    };
    match parsed.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v > CONFIG_VERSION => Err(Error::Invalid(format!(
            "Permission config at {} has version {v}, but this app only supports up to version {CONFIG_VERSION}. Update sdkck Desktop.",
            path.display()
        ))),
        Some(_) => Ok(Some(PermissionConfig {
            allow_rules: rules("allowRules").unwrap_or_default(),
            deny_rules: rules("denyRules").unwrap_or_default(),
        })),
        // Legacy file: "rules" was the deny list; a missing allowRules means allow-all.
        None => {
            let allow_rules = if parsed.get("allowRules").is_none() {
                vec![Rule { pattern: "*".into() }]
            } else {
                rules("allowRules").unwrap_or_default()
            };
            Ok(Some(PermissionConfig {
                allow_rules,
                deny_rules: rules("denyRules").or_else(|| rules("rules")).unwrap_or_default(),
            }))
        }
    }
}

/// The config the hooks would enforce right now.
pub fn effective(config_dir: &Path) -> Result<PermissionConfig> {
    Ok(read(config_dir)?.unwrap_or_else(PermissionConfig::allow_all))
}

/// `writePermissionConfig`: `{allowRules, denyRules, version}`, two-space indent.
pub fn write(config_dir: &Path, config: &PermissionConfig) -> Result<()> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Stored<'a> {
        allow_rules: &'a [Rule],
        deny_rules: &'a [Rule],
        version: u64,
    }
    std::fs::create_dir_all(config_dir).map_err(|e| Error::io(config_dir, e))?;
    let body = serde_json::to_string_pretty(&Stored {
        allow_rules: &config.allow_rules,
        deny_rules: &config.deny_rules,
        version: CONFIG_VERSION,
    })?;
    let path = config_path(config_dir);
    std::fs::write(&path, body).map_err(|e| Error::io(&path, e))
}

/// Start from the file (or an empty config, as the plugin's commands do) and
/// add a rule unless an equivalent one is already present.
pub fn add_rule(config_dir: &Path, kind: RuleKind, pattern: &str) -> Result<PermissionConfig> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err(Error::Invalid("Pattern must not be empty".into()));
    }
    let mut config = read(config_dir)?.unwrap_or(PermissionConfig { allow_rules: vec![], deny_rules: vec![] });
    let list = match kind {
        RuleKind::Allow => &mut config.allow_rules,
        RuleKind::Deny => &mut config.deny_rules,
    };
    let canonical = canonical_pattern(pattern);
    if !list.iter().any(|r| canonical_pattern(&r.pattern) == canonical) {
        list.push(Rule { pattern: pattern.to_string() });
    }
    write(config_dir, &config)?;
    Ok(config)
}

pub fn remove_rule(config_dir: &Path, kind: RuleKind, pattern: &str) -> Result<PermissionConfig> {
    let mut config = read(config_dir)?.unwrap_or(PermissionConfig { allow_rules: vec![], deny_rules: vec![] });
    let canonical = canonical_pattern(pattern);
    let list = match kind {
        RuleKind::Allow => &mut config.allow_rules,
        RuleKind::Deny => &mut config.deny_rules,
    };
    list.retain(|r| canonical_pattern(&r.pattern) != canonical);
    write(config_dir, &config)?;
    Ok(config)
}

/// `matchesPattern`, against a space-separated command id.
pub fn matches_pattern(command_id: &str, pattern: &str) -> bool {
    let p = pattern.trim();
    if p == "*" {
        return true;
    }
    if let Some(prefix) = p.strip_suffix(" *") {
        return command_id == prefix || command_id.starts_with(&format!("{prefix} "));
    }
    command_id == p || command_id.starts_with(&format!("{p} "))
}

/// `canonicalPattern`: "jira *" and "jira" match the same set.
pub fn canonical_pattern(pattern: &str) -> &str {
    let p = pattern.trim();
    if p == "*" {
        return p;
    }
    p.strip_suffix(" *").unwrap_or(p)
}

pub fn is_permission_command(command_id: &str) -> bool {
    command_id == PERMISSION_TOPIC || command_id.starts_with(&format!("{PERMISSION_TOPIC} "))
}

/// `explainCommandDecision`. Accepts colon- or space-separated ids.
pub fn explain(command_id: &str, config: &PermissionConfig) -> Decision {
    let id = command_id.replace(':', " ");
    if is_permission_command(&id) {
        return Decision { allowed: true, reason: Reason::PermissionCommand, pattern: None };
    }
    if let Some(rule) = config.deny_rules.iter().find(|r| matches_pattern(&id, &r.pattern)) {
        return Decision { allowed: false, reason: Reason::DisallowRule, pattern: Some(rule.pattern.clone()) };
    }
    if let Some(rule) = config.allow_rules.iter().find(|r| matches_pattern(&id, &r.pattern)) {
        return Decision { allowed: true, reason: Reason::AllowRule, pattern: Some(rule.pattern.clone()) };
    }
    Decision { allowed: false, reason: Reason::NoAllowRule, pattern: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(allow: &[&str], deny: &[&str]) -> PermissionConfig {
        let rules = |v: &[&str]| v.iter().map(|p| Rule { pattern: p.to_string() }).collect();
        PermissionConfig { allow_rules: rules(allow), deny_rules: rules(deny) }
    }

    #[test]
    fn pattern_forms() {
        assert!(matches_pattern("jira issue get", "*"));
        assert!(matches_pattern("jira issue get", "jira"));
        assert!(matches_pattern("jira", "jira *"));
        assert!(matches_pattern("jira issue get", "jira issue *"));
        assert!(matches_pattern("jira issue", "jira issue"));
        assert!(!matches_pattern("jiraa", "jira"));
        assert!(!matches_pattern("jira issue get", "jira iss"));
    }

    #[test]
    fn deny_wins_and_default_deny() {
        let c = cfg(&["jira *"], &["jira issue delete"]);
        assert!(!explain("jira:issue:delete", &c).allowed);
        assert_eq!(explain("jira:issue:get", &c).reason, Reason::AllowRule);
        assert_eq!(explain("bb repo list", &c).reason, Reason::NoAllowRule);
        assert!(explain("permission:allow", &cfg(&[], &["*"])).allowed);
    }

    #[test]
    fn round_trips_v1_and_migrates_legacy() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()).unwrap(), None);
        add_rule(dir.path(), RuleKind::Deny, "jira issue delete").unwrap();
        add_rule(dir.path(), RuleKind::Allow, "jira *").unwrap();
        add_rule(dir.path(), RuleKind::Allow, "jira").unwrap(); // duplicate of "jira *"
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config_path(dir.path())).unwrap()).unwrap();
        assert_eq!(raw["version"], 1);
        assert_eq!(raw["allowRules"].as_array().unwrap().len(), 1);
        remove_rule(dir.path(), RuleKind::Allow, "jira").unwrap();
        assert!(read(dir.path()).unwrap().unwrap().allow_rules.is_empty());

        std::fs::write(config_path(dir.path()), r#"{"rules":[{"pattern":"bb"}]}"#).unwrap();
        let legacy = read(dir.path()).unwrap().unwrap();
        assert_eq!(legacy, cfg(&["*"], &["bb"]));

        std::fs::write(config_path(dir.path()), r#"{"version":9}"#).unwrap();
        assert!(read(dir.path()).is_err());
        std::fs::write(config_path(dir.path()), "{nope").unwrap();
        assert!(read(dir.path()).is_err());
    }
}
