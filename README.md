# ⚡ StreamZip (`strzip`)

> **Extract 150 GB+ (game) archives with limited disk space — zero double-space requirement.**  
> *Streams archives into memory, extracts files safely, and reclaims consumed volume space on-the-fly.*

---

## 🎮 The Problem

Usually, downloading and extracting a **100 GB – 150 GB game archive** requires at least **200 GB – 300 GB of free space**. For gamers with limited SSD storage, this double-space bottleneck forces agonizing file deletion or expensive storage upgrades.

## 🚀 The Solution

**StreamZip** treats archives as real-time streams. As multi-volume archive parts (`.part1.rar`, `.z01`, `.zip.001`) complete extraction, StreamZip **safely deletes consumed volume parts immediately**, keeping your disk footprint minimal throughout the process.

```text
  Traditional Extraction:   [ Archive (150GB) ] + [ Extracted Game (150GB) ] = 300 GB Free Space Needed ❌
  StreamZip Extraction:     [ Archive Parts ] ──(Extract & Reclaim)──> [ Game ] = ~155 GB Free Space Needed ✅
```

---

## ✨ Features

- ⚡ **Zero-Copy Streaming Decompression**: Pure Rust engine powered by `zip`, `flate2`, and `unrar`.
- 📊 **Real-Time TUI Dashboard**: Modern `ratatui` terminal UI featuring live progress, write speeds (MB/s), ETA calculations, and disk space reclaimed metrics.
- 🛡️ **Full Data Protection**:
  - **Pre-Flight Payload Verification (`--verify-first`)**: Scans & verifies file payload CRC32 checksums before touching disk.
  - **Zip-Slip Protection**: Prevents path traversal security vulnerabilities.
  - **Non-Destructive Truncation**: Only reclaims source volume bytes *after* extracted output is verified and flushed.
- 🔄 **Journaled Crash Recovery**: JSON-backed state journal (`.streamzip_journal_*.json`) tracks progress. If interrupted, run `strzip` again to resume right where you left off.
- 🚀 **Max Speed Mode (`--no-log`)**: Bypasses per-file log formatting overhead to maximize NVMe/SSD write throughput on archives with 50,000+ tiny texture/shader files.
- 💻 **One-Click PATH Installer**: Includes a double-clickable `install_to_path.bat` script for instant Windows command prompt setup.

---

## 🖥️ Terminal UI Preview

```text
 ┌─ Archive Info ────────────────────────────────────────────────────────────────────────┐
 │ ⚡ StreamZip v0.1.0  │  Archive: Cyberpunk2077.part1.rar  │  Mode: RAR                │
 └───────────────────────────────────────────────────────────────────────────────────────┘
 ┌─ Extraction Progress (ETA: 08:42) ────────────────────────────────────────────────────┐
 │ [####################--------------------] 52% (41,240.50 MB / 78,500.00 MB @ 82.4 MB/s)│
 └───────────────────────────────────────────────────────────────────────────────────────┘
 ┌─ Live Stats ──────────────────────────────────────────────────────────────────────────┐
 │ Status: Extracting...    │ Files: 12,410 │ Speed: 82.4 MB/s │ Reclaimed: 40,960.00 MB │
 └───────────────────────────────────────────────────────────────────────────────────────┘
 ┌─ Live Extraction Logs ────────────────────────────────────────────────────────────────┐
 │  • extracted: engine/shaders/cache/pbr_shader_042.bin                                 │
 │  • extracted: engine/textures/environment/city_wall_d.dds                             │
 └───────────────────────────────────────────────────────────────────────────────────────┘
 [Ctrl+C / Q] Safe Stop & Save Journal
```

---

## 📦 Installation

### Option 1: Pre-Compiled Binary (For End Users)

1. Download **`strzip-v0.1.0-windows-x64.zip`** from [GitHub Releases](https://github.com/v-lenn/StreamZip/releases).
2. Unzip it anywhere (e.g. `C:\Tools\StreamZip\`).
3. Double-click **`install_to_path.bat`** to automatically add `strzip` to your Windows User `PATH`.
4. Open a **new** Command Prompt or PowerShell window and run `strzip` from anywhere!

### Option 2: Build from Source (For Developers)

```cmd
git clone https://github.com/v-lenn/StreamZip.git
cd StreamZip
cargo install --path .
```

---

## 💡 Usage & Command Line Options

```cmd
# Standard extraction with auto-deletion of completed volume parts
strzip "D:\Downloads\Game.part1.rar"

# Recommended for 100 GB+ Games (Pre-flight integrity check + max speed mode)
strzip "D:\Downloads\Game.part1.rar" --verify-first --no-log

# Extract with password
strzip "D:\Downloads\ProtectedArchive.zip" -p "SecretPassword123"

# Custom output folder
strzip "archive.zip" -o "E:\Games\MyExtractedGame"

# Keep original source files (normal extraction without deletion)
strzip "archive.zip" --keep
```

### Options Reference

| Flag | Short | Description |
|---|---|---|
| `<ARCHIVE>` | | Path to main archive or first volume (`.part1.rar`, `.z01`, `.zip.001`, `.zip`) |
| `--verify-first` | | Read & verify all file payload CRC32 checksums before extracting |
| `--no-log` | | Disable per-file log viewport rendering for maximum SSD extraction throughput |
| `-o, --output` | `-o` | Custom output directory path |
| `-p, --password` | `-p` | Archive decryption password |
| `-k, --keep` | `-k` | Do not delete source volume parts after extraction |
| `--clean` | | Force-clear previous interrupted session journal and restart from scratch |
| `--keep-journal` | | Retain JSON session journal file after successful extraction |
| `--no-tui` | | Disable interactive Ratatui dashboard and use plain terminal text output |

---

## 🛠️ Architecture Overview

StreamZip uses a multi-threaded async worker pipeline:

```text
 ┌──────────────────────┐      ┌─────────────────────────┐      ┌─────────────────────┐
 │ Extraction Thread    │ ───> │ OS Disk Writes          │ ───> │ fsync / sync_data() │
 │ (Decompress Payload) │      │ (64 KB Aligned Buffers) │      └─────────────────────┘
 └──────────┬───────────┘      └─────────────────────────┘                 │
            │                                                              ▼
            │ (mpsc channel)                                       ┌─────────────────────┐
            └────────────────────────────────────────────────────> │ Background Journal  │
                                                                   │ Worker Thread (2s)  │
                                                                   └─────────────────────┘
```

1. **Extraction Thread**: Decompresses archive contents in 64 KB streaming buffers directly to disk.
2. **Data Durability**: Calls `sync_data()` per file to ensure data is written to physical media without triggering expensive filesystem metadata log commits.
3. **Background Journal Flusher**: Offloads JSON serialization and disk updates to an async `std::sync::mpsc` background thread, executing time-throttled checkpoints every 2 seconds.

---

## 📄 License

Distributed under the [MIT License](LICENSE).
