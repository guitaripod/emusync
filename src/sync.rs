use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    Auto,
    Push,
    Pull,
}

pub struct SyncResult {
    pub transferred: bool,
    pub direction: &'static str,
}

pub fn rsync(
    src: &str,
    dst: &str,
    extra_args: &[&str],
    excludes: &[String],
    dry_run: bool,
) -> Result<bool> {
    let mut cmd = Command::new("rsync");
    cmd.args(["-rvltDP", "--no-perms", "--no-owner", "--no-group"]);

    if dry_run {
        cmd.arg("-n");
    }

    for ex in excludes {
        cmd.arg(format!("--exclude={ex}"));
    }

    for arg in extra_args {
        cmd.arg(arg);
    }

    cmd.arg(src);
    cmd.arg(dst);

    let output = cmd.output().context("failed to run rsync")?;

    match output.status.code() {
        Some(0) => {}
        Some(23) | Some(24) => {
            eprintln!(
                "  rsync partial transfer: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Some(code) => {
            bail!(
                "rsync failed (exit {}): {}",
                code,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        None => bail!("rsync terminated by signal"),
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let transferred = stdout.lines().any(|l| {
        let trimmed = l.trim();
        !trimmed.is_empty()
            && !trimmed.starts_with("sending")
            && !trimmed.starts_with("sent ")
            && !trimmed.starts_with("total ")
            && !trimmed.starts_with("receiving")
            && !trimmed.contains("bytes/sec")
            && !trimmed.starts_with("building file list")
            && !trimmed.starts_with("./")
            && !trimmed.contains("files to consider")
            && !trimmed.contains("files...")
            && !trimmed.starts_with("created directory")
            && !trimmed.ends_with("(DRY RUN)")
            && !trimmed.starts_with("Transfer starting")
    });

    Ok(transferred)
}

pub fn rsync_bidirectional(
    local_path: &str,
    ssh_target: &str,
    remote_path: &str,
    excludes: &[String],
    direction: Direction,
    dry_run: bool,
) -> Result<SyncResult> {
    let remote = format!("{ssh_target}:{remote_path}");
    let local_trailing = ensure_trailing_slash(local_path);
    let remote_trailing = ensure_trailing_slash(&remote);

    match direction {
        Direction::Push => {
            let transferred =
                rsync(&local_trailing, &remote_trailing, &["--update"], excludes, dry_run)?;
            Ok(SyncResult {
                transferred,
                direction: "push",
            })
        }
        Direction::Pull => {
            let transferred =
                rsync(&remote_trailing, &local_trailing, &["--update"], excludes, dry_run)?;
            Ok(SyncResult {
                transferred,
                direction: "pull",
            })
        }
        Direction::Auto => {
            let pushed =
                rsync(&local_trailing, &remote_trailing, &["--update"], excludes, dry_run)?;
            let pulled =
                rsync(&remote_trailing, &local_trailing, &["--update"], excludes, dry_run)?;
            let dir = if pushed && pulled {
                "both"
            } else if pushed {
                "push"
            } else if pulled {
                "pull"
            } else {
                "none"
            };
            Ok(SyncResult {
                transferred: pushed || pulled,
                direction: dir,
            })
        }
    }
}

pub fn rsync_one_way(
    src: &str,
    dst: &str,
    excludes: &[String],
    extra_args: &[&str],
    dry_run: bool,
) -> Result<bool> {
    rsync(
        &ensure_trailing_slash(src),
        &ensure_trailing_slash(dst),
        extra_args,
        excludes,
        dry_run,
    )
}

pub fn ssh_output(target: &str, command: &str) -> Result<String> {
    let output = Command::new("ssh")
        .arg(target)
        .arg(command)
        .output()
        .context("failed to run ssh")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ssh command failed: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn newest_mtime_recursive(dir: &Path) -> Result<u64> {
    let mut newest: u64 = 0;
    walk_dir_mtime(dir, &mut newest)?;
    Ok(newest)
}

fn walk_dir_mtime(dir: &Path, newest: &mut u64) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            walk_dir_mtime(&entry.path(), newest)?;
        } else if ft.is_file() {
            let mtime = entry
                .metadata()?
                .modified()?
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if mtime > *newest {
                *newest = mtime;
            }
        }
    }
    Ok(())
}

pub fn count_files_local(dir: &Path) -> u64 {
    let mut count: u64 = 0;
    count_recursive(dir, &mut count);
    count
}

fn count_recursive(dir: &Path, count: &mut u64) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if let Ok(ft) = entry.file_type() {
            if ft.is_file() {
                *count += 1;
            } else if ft.is_dir() {
                count_recursive(&entry.path(), count);
            }
        }
    }
}

pub fn count_files_remote(ssh_target: &str, path: &str) -> Result<u64> {
    let output = ssh_output(ssh_target, &format!("find '{path}' -type f 2>/dev/null | wc -l"))?;
    Ok(output.trim().parse().unwrap_or(0))
}

fn ensure_trailing_slash(path: &str) -> String {
    if path.ends_with('/') {
        path.to_string()
    } else {
        format!("{path}/")
    }
}
