//! Plugins: Rhai scripts in a sandbox, installed from `.sfplugin` archives, with explicit permissions.

pub mod host;
pub mod manifest;
pub mod package;
pub mod runtime;
pub mod store;

#[cfg(test)]
mod sandbox_tests;

pub use manifest::{Manifest, Permissions};
pub use runtime::{ConsoleLine, Env, Level, Limits, Plugin, PluginError, Registrations, RunOutcome};
