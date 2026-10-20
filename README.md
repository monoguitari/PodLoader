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
     sudo mount -t drvfs D: /mnt/d -o uid=$(id -u),gid=$(id -g)
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

### Browsing Host Files & Syncing Audio to Your iPod

1. **Check `FireWire GUID` (for 2007+ iPods such as Classic 6G/7G and Nano 3G/4G)**:
   - Look at the top header. If `FireWire GUID` is displayed, PodLoader read it from `iPod_Control/Device/SysInfo` (or USB) and will automatically sign `iTunesDB` with `hash58`/`hash72`.
   - If your iPod is a 2007+ model and no `FireWire GUID` is shown, enter your 16-character hex `FireWire GUID` in the **Manual FireWire GUID** field and click **Save GUID** (this caches it into `iPod_Control/Device/SysInfo`).
2. **Browse Local Music (`Host Files` Pane)**:
   - Enter any folder path on your computer containing iPod-native audio files (`.mp3`, `.m4a`, `.aac`, `.alac`, `.wav`) — for example `/mnt/c/Users/<YourWindowsUser>/Music` or `/home/jesus/music` — and click **Browse** (or navigate using `📁` subfolders and **⬆️ Parent Folder**).
3. **Stage Tracks into the `Sync Batch`**:
   - Click **Stage** next to an individual track, **Stage Folder** next to a subfolder, **Stage All to Sync Batch** for the current directory, or **drag and drop** a track/folder onto the **Device Tracks** table or a user **Device Playlist** in the sidebar.
   - Any track matching an existing `Device Track` by `(Artist, Album, Title, Track #)` is flagged as **Duplicate** and defaults to **Skip**; use the per-item dropdown to switch between **Skip**, **Import Anyway**, or **Replace Existing**.
4. **Commit the `Sync Batch`**:
   - Click **Commit Sync Batch**. PodLoader verifies free disk space, copies the audio files into randomized `iPod_Control/Music/F00`–`F49` paths, backs up the previous database to `iPod_Control/iTunes/iTunesDB.bak`, atomically writes and signs `iTunesDB`, and updates the UI.
   - If a transfer is paused or interrupted mid-batch, choose between **Commit Completed Tracks** or **Roll Back Batch**.
5. **Safely Unmount Your iPod**:
   ```bash
   sudo umount /mnt/d
   ```
   Then safely eject the iPod in your OS before unplugging the USB cable.

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
