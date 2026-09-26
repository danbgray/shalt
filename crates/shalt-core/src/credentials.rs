//! Hosted Space identity. The token is never printed.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_HOST: &str = "https://shalt.dev";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Credentials {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub token: String,
}

fn default_host() -> String {
    DEFAULT_HOST.into()
}

impl Credentials {
    pub fn path() -> PathBuf {
        crate::org::Org::home_dir().join("credentials.toml")
    }

    pub fn load() -> Self {
        let p = Self::path();
        if !p.exists() {
            return Self {
                host: DEFAULT_HOST.into(),
                ..Default::default()
            };
        }
        toml::from_str(&fs::read_to_string(p).unwrap_or_default()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(p, toml::to_string_pretty(self).unwrap_or_default())
    }

    pub fn clear() -> std::io::Result<()> {
        let p = Self::path();
        if p.exists() {
            fs::remove_file(p)?;
        }
        Ok(())
    }

    pub fn signed_in(&self) -> bool {
        !self.token.trim().is_empty()
    }
}
