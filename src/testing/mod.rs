//! Module with testing utility functions.
#![allow(clippy::unwrap_in_result, clippy::unwrap_used, clippy::expect_used)]

use crate::config::CONFIG_PREFIX;
use crate::config::Settings;
use crate::config::error::SpoxConfigError;

/// The path for the configuration file that we should use during testing.
pub const DEFAULT_CONFIG_PATH: Option<&str> = Some("./src/config/default.toml");

impl Settings {
    /// Create a new `Settings` instance from the default configuration file.
    /// This is useful for testing.
    pub fn new_from_default_config() -> Result<Self, SpoxConfigError> {
        Self::new(DEFAULT_CONFIG_PATH)
    }
}

/// Clears all application-specific configuration environment variables. This is
/// needed for a number of tests which use the `Settings` struct due to the fact
/// that `cargo test` runs tests in threads, and environment variables are
/// per-process.
///
/// If we switched to `cargo nextest` (which runs tests in separate processes),
/// this would no longer be needed.
pub fn clear_env() {
    let mut prefix = CONFIG_PREFIX.to_owned();
    prefix.push('_');
    for var in std::env::vars() {
        if var.0.starts_with(&prefix) {
            unsafe {
                std::env::remove_var(var.0);
            }
        }
    }
}

/// A wrapper for setting environment variables in tests
pub fn set_var<K: AsRef<std::ffi::OsStr>, V: AsRef<std::ffi::OsStr>>(key: K, value: V) {
    unsafe {
        std::env::set_var(key, value);
    }
}
