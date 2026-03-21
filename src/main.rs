mod config;
mod directory;
mod ryujinx;
mod sync;

use std::io::IsTerminal;

use anyhow::Result;
use clap::{Parser, Subcommand};

use config::DetectedConfig;
use sync::Direction;

fn use_color() -> bool {
    std::io::stderr().is_terminal() && std::env::var("NO_COLOR").is_err()
}

fn green() -> &'static str {
    if use_color() { "\x1b[32m" } else { "" }
}
fn yellow() -> &'static str {
    if use_color() { "\x1b[33m" } else { "" }
}
fn bold() -> &'static str {
    if use_color() { "\x1b[1m" } else { "" }
}
fn reset() -> &'static str {
    if use_color() { "\x1b[0m" } else { "" }
}

#[derive(Parser)]
#[command(
    name = "emusync",
    version,
    about = "Cross-machine emulation sync over SSH",
    long_about = "Sync emulator saves, mods, and shader caches between machines over SSH.\n\n\
        Supports two target types:\n  \
        - \"directory\": bidirectional rsync (newest file wins)\n  \
        - \"ryujinx\": title-ID-aware save/mod/shader sync\n\n\
        Ryujinx saves use numbered folders that differ per machine. emusync parses\n\
        the binary ExtraData0 to map title IDs, syncing by game identity.\n\n\
        Config: ~/.config/emusync/config.json\n\
        Run `emusync init` to generate a template.",
    after_help = "Examples:\n  \
        emusync                              Sync all targets\n  \
        emusync sync duckstation             Sync a specific target\n  \
        emusync sync ryujinx                 Sync all Ryujinx data\n  \
        emusync sync ryujinx --only saves    Sync only Ryujinx saves\n  \
        emusync sync ryujinx --only mods     Sync only Ryujinx mods\n  \
        emusync sync ryujinx --only shaders  Sync only portable shader caches\n  \
        emusync sync --push                  Force local -> remote\n  \
        emusync sync --pull                  Force remote -> local\n  \
        emusync status                       Show sync state for all targets\n  \
        emusync status --json                Machine-readable sync state\n  \
        emusync --dry-run                    Preview all sync operations\n  \
        emusync --json                       Sync all, output JSON result"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long, global = true, help = "Preview sync operations without making changes")]
    dry_run: bool,

    #[arg(long, global = true, conflicts_with = "pull", help = "Force sync direction: local -> remote")]
    push: bool,

    #[arg(long, global = true, conflicts_with = "push", help = "Force sync direction: remote -> local")]
    pull: bool,

    #[arg(long, global = true, help = "Output structured JSON to stdout (for AI agents and scripts)")]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
    #[command(
        about = "Sync one or all targets (default when no subcommand given)",
        long_about = "Sync one or all configured targets between local and remote machines.\n\n\
            Without a target name, syncs everything. With a target name, syncs only that target.\n\
            For \"ryujinx\" targets, use --only to sync a specific subtarget.",
        after_help = "Examples:\n  \
            emusync sync                         Sync all targets\n  \
            emusync sync duckstation             Sync one directory target\n  \
            emusync sync ryujinx                 Sync all Ryujinx data\n  \
            emusync sync ryujinx --only saves    Sync only Ryujinx saves\n  \
            emusync sync ryujinx --only mods     Sync only Ryujinx mods\n  \
            emusync sync ryujinx --only shaders  Sync only portable shader caches\n  \
            emusync sync --push                  Force local -> remote\n  \
            emusync sync --dry-run               Preview without changes"
    )]
    Sync {
        #[arg(help = "Target name from config (omit to sync all targets)")]
        target: Option<String>,
        #[arg(long, help = "For ryujinx targets: sync only a subtarget [saves|mods|shaders]")]
        only: Option<String>,
    },
    #[command(
        about = "Show sync status for all targets",
        long_about = "Show sync status for all configured targets.\n\n\
            For directory targets: compares file counts between local and remote.\n\
            For ryujinx targets: shows per-game save mapping with title IDs,\n\
            folder numbers on each machine, and sync direction (push/pull/synced).\n\n\
            Use --json for machine-readable output."
    )]
    Status,
    #[command(
        about = "Generate a config template at ~/.config/emusync/config.json",
        long_about = "Generate a starter config at ~/.config/emusync/config.json.\n\n\
            The template includes placeholder machine names and paths.\n\
            Edit it with your actual SSH targets and emulator data paths,\n\
            then run `emusync status` to verify detection works."
    )]
    Init,
}

fn direction_from_flags(push: bool, pull: bool) -> Direction {
    if push {
        Direction::Push
    } else if pull {
        Direction::Pull
    } else {
        Direction::Auto
    }
}

