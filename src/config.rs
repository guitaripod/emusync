use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub machines: Vec<Machine>,
    pub targets: Vec<Target>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Machine {
    pub name: String,
    pub ssh_target: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Target {
    pub name: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub paths: HashMap<String, String>,
}

pub struct DetectedConfig<'a> {
    pub local: &'a Machine,
    pub remote: &'a Machine,
    pub config: &'a Config,
}

impl<'a> DetectedConfig<'a> {
    pub fn local_path<'b>(&self, target: &'b Target) -> Option<&'b String> {
        target.paths.get(&self.local.name)
    }

    pub fn remote_path<'b>(&self, target: &'b Target) -> Option<&'b String> {
        target.paths.get(&self.remote.name)
    }

    pub fn remote_rsync_path(&self, path: &str) -> String {
        format!("{}:{path}", self.remote.ssh_target)
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("config not found at {}\nrun `emusync init` to create one", path.display()))?;
        serde_json::from_str(&content).context("invalid config format")
    }

    pub fn detect(&self) -> Result<DetectedConfig<'_>> {
        for (i, machine) in self.machines.iter().enumerate() {
            let has_local_path = self.targets.iter().any(|t| {
                t.paths
                    .get(&machine.name)
                    .map(|p| Path::new(p).exists())
                    .unwrap_or(false)
            });

            if has_local_path {
                let remote_idx = if i == 0 { 1 } else { 0 };
                if remote_idx >= self.machines.len() {
                    bail!("need at least 2 machines in config");
                }
                return Ok(DetectedConfig {
                    local: &self.machines[i],
                    remote: &self.machines[remote_idx],
                    config: self,
                });
            }
        }

        bail!(
            "could not detect local machine — none of the configured paths exist on this system"
        );
    }
}

pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home)
        .join(".config")
        .join("emusync")
        .join("config.json")
}

pub fn generate_default_config() -> Result<()> {
    let path = config_path();

    if path.exists() {
        bail!("config already exists at {}", path.display());
    }

    let config = Config {
        machines: vec![
            Machine {
                name: "machine-1".to_string(),
                ssh_target: "user@hostname-or-ip".to_string(),
            },
            Machine {
                name: "machine-2".to_string(),
                ssh_target: "user@hostname-or-ip".to_string(),
            },
        ],
        targets: vec![Target {
            name: "example-saves".to_string(),
            target_type: "directory".to_string(),
            paths: HashMap::from([
                ("machine-1".to_string(), "/path/to/saves".to_string()),
                ("machine-2".to_string(), "/path/to/saves".to_string()),
            ]),
        }],
        exclude: vec![
            ".DS_Store".to_string(),
            "Thumbs.db".to_string(),
            ".lock".to_string(),
        ],
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(&path, json)?;

    eprintln!("config created at {}", path.display());
    eprintln!();
    eprintln!("target types:");
    eprintln!("  \"directory\" — bidirectional rsync (newest file wins)");
    eprintln!("  \"ryujinx\"  — title-ID-aware save/mod/shader sync");
    eprintln!();

    if cfg!(target_os = "macos") {
        eprintln!("macOS hints:");
        eprintln!("  Ryujinx data: ~/Library/Application Support/Ryujinx");
    } else {
        eprintln!("Linux hints:");
        eprintln!("  Ryujinx data: ~/.config/Ryujinx");
    }

    eprintln!();
    eprintln!("edit the config, then run `emusync status` to verify");

    Ok(())
}
