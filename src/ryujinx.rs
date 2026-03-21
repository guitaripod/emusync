use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::DetectedConfig;
use crate::sync::{Direction, newest_mtime_recursive, rsync_bidirectional, rsync_one_way, ssh_output};

pub struct SaveEntry {
    pub folder: String,
    pub mtime: u64,
}

fn parse_extra_data(bytes: &[u8]) -> Option<(String, u32)> {
    if bytes.len() < 0x24 {
        return None;
    }
    let title_id = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let save_type = u32::from_le_bytes(bytes[0x20..0x24].try_into().ok()?);
    if title_id == 0 {
        return None;
    }
    Some((format!("{:016X}", title_id), save_type))
}

fn build_local_save_map(ryujinx_path: &Path) -> Result<HashMap<String, SaveEntry>> {
    let save_dir = ryujinx_path.join("bis").join("user").join("save");
    let mut map = HashMap::new();

    let entries = match std::fs::read_dir(&save_dir) {
        Ok(e) => e,
        Err(_) => return Ok(map),
    };

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let folder_name = entry.file_name().to_string_lossy().to_string();
        let extra_data_path = entry.path().join("ExtraData0");

        if !extra_data_path.exists() {
            continue;
        }

        let bytes = std::fs::read(&extra_data_path)?;
        if let Some((title_id, save_type)) = parse_extra_data(&bytes) {
            if save_type != 1 {
                continue;
            }

            let mtime = newest_mtime_recursive(&entry.path())?;
            map.insert(
                title_id,
                SaveEntry {
                    folder: folder_name,
                    mtime,
                },
            );
        }
    }

    Ok(map)
}

fn build_remote_save_map(
    ssh_target: &str,
    ryujinx_path: &str,
) -> Result<HashMap<String, SaveEntry>> {
    let script = format!(
        r#"for d in '{ryujinx_path}/bis/user/save'/*/; do
  f="${{d}}ExtraData0"
  if [ -f "$f" ]; then
    tid=$(xxd -p -l 8 "$f")
    stype=$(xxd -p -s 32 -l 4 "$f")
    newest=$(find "$d" -type f -name '*.sav' -exec stat -c %Y {{}} + 2>/dev/null || find "$d" -type f -name '*.sav' -exec stat -f %m {{}} + 2>/dev/null | sort -rn | head -1)
    echo "$(basename "$d")|$tid|$stype|${{newest:-0}}"
  fi
done"#
    );

    let output = ssh_output(ssh_target, &script)?;
    let mut map = HashMap::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 4 {
            continue;
        }

        let folder = parts[0].to_string();
        let tid_hex = parts[1].trim();
        let stype_hex = parts[2].trim();
        let mtime_str = parts[3].trim();

        if stype_hex != "01000000" {
            continue;
        }

        let title_id = hex_to_title_id(tid_hex);
        let mtime: u64 = mtime_str.parse().unwrap_or(0);

        map.insert(title_id, SaveEntry { folder, mtime });
    }

    Ok(map)
}

fn hex_to_title_id(hex: &str) -> String {
    if hex.len() < 16 {
        return hex.to_uppercase();
    }
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect();

    if bytes.len() >= 8 {
        let val = u64::from_le_bytes(bytes[0..8].try_into().unwrap_or_default());
        format!("{:016X}", val)
    } else {
        hex.to_uppercase()
    }
}

pub fn title_name(id: &str) -> &str {
    match id {
        "0100152000022000" => "Mario Kart 8 Deluxe",
        "01001F5010DFA000" => "Pokemon Legends Arceus",
        "0100B3F000BE2000" => "Pokken Tournament DX",
        "010028600EBDA000" => "SM3D World + Bowser's Fury",
        "0100000000010000" => "Super Mario Odyssey",
        "01006A800016E000" => "Super Smash Bros Ultimate",
        "01007EF00011E000" => "Zelda: Breath of the Wild",
        "0100F2C0115B6000" => "Zelda: Tears of the Kingdom",
        _ => id,
    }
}

