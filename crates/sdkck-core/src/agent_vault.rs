//! Detection of sdkck's command-wide Agent Vault interception, mirroring
//! `shouldIntercept` in sdkck's `src/agent-vault-process.ts`.
//!
//! The desktop app never brokers anything itself — the CLI's init hook does —
//! but it must know when interception is on: the warm runner executes commands
//! inside one long-lived Node process, which cannot re-exec itself into the
//! proxied child the hook needs. When interception is (or might be) on, every
//! command takes the cold path so the hook's fail-closed behaviour holds.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::env::Env;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Interception {
    Off,
    Platform {
        proxy: String,
    },
    Broker {
        vault: String,
    },
    /// Configured but unusable (lone platform field, unreadable file). The CLI
    /// fails such commands closed; the app routes them cold so it does too.
    Misconfigured {
        message: String,
    },
}

impl Interception {
    pub fn forces_cold_runs(&self) -> bool {
        !matches!(self, Interception::Off)
    }
}

pub fn detect(env: &Env, config_dir: &Path) -> Interception {
    if env.get_raw("SDKCK_AGENT_VAULT_ACTIVE").is_some_and(|v| !v.is_empty())
        || env.get_raw("SDKCK_AGENT_VAULT_DISABLED").is_some_and(|v| !v.is_empty())
    {
        return Interception::Off;
    }
    let path = config_dir.join("agent-vault.json");
    let file: serde_json::Map<String, Value> = if path.exists() {
        match std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            Some(Value::Object(o)) => o,
            _ => return Interception::Misconfigured { message: format!("{} is not a JSON object", path.display()) },
        }
    } else {
        Default::default()
    };
    let field = |env_key: &str, file_key: &str| -> Option<String> {
        env.get_raw(env_key).map(str::to_string).or_else(|| file.get(file_key).and_then(Value::as_str).map(str::to_string))
    };
    let session = field("AGENT_VAULT_SESSION_TOKEN", "sessionToken");
    let proxy = field("AGENT_VAULT_PROXY", "proxy");
    if session.is_some() || proxy.is_some() {
        return match (session.filter(|s| !s.is_empty()), proxy.filter(|p| !p.is_empty())) {
            (Some(_), Some(proxy)) => Interception::Platform { proxy },
            (Some(_), None) => Interception::Misconfigured {
                message: "AGENT_VAULT_PROXY is required alongside AGENT_VAULT_SESSION_TOKEN".into(),
            },
            _ => Interception::Misconfigured {
                message: "AGENT_VAULT_SESSION_TOKEN is required alongside AGENT_VAULT_PROXY".into(),
            },
        };
    }
    match (
        field("AGENT_VAULT_TOKEN", "token").filter(|t| !t.is_empty()),
        field("AGENT_VAULT_VAULT", "vault").filter(|v| !v.is_empty()),
    ) {
        (Some(_), Some(vault)) => Interception::Broker { vault },
        _ => Interception::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Platform;

    fn env(vars: &[(&str, &str)]) -> Env {
        Env::new(vars.iter().map(|(k, v)| (*k, *v)), "/h", Platform::Linux)
    }

    #[test]
    fn mirrors_should_intercept() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(&env(&[]), dir.path()), Interception::Off);
        assert_eq!(
            detect(&env(&[("AGENT_VAULT_TOKEN", "t"), ("AGENT_VAULT_VAULT", "v")]), dir.path()),
            Interception::Broker { vault: "v".into() }
        );
        assert!(matches!(detect(&env(&[("AGENT_VAULT_PROXY", "p:1")]), dir.path()), Interception::Misconfigured { .. }));
        assert_eq!(
            detect(&env(&[("AGENT_VAULT_PROXY", "p:1"), ("SDKCK_AGENT_VAULT_DISABLED", "1")]), dir.path()),
            Interception::Off
        );

        std::fs::write(dir.path().join("agent-vault.json"), r#"{"sessionToken":"agv_x","proxy":"h:17323"}"#).unwrap();
        assert_eq!(detect(&env(&[]), dir.path()), Interception::Platform { proxy: "h:17323".into() });
        std::fs::write(dir.path().join("agent-vault.json"), "[]").unwrap();
        assert!(detect(&env(&[]), dir.path()).forces_cold_runs());
    }
}
