# PodLoader

Cross-platform desktop application for inspecting, editing, and transferring audio to legacy USB mass-storage Apple iPods without iTunes or `libgpod`.

## Prerequisites

Make sure `cargo`/`rustc` and `node`/`npm` are on your `PATH`. On this WSL2 machine, they are installed under `/home/jesus/.cache/cargo/bin`:

```bash
export PATH="/home/jesus/.cache/cargo/bin:$PATH"
export CARGO_HOME="/home/jesus/.cache/cargo"
export RUSTUP_HOME="/home/jesus/.cache/rustup"
```

---

## Connecting Your iPod

1. Plug your iPod in over USB and ensure it is in **Disk Mode** / mounted as a USB mass-storage drive.
2. **On WSL2 (Windows host)**:
   - Check which drive letter Windows assigned to your iPod (for example `D:` or `E:`).
   - Mount that Windows drive into WSL2 if it isn't mounted automatically:
     ```bash
     sudo mkdir -p /mnt/d
     sudo mount -t drvfs D: /mnt/d
     ```
   - Verify the `iPod_Control` folder is visible:
     ```bash
     ls -la /mnt/d/iPod_Control
     ```
3. **On macOS / Linux**:
   - PodLoader automatically scans `/Volumes`, `/media`, `/run/media`, and `/mnt` for any volume containing `iPod_Control`, or you can pass the mount path explicitly.

---

## Option 1: Quick CLI Test (Read Your iPod in the Terminal)

Run the Rust inspector directly against your iPod's mount path (or let it auto-detect):

```bash
# Auto-detect across /Volumes, /media, /run/media, /mnt:
npm run inspect

# Or pass your iPod mount path explicitly (e.g. /mnt/d):
npm run inspect -- /mnt/d
```

This parses `iPod_Control/Device/SysInfo`, `iPod_Control/iTunes/iTunesDB`, and merges `iPod_Control/iTunes/Play Counts` (if present), printing:
- Device name, model number, `FireWire GUID`, and used/free/total GB
- Every playlist (`Master Playlist`, user `Device Playlists`, and `Smart Playlists`) with track counts
- Track list with Title, Artist, Album, Duration, Bitrate, Play Count, and Star Rating

---

## Option 2: Run the Full UI (`PodLoader` React + Rust Backend)

Start the Rust backend server in one terminal and the Vite React UI in a second terminal:

**Terminal 1 — Rust Backend:**
```bash
export PATH="/home/jesus/.cache/cargo/bin:$PATH"
export CARGO_HOME="/home/jesus/.cache/cargo"
export RUSTUP_HOME="/home/jesus/.cache/rustup"

# Pass your iPod mount path (or omit to auto-detect):
npm run dev:backend -- /mnt/d
```

**Terminal 2 — React UI:**
```bash
export PATH="/home/jesus/.cache/cargo/bin:$PATH"
npm run dev
```

Then open **http://localhost:5173** in your browser to browse your iPod's storage summary, playlists (`Master Playlist`, `Device Playlists`, `Smart Playlists`), and ordered `Device Tracks` table (or run `Device Initialization` on a freshly formatted drive).

---

## Running Tests

```bash
# Run Rust integration tests (PodLoaderEngine seam)
cargo test --offline

# Run TypeScript/React integration tests (PodLoaderBridge seam)
npm test

# Run TypeScript typechecking
npm run typecheck
```
