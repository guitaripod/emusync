use anyhow::Result;
use std::path::Path;

use crate::config::DetectedConfig;
use crate::sync::{Direction, rsync_bidirectional};

pub fn sync(
    detected: &DetectedConfig,
    target: &crate::config::Target,
    direction: Direction,
    dry_run: bool,
    json: bool,
) -> Result<Option<serde_json::Value>> {
    let local_path = match detected.local_path(target) {
        Some(p) => p,
        None => {
            if !json {
                eprintln!("  skipped (no local path configured)");
            }
            return Ok(None);
        }
    };

    let remote_path = match detected.remote_path(target) {
        Some(p) => p,
        None => {
            if !json {
                eprintln!("  skipped (no remote path configured)");
            }
            return Ok(None);
        }
    };

    if !Path::new(local_path).exists() {
        if !json {
            eprintln!("  skipped (local path does not exist)");
        }
        return Ok(None);
    }

    let result = rsync_bidirectional(
        local_path,
        &detected.remote.ssh_target,
        remote_path,
        &detected.config.exclude,
        direction,
        dry_run,
    )?;

    if !json {
        if result.transferred {
            eprintln!("  synced ({})", result.direction);
        } else {
            eprintln!("  up to date");
        }
    }

    if result.transferred {
        Ok(Some(serde_json::json!({
            "target": target.name,
            "type": "directory",
            "direction": result.direction,
        })))
    } else {
        Ok(None)
    }
}
