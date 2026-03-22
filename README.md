# emusync

Sync emulator saves, configs, mods, and shader caches across machines over SSH.

## What It Does

If you play emulated games across multiple machines, emusync keeps everything in sync. Define your machines and what to sync in a single config file, and emusync handles bidirectional sync over SSH using rsync. Newest file always wins.

Two target types:

- **`directory`** — bidirectional rsync for any emulator's data. Works for saves, configs, cheats, patches — anything stored in a directory.
- **`ryujinx`** — title-ID-aware sync for Ryujinx (Nintendo Switch). Handles saves, mods, and portable shader caches. Ryujinx assigns save folder numbers by first-launch order, so the same game gets different folder numbers on different machines. emusync maps saves by game identity instead of folder number, preventing cross-save corruption.

## Requirements

- Rust (for building)
- `rsync` on both machines
- SSH access between machines
- `xxd` on the remote machine (only needed for Ryujinx targets)

## Installation

```bash
git clone https://github.com/guitaripod/emusync
cd emusync
cargo install --path .
```

## Configuration

```bash
emusync init
```

Edit `~/.config/emusync/config.json`:

```json
{
  "machines": [
    { "name": "desktop", "ssh_target": "user@192.168.1.10" },
    { "name": "laptop", "ssh_target": "user@192.168.1.20" }
  ],
  "targets": [
    {
      "name": "duckstation",
      "type": "directory",
      "paths": {
        "desktop": "/home/user/Emulation/saves/duckstation",
        "laptop": "/Users/user/Emulation/saves/duckstation"
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

## Usage

```bash
emusync                              # sync all targets
emusync sync duckstation             # sync a specific target
emusync sync ryujinx                 # sync all Ryujinx data
emusync sync ryujinx --only saves    # Ryujinx saves only
emusync sync ryujinx --only mods     # mods only
emusync sync ryujinx --only shaders  # portable shader caches only
emusync status                       # show sync state
emusync --dry-run                    # preview without changes
emusync --push                       # force local → remote
emusync --pull                       # force remote → local
emusync --json                       # structured JSON output
```

## How Ryujinx Sync Works

Ryujinx assigns save folder numbers sequentially by first-launch order. If you launch Zelda first on one machine it gets folder `0000000000000001`, but on another machine it might be `0000000000000003`. Syncing folders directly would overwrite the wrong game's saves.

emusync parses the binary `ExtraData0` in each save folder to extract the title ID, builds a mapping on both machines, and syncs by game identity.

Three subtargets:
- **saves** — title-ID-mapped save sync across different folder numbers
- **mods** — syncs `mods/contents/` (organized by title ID)
- **shaders** — syncs portable shader caches (`guest.*`, `shared.*`), skips GPU-vendor-specific compiled shaders since those are rebuilt from portable caches on first launch

## Multi-Machine Setup

emusync configs are per-pair. For three or more machines, use a hub model: designate one always-on machine as the hub and put it first in the machines array. Each other machine syncs with the hub, keeping everything current across all machines.
