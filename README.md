# emusync

Cross-machine emulation save, mod, and shader cache sync over SSH.

## The Problem

If you play emulated games across multiple machines, keeping saves in sync is painful. Most emulators store saves in straightforward directories that can be rsynced, but **Ryujinx** (Nintendo Switch emulator) has a particularly nasty issue:

Ryujinx assigns save folder numbers sequentially by first-launch order. If you launch Zelda: TOTK first on Machine A, it gets folder `0000000000000001`. But if you launched Mario Kart first on Machine B, TOTK might be `0000000000000003`. **Naive folder sync between machines will overwrite the wrong game's saves.**

emusync solves this by parsing Ryujinx's binary `ExtraData0` files to build a title-ID-to-folder mapping on each machine, then syncing save data by game identity rather than folder number.

## Features

- **Config-driven sync targets** — define any directory pairs to sync between machines
- **Bidirectional sync** — newest file wins, powered by `rsync --update`
- **Ryujinx-aware sync** — title-ID-based save mapping, mod sync, portable shader cache sync
- **JSON output** — `--json` flag for structured output, designed for CLI AI agents
- **Dry run** — preview all operations before executing
- **Cross-platform** — works on Linux and macOS

## Requirements

- Rust (for building)
- `rsync` on both machines
- SSH access between machines (Tailscale recommended)
- `xxd` on the remote machine (for Ryujinx save mapping)

## Installation

```bash
git clone https://github.com/guitaripod/emusync
cd emusync
cargo install --path .
```

## Configuration

Generate a config template:

```bash
emusync init
```

This creates `~/.config/emusync/config.json`. Edit it to define your machines and sync targets:

```json
{
  "machines": [
    { "name": "desktop", "ssh_target": "user@desktop-ip" },
    { "name": "laptop", "ssh_target": "user@laptop-ip" }
  ],
  "targets": [
    {
      "name": "retroarch",
      "type": "directory",
      "paths": {
        "desktop": "/path/to/retroarch/saves",
        "laptop": "/path/to/retroarch/saves"
      }
    },
    {
      "name": "ryujinx",
      "type": "ryujinx",
      "paths": {
        "desktop": "/home/user/.config/Ryujinx",
        "laptop": "/Users/user/Library/Application Support/Ryujinx"
      }
    }
  ],
  "exclude": [".DS_Store", "Thumbs.db", ".lock"]
}
```

emusync auto-detects which machine it's running on by checking which configured paths exist locally.

### Target Types

**`directory`** — Bidirectional rsync with `--update` (newest file wins). Works for any emulator saves directory.

**`ryujinx`** — Title-ID-aware sync with three subtargets:
- **saves** — Maps title IDs to folder numbers on each machine, syncs by game identity
- **mods** — Syncs `mods/contents/` (already organized by title ID)
- **shaders** — Syncs portable shader cache files (`guest.*`, `shared.*`), skips GPU-vendor-specific files (`vulkan_nvidia.*`, `vulkan_apple.*`)

## Usage

```bash
emusync                              # sync all targets
emusync sync                         # same as above
emusync sync retroarch               # sync specific target
emusync sync ryujinx                 # sync all Ryujinx data
emusync sync ryujinx --only saves    # Ryujinx saves only
emusync sync ryujinx --only mods     # mods only
emusync sync ryujinx --only shaders  # portable shader caches only
emusync status                       # show sync state
emusync --dry-run                    # preview without changes
emusync --push                       # force local → remote
emusync --pull                       # force remote → local
emusync --json                       # structured JSON output
emusync status --json                # machine-readable status
```

## How It Works

### Directory Targets

Simple bidirectional rsync: push local changes with `--update`, then pull remote changes with `--update`. Newest file always wins.

### Ryujinx Saves

1. Parses `ExtraData0` (binary file in each save folder) to extract the Nintendo Switch title ID
2. Builds a `{title_id → folder_number}` mapping on both machines
3. Filters to user saves only (save type 1), skipping device saves (type 3)
4. Compares modification times per title ID
5. Syncs the newer save to the older machine, using the correct folder numbers on each side

### Ryujinx Shaders

Shader caches contain both portable data (`guest.*`, `shared.*`) and GPU-vendor-specific compiled shaders (`vulkan_nvidia.*`, `vulkan_apple.*`). emusync only syncs the portable files — the vendor-specific shaders are automatically rebuilt from them on first launch.