fn run_sync(
    detected: &DetectedConfig,
    target_name: Option<&str>,
    only: Option<&str>,
    direction: Direction,
    dry_run: bool,
    json: bool,
) -> Result<Vec<serde_json::Value>> {
    let mut all_results = Vec::new();

    let targets: Vec<&config::Target> = if let Some(name) = target_name {
        let t = detected
            .config
            .targets
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| anyhow::anyhow!("unknown target: {name}"))?;
        vec![t]
    } else {
        detected.config.targets.iter().collect()
    };

    for target in targets {
        if !json {
            eprintln!("{}[{}]{} ({})", bold(), target.name, reset(), target.target_type);
        }

        let results = match target.target_type.as_str() {
            "directory" => {
                if only.is_some() {
                    if !json {
                        eprintln!("  --only is not supported for directory targets");
                    }
                    vec![]
                } else {
                    let r = directory::sync(detected, target, direction, dry_run, json)?;
                    r.into_iter().collect()
                }
            }
            "ryujinx" => {
                ryujinx::sync_all(detected, target, only, direction, dry_run, json)?
            }
            other => {
                if !json {
                    eprintln!("  unknown target type: {other}");
                }
                vec![]
            }
        };

        all_results.extend(results);
    }

    Ok(all_results)
}

fn run_status(detected: &DetectedConfig, json: bool) -> Result<serde_json::Value> {
    let mut targets_status = serde_json::Map::new();

    for target in &detected.config.targets {
        if !json {
            eprintln!("{}[{}]{} ({})", bold(), target.name, reset(), target.target_type);
        }

        match target.target_type.as_str() {
            "directory" => {
                let local_path = detected.local_path(target);
                let remote_path = detected.remote_path(target);
                let local_exists = local_path
                    .map(|p| std::path::Path::new(p).exists())
                    .unwrap_or(false);

                let local_files = if local_exists {
                    local_path
                        .map(|p| sync::count_files_local(std::path::Path::new(p)))
                        .unwrap_or(0)
                } else {
                    0
                };

                let remote_files = remote_path
                    .and_then(|p| {
                        sync::count_files_remote(&detected.remote.ssh_target, p).ok()
                    })
                    .unwrap_or(0);

                let in_sync = local_exists && local_files == remote_files;

                if !json {
                    let status_str = if !local_exists {
                        "missing".to_string()
                    } else if in_sync {
                        format!("{}synced{} ({} files)", green(), reset(), local_files)
                    } else {
                        format!(
                            "{}out of sync{} (local: {}, remote: {})",
                            yellow(), reset(), local_files, remote_files
                        )
                    };
                    eprintln!("  {status_str}");
                }

                targets_status.insert(
                    target.name.clone(),
                    serde_json::json!({
                        "type": "directory",
                        "local_exists": local_exists,
                        "local_files": local_files,
                        "remote_files": remote_files,
                        "in_sync": in_sync,
                    }),
                );
            }
            "ryujinx" => {
                let status = ryujinx::status(detected, target)?;

                if !json {
                    if let Some(saves) = status.get("saves").and_then(|s| s.as_object()) {
                        for (tid, info) in saves {
                            let name = info
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or(tid);
                            let dir = info
                                .get("direction")
                                .or(info.get("status"))
                                .and_then(|d| d.as_str())
                                .unwrap_or("unknown");

                            let color = match dir {
                                "synced" => green(),
                                "push" | "pull" => yellow(),
                                _ => "",
                            };
                            eprintln!("  {name}: {color}{dir}{}", reset());
                        }
                    }
                }

                targets_status.insert(target.name.clone(), status);
            }
            _ => {}
        }
    }

    Ok(serde_json::json!({
        "local": detected.local.name,
        "remote": detected.remote.name,
        "targets": targets_status,
    }))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let direction = direction_from_flags(cli.push, cli.pull);

    let command = cli.command.unwrap_or(Commands::Sync {
        target: None,
        only: None,
    });

    match command {
        Commands::Init => {
            config::generate_default_config()?;
        }
        Commands::Status => {
            let config = config::Config::load()?;
            let detected = config.detect()?;

            if !cli.json {
                eprintln!(
                    "{}emusync{} — local: {}{}{}, remote: {}",
                    bold(), reset(), green(), detected.local.name, reset(), detected.remote.name
                );
                eprintln!();
            }

            let status = run_status(&detected, cli.json)?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            }
        }
        Commands::Sync { target, only } => {
            let config = config::Config::load()?;
            let detected = config.detect()?;

            if !cli.json {
                let mode = if cli.dry_run { " (dry run)" } else { "" };
                eprintln!(
                    "{}emusync{} — local: {}{}{}, remote: {}{mode}",
                    bold(), reset(), green(), detected.local.name, reset(), detected.remote.name
                );
                eprintln!();
            }

            let results = run_sync(
                &detected,
                target.as_deref(),
                only.as_deref(),
                direction,
                cli.dry_run,
                cli.json,
            )?;

            if cli.json {
                let output = serde_json::json!({
                    "synced": results,
                    "dry_run": cli.dry_run,
                    "exit_code": 0,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                eprintln!();
                if results.is_empty() {
                    eprintln!("{}everything up to date{}", green(), reset());
                } else {
                    eprintln!(
                        "{}synced {} item(s){}",
                        green(), results.len(), reset()
                    );
                }
            }
        }
    }

    Ok(())
}
