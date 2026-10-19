//! Native core of sdkck Desktop.
//!
//! sdkck's plugins are oclif packages written in TypeScript, so running a
//! plugin command needs Node. Everything *around* running one does not: this
//! crate reads the CLI's manifests, plugin registry and stores directly to
//! list, search and permission-check commands in microseconds, manages plugins
//! through the CLI's own `plugins` commands, and runs commands either cold
//! (`node bin/run.js`) or in a pre-booted warm worker.

pub mod agent_vault;
pub mod ansi;
pub mod catalog;
pub mod dynamic;
pub mod engine;
pub mod env;
mod error;
pub mod manifest;
pub mod paths;
pub mod permission;
pub mod registry;
pub mod runner;
pub mod runtime;
pub mod search;

pub use error::{Error, Result};
