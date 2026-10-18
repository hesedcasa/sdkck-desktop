//! Resolution of sdkck's config, data and cache directories, mirroring
//! oclif's `Config` (`scopedEnvVar('<CATEGORY>_DIR') || macosCacheDir() ||
//! dir(category)`) so the desktop app reads exactly the files the CLI writes.

use std::path::PathBuf;

use crate::env::{Env, Platform};

/// oclif `Config.dirname` for sdkck (its package name).
pub const DIRNAME: &str = "sdkck";
/// oclif scopes `<BIN>_*` env vars by the CLI's bin name.
const BIN_ENV_PREFIX: &str = "SDKCK";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SdkckDirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl SdkckDirs {
    pub fn resolve(env: &Env) -> Self {
        let scoped = |category: &str| env.get(&format!("{BIN_ENV_PREFIX}_{category}_DIR")).map(PathBuf::from);
        let macos_cache = (env.platform() == Platform::MacOs).then(|| env.home().join("Library").join("Caches").join(DIRNAME));
        SdkckDirs {
            config: scoped("CONFIG").unwrap_or_else(|| dir(env, "config")),
            data: scoped("DATA").unwrap_or_else(|| dir(env, "data")),
            cache: scoped("CACHE").or(macos_cache).unwrap_or_else(|| dir(env, "cache")),
        }
    }
}

/// oclif `Config.dir(category)`.
fn dir(env: &Env, category: &str) -> PathBuf {
    let xdg = env.get(&format!("XDG_{}_HOME", category.to_uppercase())).map(PathBuf::from);
    let local_app_data = (env.platform() == Platform::Windows).then(|| env.get("LOCALAPPDATA").map(PathBuf::from)).flatten();
    let base = xdg.or(local_app_data).unwrap_or_else(|| {
        let rel = if category == "data" { ".local/share".to_string() } else { format!(".{category}") };
        env.home().join(rel)
    });
    base.join(DIRNAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)], platform: Platform) -> Env {
        Env::new(vars.iter().map(|(k, v)| (*k, *v)), "/home/u", platform)
    }

    #[test]
    fn defaults_follow_oclif_on_linux() {
        let dirs = SdkckDirs::resolve(&env(&[], Platform::Linux));
        assert_eq!(dirs.config, PathBuf::from("/home/u/.config/sdkck"));
        assert_eq!(dirs.data, PathBuf::from("/home/u/.local/share/sdkck"));
        assert_eq!(dirs.cache, PathBuf::from("/home/u/.cache/sdkck"));
    }

    #[test]
    fn scoped_env_wins_then_xdg() {
        let dirs = SdkckDirs::resolve(&env(
            &[("SDKCK_CONFIG_DIR", "/c"), ("XDG_DATA_HOME", "/xdg"), ("SDKCK_CACHE_DIR", "")],
            Platform::Linux,
        ));
        assert_eq!(dirs.config, PathBuf::from("/c"));
        assert_eq!(dirs.data, PathBuf::from("/xdg/sdkck"));
        // An empty scoped var is unset, as with oclif's `||`.
        assert_eq!(dirs.cache, PathBuf::from("/home/u/.cache/sdkck"));
    }

    #[test]
    fn macos_cache_and_windows_local_app_data() {
        let mac = SdkckDirs::resolve(&env(&[], Platform::MacOs));
        assert_eq!(mac.cache, PathBuf::from("/home/u/Library/Caches/sdkck"));
        assert_eq!(mac.config, PathBuf::from("/home/u/.config/sdkck"));

        let win = SdkckDirs::resolve(&env(&[("LOCALAPPDATA", "/appdata")], Platform::Windows));
        assert_eq!(win.config, PathBuf::from("/appdata/sdkck"));
        assert_eq!(win.data, PathBuf::from("/appdata/sdkck"));
    }
}
