# PodLoader

Cross-platform desktop application for inspecting, editing, and transferring audio to legacy USB mass-storage Apple iPods without iTunes.

## Language

**Device**:
A connected USB mass-storage iPod containing an `iPod_Control` directory structure.
_Avoid_: Player, client, target, drive

**Device Initialization**:
The process of creating the canonical `iPod_Control` directory hierarchy (`Music/F00`–`F49`, `iTunes`, `Device/SysInfo`) and writing a clean `iTunesDB` with an empty Master Playlist onto a freshly formatted volume.
_Avoid_: Formatting, provisioning, bootstrapping

**Device Library**:
The complete index of tracks and playlists stored inside a Device's `iTunesDB` binary database.
_Avoid_: iPod database, local library, sync database

**Device Track**:
An audio entry registered in the Device Library and stored on the Device at an Obfuscated Path under `iPod_Control/Music/Fxx/`.
_Avoid_: Song, iPod file, media item

**Obfuscated Path**:
The randomized four-letter filename inside `iPod_Control/Music/F00/`–`F49/` (stored in `iTunesDB` with colon separators, e.g. `:iPod_Control:Music:F14:ABCD.m4a`) where a Device Track's audio file resides.
_Avoid_: Hidden file, hashed filename, storage path

**Orphan File**:
An audio file located inside `iPod_Control/Music/Fxx/` on a Device that has no corresponding Device Track entry in `iTunesDB`.
_Avoid_: Dangling track, ghost file, unlinked song

**Play Counts File**:
The ephemeral binary file (`iPod_Control/iTunes/Play Counts`) written by the iPod firmware between connections to record recent play counts, last-played timestamps, and on-device star rating changes before they are merged into `iTunesDB`.
_Avoid_: Stats file, history log

**Master Playlist**:
The mandatory primary playlist entry in `iTunesDB` that references every Device Track in the Device Library.
_Avoid_: Main playlist, root list, library playlist

**Device Playlist**:
A user-visible ordered list of Device Tracks stored in the Device Library (separate from the Master Playlist and rule-based Smart Playlists).
_Avoid_: Folder, album list, queue

**Host File**:
An audio file residing on the user's computer filesystem that can be browsed, previewed, or queued for transfer to a Device.
_Avoid_: Local song, PC track, source track

**Sync Batch**:
A staged set of pending changes—Host Files to add (with optional transcoding), Device Tracks to remove, Orphan Files to recover or delete, metadata edits, and Device Playlist modifications—applied atomically to a Device.
_Avoid_: Sync, live edit, transaction

**Device Export**:
Copying selected Device Tracks from their Obfuscated Paths on a Device to a human-readable folder hierarchy (`Artist/Album/Track - Title.ext`) on the host computer.
_Avoid_: Rip, reverse sync, backup

**FireWire GUID**:
The 8-byte hardware identifier of a Device (read from the USB device serial descriptor or cached in `iPod_Control/Device/SysInfo`) required to compute `hash58` and `hash72` database signatures on 2007+ iPods.
_Avoid_: Serial number, UUID, device hash
