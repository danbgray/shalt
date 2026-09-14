use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Org {
    pub name: String,
    #[serde(default)]
    pub projects: Vec<ProjectRef>,
}

impl Org {
    pub fn home_dir() -> PathBuf {
        if let Ok(h) = std::env::var("SHALT_HOME") {
            return PathBuf::from(h);
        }
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".shalt")
    }

    pub fn path() -> PathBuf {
        Self::home_dir().join("org.toml")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        if !path.exists() {
            return Self {
                name: "local".into(),
                projects: vec![],
            };
        }
        toml::from_str(&fs::read_to_string(path).unwrap_or_default()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(path, toml::to_string_pretty(self).unwrap_or_default())
    }

    pub fn add(&mut self, path: &Path) -> Result<ProjectRef, String> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let id = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into());
        if self.projects.iter().any(|p| p.id == id || p.path == path.display().to_string()) {
            return Err(format!("project {id} already in org"));
        }
        let pref = ProjectRef {
            id: id.clone(),
            name: id.clone(),
            path: path.display().to_string(),
        };
        self.projects.push(pref.clone());
        Ok(pref)
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let n = self.projects.len();
        self.projects.retain(|p| p.id != id);
        self.projects.len() != n
    }

    pub fn get(&self, id: &str) -> Option<&ProjectRef> {
        self.projects.iter().find(|p| p.id == id)
    }
}