pub fn sync_saves(
    detected: &DetectedConfig,
    ryujinx_path_local: &str,
    ryujinx_path_remote: &str,
    direction: Direction,
    dry_run: bool,
    json: bool,
) -> Result<Vec<serde_json::Value>> {
    let local_map = build_local_save_map(Path::new(ryujinx_path_local))?;
    let remote_map = build_remote_save_map(&detected.remote.ssh_target, ryujinx_path_remote)?;

    let mut results = Vec::new();

    for (title_id, local_entry) in &local_map {
        let name = title_name(title_id);

        let remote_entry = match remote_map.get(title_id) {
            Some(e) => e,
            None => {
                if !json {
                    eprintln!("  {name} — only on local, skipping");
                }
                continue;
            }
        };

        let sync_dir = match direction {
            Direction::Push => Direction::Push,
            Direction::Pull => Direction::Pull,
            Direction::Auto => {
                if local_entry.mtime > remote_entry.mtime {
                    Direction::Push
                } else if remote_entry.mtime > local_entry.mtime {
                    Direction::Pull
                } else {
                    if !json {
                        eprintln!("  {name} — up to date");
                    }
                    continue;
                }
            }
        };

        let local_save = PathBuf::from(ryujinx_path_local)
            .join("bis/user/save")
            .join(&local_entry.folder);
        let remote_save = PathBuf::from(ryujinx_path_remote)
            .join("bis/user/save")
            .join(&remote_entry.folder);

        let (src, dst) = match sync_dir {
            Direction::Push => (
                local_save.to_string_lossy().to_string(),
                detected.remote_rsync_path(&remote_save.to_string_lossy()),
            ),
            Direction::Pull => (
                detected.remote_rsync_path(&remote_save.to_string_lossy()),
                local_save.to_string_lossy().to_string(),
            ),
            _ => unreachable!(),
        };

        let dir_label = if sync_dir == Direction::Push {
            "push"
        } else {
            "pull"
        };

        if !json {
            eprintln!(
                "  {name} — {dir_label} (local:{} → remote:{})",
                local_entry.folder, remote_entry.folder
            );
        }

        let excludes = vec![".lock".to_string()];
        rsync_one_way(&src, &dst, &excludes, &[], dry_run)?;

        results.push(serde_json::json!({
            "target": "ryujinx",
            "type": "ryujinx_save",
            "title_id": title_id,
            "name": name,
            "direction": dir_label,
            "local_folder": local_entry.folder,
            "remote_folder": remote_entry.folder,
        }));
    }

    for (title_id, _) in &remote_map {
        if !local_map.contains_key(title_id) {
            let name = title_name(title_id);
            if !json {
                eprintln!("  {name} — only on remote, skipping");
            }
        }
    }

    Ok(results)
}

pub fn sync_mods(
    detected: &DetectedConfig,
    ryujinx_path_local: &str,
    ryujinx_path_remote: &str,
    direction: Direction,
    dry_run: bool,
    json: bool,
) -> Result<Vec<serde_json::Value>> {
    let local_mods = format!("{ryujinx_path_local}/mods/contents");
    let remote_mods = format!("{ryujinx_path_remote}/mods/contents");

    if !Path::new(&local_mods).exists() {
        if !json {
            eprintln!("  mods — no local mods directory");
        }
        return Ok(vec![]);
    }

    if !json {
        eprintln!("  mods — syncing...");
    }

    let result = rsync_bidirectional(
        &local_mods,
        &detected.remote.ssh_target,
        &remote_mods,
        &detected.config.exclude,
        direction,
        dry_run,
    )?;

    if result.transferred {
        Ok(vec![serde_json::json!({
            "target": "ryujinx",
            "type": "ryujinx_mods",
            "direction": result.direction,
        })])
    } else {
        if !json {
            eprintln!("  mods — up to date");
        }
        Ok(vec![])
    }
}

pub fn sync_shaders(
    detected: &DetectedConfig,
    ryujinx_path_local: &str,
    ryujinx_path_remote: &str,
    direction: Direction,
    dry_run: bool,
    json: bool,
) -> Result<Vec<serde_json::Value>> {
    let local_games = format!("{ryujinx_path_local}/games");
    let remote_games = format!("{ryujinx_path_remote}/games");

    if !Path::new(&local_games).exists() {
        if !json {
            eprintln!("  shaders — no local games directory");
        }
        return Ok(vec![]);
    }

    if !json {
        eprintln!("  shaders — syncing portable caches...");
    }

    let extra_args = &[
        "--include=*/",
        "--include=guest.*",
        "--include=shared.*",
        "--exclude=*",
    ];

    let mut excludes = detected.config.exclude.clone();
    excludes.push("vulkan_nvidia.*".to_string());
    excludes.push("vulkan_apple.*".to_string());

    let remote_target = detected.remote_rsync_path(&remote_games);

    let transferred = match direction {
        Direction::Push => {
            crate::sync::rsync(
                &format!("{local_games}/"),
                &format!("{remote_target}/"),
                extra_args,
                &excludes,
                dry_run,
            )?
        }
        Direction::Pull => {
            crate::sync::rsync(
                &format!("{remote_target}/"),
                &format!("{local_games}/"),
                extra_args,
                &excludes,
                dry_run,
            )?
        }
        Direction::Auto => {
            let pushed = crate::sync::rsync(
                &format!("{local_games}/"),
                &format!("{remote_target}/"),
                &[extra_args.as_slice(), &["--update"]].concat::<&str>(),
                &excludes,
                dry_run,
            )?;
            let pulled = crate::sync::rsync(
                &format!("{remote_target}/"),
                &format!("{local_games}/"),
                &[extra_args.as_slice(), &["--update"]].concat::<&str>(),
                &excludes,
                dry_run,
            )?;
            pushed || pulled
        }
    };

    if transferred {
        Ok(vec![serde_json::json!({
            "target": "ryujinx",
            "type": "ryujinx_shaders",
            "direction": match direction {
                Direction::Push => "push",
                Direction::Pull => "pull",
                Direction::Auto => "auto",
            },
        })])
    } else {
        if !json {
            eprintln!("  shaders — up to date");
        }
        Ok(vec![])
    }
}

