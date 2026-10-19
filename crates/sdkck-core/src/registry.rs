//! npm registry lookups for the plugin browser, done natively over HTTPS
//! instead of spawning `npm search`. Installation itself still goes through
//! `sdkck plugins install`, so oclif keeps owning the data dir's layout.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryPackage {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub keywords: Vec<String>,
    pub date: Option<String>,
    pub homepage: Option<String>,
}

#[derive(Deserialize)]
struct SearchResponse {
    objects: Vec<SearchObject>,
}

#[derive(Deserialize)]
struct SearchObject {
    package: SearchPackage,
}

#[derive(Deserialize)]
struct SearchPackage {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    links: Option<Links>,
}

#[derive(Deserialize)]
struct Links {
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    npm: Option<String>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .user_agent(concat!("sdkck-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// Search the registry. `registry` honours oclif's `SDKCK_NPM_REGISTRY`.
pub fn search(registry: Option<&str>, text: &str, size: usize) -> Result<Vec<RegistryPackage>> {
    let base = registry.unwrap_or(DEFAULT_REGISTRY).trim_end_matches('/');
    let url = format!("{base}/-/v1/search");
    let response: SearchResponse = agent()
        .get(&url)
        .query("text", text)
        .query("size", size.to_string())
        .call()
        .map_err(|e| Error::Runtime(format!("npm registry search failed: {e}")))?
        .body_mut()
        .read_json()
        .map_err(|e| Error::Runtime(format!("npm registry returned an unexpected response: {e}")))?;
    Ok(response
        .objects
        .into_iter()
        .map(|o| RegistryPackage {
            name: o.package.name,
            version: o.package.version,
            description: o.package.description,
            keywords: o.package.keywords,
            date: o.package.date,
            homepage: o.package.links.and_then(|l| l.homepage.or(l.npm)),
        })
        .collect())
}
