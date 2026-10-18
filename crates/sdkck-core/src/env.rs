//! A snapshot of the process environment, injectable so path and
//! interception decisions are pure functions that tests can drive.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct Env {
    vars: BTreeMap<String, String>,
    home: Option<PathBuf>,
    platform: Platform,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    #[default]
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

impl Env {
    /// The real environment of this process.
    pub fn from_process() -> Self {
        let vars = std::env::vars().collect();
        Env { vars, home: std::env::home_dir(), platform: Platform::current() }
    }

    /// An explicit environment, for tests.
    pub fn new<I, K, V>(vars: I, home: impl Into<PathBuf>, platform: Platform) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Env { vars: vars.into_iter().map(|(k, v)| (k.into(), v.into())).collect(), home: Some(home.into()), platform }
    }

    /// The value of `key`, treating an empty string as unset — the same
    /// truthiness oclif's `process.env[k] ||` checks use.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }

    /// The value of `key` even when it is empty (`??` semantics).
    pub fn get_raw(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    pub fn home(&self) -> PathBuf {
        self.home.clone().unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn vars(&self) -> &BTreeMap<String, String> {
        &self.vars
    }
}
