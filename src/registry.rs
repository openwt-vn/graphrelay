use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Registry {
    #[serde(default)]
    pub projects: Vec<ProjectEntry>,
}

impl Registry {
    pub fn global_dir() -> PathBuf {
        home_dir().join(".graphrelay")
    }

    pub fn pid_path() -> PathBuf {
        Self::global_dir().join("daemon.pid")
    }

    pub fn port_path() -> PathBuf {
        Self::global_dir().join("daemon.port")
    }

    pub fn log_path() -> PathBuf {
        Self::global_dir().join("daemon.log")
    }

    fn registry_path() -> PathBuf {
        Self::global_dir().join("projects.toml")
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::registry_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {:?}", path))?;
        toml::from_str(&text).context("parsing projects.toml")
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Self::global_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating {:?}", dir))?;
        let text = toml::to_string_pretty(self)?;
        std::fs::write(Self::registry_path(), text)
            .with_context(|| format!("writing {:?}", Self::registry_path()))
    }

    pub fn add(&mut self, name: String, root: PathBuf) {
        // Remove any existing entry with the same name or same root
        self.projects.retain(|p| p.name != name && p.root != root);
        self.projects.push(ProjectEntry { name, root });
    }

    pub fn remove_by_root(&mut self, root: &Path) {
        self.projects.retain(|p| p.root != root);
    }

    pub fn find_by_root(&self, root: &Path) -> Option<&ProjectEntry> {
        self.projects.iter().find(|p| p.root == root)
    }
}

pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Stable data-dir for a project, based on its absolute root path.
pub fn project_data_dir(root: &Path) -> PathBuf {
    let root_str = root.to_string_lossy();
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in root_str.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let hash = format!("{:016x}", h);
    home_dir().join(".graphrelay").join(&hash[..8])
}