pub fn sync_all(
    detected: &DetectedConfig,
    target: &crate::config::Target,
    only: Option<&str>,
    direction: Direction,
    dry_run: bool,
    json: bool,
) -> Result<Vec<serde_json::Value>> {
    let local_path = match detected.local_path(target) {
        Some(p) => p.clone(),
        None => {
            if !json {
                eprintln!("  skipped (no local path configured)");
            }
            return Ok(vec![]);
        }
    };

    let remote_path = match detected.remote_path(target) {
        Some(p) => p.clone(),
        None => {
            if !json {
                eprintln!("  skipped (no remote path configured)");
            }
            return Ok(vec![]);
        }
    };

    let mut results = Vec::new();

    let sync_saves = only.is_none() || only == Some("saves");
    let sync_mods_flag = only.is_none() || only == Some("mods");
    let sync_shaders_flag = only.is_none() || only == Some("shaders");

    if sync_saves {
        if !json {
            eprintln!("  saves:");
        }
        results.extend(self::sync_saves(
            detected,
            &local_path,
            &remote_path,
            direction,
            dry_run,
            json,
        )?);
    }

    if sync_mods_flag {
        results.extend(self::sync_mods(
            detected,
            &local_path,
            &remote_path,
            direction,
            dry_run,
            json,
        )?);
    }

    if sync_shaders_flag {
        results.extend(self::sync_shaders(
            detected,
            &local_path,
            &remote_path,
            direction,
            dry_run,
            json,
        )?);
    }

    Ok(results)
}

pub fn status(
    detected: &DetectedConfig,
    target: &crate::config::Target,
) -> Result<serde_json::Value> {
    let local_path = match detected.local_path(target) {
        Some(p) => p.clone(),
        None => return Ok(serde_json::json!({"error": "no local path"})),
    };

    let remote_path = match detected.remote_path(target) {
        Some(p) => p.clone(),
        None => return Ok(serde_json::json!({"error": "no remote path"})),
    };

    let local_map = build_local_save_map(Path::new(&local_path))?;
    let remote_map =
        build_remote_save_map(&detected.remote.ssh_target, &remote_path)?;

    let mut saves = serde_json::Map::new();

    for (title_id, local_entry) in &local_map {
        let name = title_name(title_id);
        if let Some(remote_entry) = remote_map.get(title_id) {
            let dir = if local_entry.mtime > remote_entry.mtime {
                "push"
            } else if remote_entry.mtime > local_entry.mtime {
                "pull"
            } else {
                "synced"
            };
            saves.insert(
                title_id.clone(),
                serde_json::json!({
                    "name": name,
                    "local_folder": local_entry.folder,
                    "remote_folder": remote_entry.folder,
                    "local_mtime": local_entry.mtime,
                    "remote_mtime": remote_entry.mtime,
                    "direction": dir,
                }),
            );
        } else {
            saves.insert(
                title_id.clone(),
                serde_json::json!({
                    "name": name,
                    "local_folder": local_entry.folder,
                    "status": "local_only",
                }),
            );
        }
    }

    for (title_id, remote_entry) in &remote_map {
        if !local_map.contains_key(title_id) {
            let name = title_name(title_id);
            saves.insert(
                title_id.clone(),
                serde_json::json!({
                    "name": name,
                    "remote_folder": remote_entry.folder,
                    "status": "remote_only",
                }),
            );
        }
    }

    Ok(serde_json::json!({
        "type": "ryujinx",
        "saves": saves,
    }))
}
