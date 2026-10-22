//! Core domain engine for PodLoader (`ADR-0001`, `ADR-0004`, `ADR-0006`).

pub mod audio;
pub mod host;
pub mod itunesdb;
pub mod playcounts;
pub mod signing;
pub mod sysinfo;
pub mod tagging;
pub mod transcode;
pub mod fixtures;

pub use crate::audio::{
    ActivePlaybackTrack, AudioSinkAdapter, AudioSourceKind, HeadlessAudioSinkAdapter,
    PlaybackState, PlaybackStatus,
};

use std::cell::RefCell;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::host::{
    collect_host_files_from_paths, is_native_ipod_format, is_supported_audio_path,
    is_transcodable_format,
};
use crate::itunesdb::{build_empty_itunesdb, ItunesDb};
use crate::playcounts::{merge_play_counts_into_tracks, parse_play_counts};
pub use crate::signing::verify_itunesdb_signatures;
use crate::signing::{
    extract_hash72_params, parse_firewire_guid_bytes, resign_itunesdb_hash72_with_params,
    sign_itunesdb,
};
use crate::sysinfo::{cache_guid_in_sysinfo, normalize_firewire_guid, parse_sysinfo};

#[derive(Debug, Error)]
pub enum PodLoaderError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid iTunesDB format: {0}")]
    InvalidItunesDb(String),
    #[error("Invalid Play Counts file: {0}")]
    InvalidPlayCounts(String),
    #[error("Volume not found at {0}")]
    VolumeNotFound(PathBuf),
    #[error("Directory not found at {0}")]
    DirectoryNotFound(PathBuf),
    #[error("Insufficient disk space: required {required_bytes} bytes, available {available_bytes} bytes")]
    InsufficientDiskSpace {
        required_bytes: u64,
        available_bytes: u64,
    },
    #[error("Invalid FireWire GUID: {0}")]
    InvalidFireWireGuid(String),
    #[error("No active Device session")]
    NoActiveDevice,
    #[error("No interrupted Sync Batch to resolve")]
    NoInterruptedBatch,
    #[error("System ffmpeg is required for lossy AAC/MP3 transcoding but was not found on PATH")]
    FfmpegUnavailable,
    #[error("Transcode failed: {0}")]
    TranscodeFailed(String),
    #[error("Device operation failed: {0}")]
    DeviceOperationFailed(String),
    #[error("Uncommitted changes are pending in the Sync Batch")]
    UncommittedChangesPending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDirectoryEntry {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostFile {
    pub path: PathBuf,
    pub file_name: String,
    pub format: String,
    pub file_size: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_number: u32,
    pub disc_number: u32,
    pub genre: String,
    pub year: u32,
    pub duration_ms: u32,
    pub bitrate_kbps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDirectoryListing {
    pub current_path: PathBuf,
    pub parent_path: Option<PathBuf>,
    pub subdirectories: Vec<HostDirectoryEntry>,
    pub files: Vec<HostFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MountedVolume {
    pub mount_path: PathBuf,
    pub volume_name: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub usb_firewire_guid: Option<String>,
}

pub trait HardwareDeviceAdapter: Send + Sync {
    fn list_mounted_volumes(&self) -> Result<Vec<MountedVolume>, PodLoaderError>;
    fn lookup_usb_firewire_guid(&self, mount_path: &Path) -> Option<String>;
    fn is_ffmpeg_available(&self) -> bool {
        false
    }
    fn transcode_lossy(
        &self,
        input_path: &Path,
        output_path: &Path,
        preset: TranscodePreset,
        host_file: &HostFile,
    ) -> Result<(), PodLoaderError> {
        if !self.is_ffmpeg_available() {
            return Err(PodLoaderError::FfmpegUnavailable);
        }
        let codec_args: &[&str] = match preset {
            TranscodePreset::Aac => &["-vn", "-c:a", "aac", "-b:a", "256k"],
            TranscodePreset::Mp3 => &["-vn", "-c:a", "libmp3lame", "-b:a", "256k"],
            TranscodePreset::Alac => {
                return Err(PodLoaderError::TranscodeFailed(
                    "Lossless ALAC is transcoded via pure-Rust pipeline".into(),
                ));
            }
        };
        let status = std::process::Command::new("ffmpeg")
            .arg("-y")
            .arg("-i")
            .arg(input_path)
            .args(codec_args)
            .arg("-metadata")
            .arg(format!("title={}", host_file.title))
            .arg("-metadata")
            .arg(format!("artist={}", host_file.artist))
            .arg("-metadata")
            .arg(format!("album={}", host_file.album))
            .arg("-metadata")
            .arg(format!("track={}", host_file.track_number))
            .arg("-metadata")
            .arg(format!("disc={}", host_file.disc_number))
            .arg("-metadata")
            .arg(format!("genre={}", host_file.genre))
            .arg("-metadata")
            .arg(format!("date={}", host_file.year))
            .arg(output_path)
            .status()?;
        if !status.success() {
            return Err(PodLoaderError::TranscodeFailed(format!(
                "ffmpeg exited with status {}",
                status
            )));
        }
        Ok(())
    }

    fn eject_volume(&self, _mount_path: &Path) -> Result<(), PodLoaderError> {
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct SystemHardwareDeviceAdapter;

impl HardwareDeviceAdapter for SystemHardwareDeviceAdapter {
    fn list_mounted_volumes(&self) -> Result<Vec<MountedVolume>, PodLoaderError> {
        let mut candidates = Vec::new();
        if let Ok(override_path) = std::env::var("PODLOADER_DEVICE_MOUNT") {
            let p = PathBuf::from(&override_path);
            if p.is_dir() {
                candidates.push(p);
            }
        }

        for root in ["/Volumes", "/media", "/run/media", "/mnt"] {
            let root_path = Path::new(root);
            if let Ok(entries) = fs::read_dir(root_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.join("iPod_Control").is_dir() {
                        candidates.push(path);
                    }
                }
            }
        }

        let usb_guid = self.scan_apple_usb_serial_guid();
        let mut volumes = Vec::new();
        for mount_path in candidates {
            let volume_name = mount_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("IPOD")
                .to_string();
            let (total_bytes, free_bytes) = query_volume_space(&mount_path);
            volumes.push(MountedVolume {
                mount_path,
                volume_name,
                total_bytes,
                free_bytes,
                usb_firewire_guid: usb_guid.clone(),
            });
        }
        Ok(volumes)
    }

    fn lookup_usb_firewire_guid(&self, _mount_path: &Path) -> Option<String> {
        self.scan_apple_usb_serial_guid()
    }

    fn is_ffmpeg_available(&self) -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn eject_volume(&self, mount_path: &Path) -> Result<(), PodLoaderError> {
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("sync").status();

            let path_str = mount_path.to_string_lossy();
            let mut ejected_via_wsl_powershell = false;

            // Detect WSL Windows drive mounts such as /mnt/d or /mnt/e
            if let Some(rest) = path_str.strip_prefix("/mnt/") {
                let trimmed = rest.trim_end_matches(['/', '\\']);
                if trimmed.len() == 1 && trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
                    let drive_letter = format!("{}:", trimmed.to_ascii_uppercase());
                    if let Ok(mut child) = std::process::Command::new("powershell.exe")
                        .current_dir("/mnt/c")
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .args([
                            "-NoProfile",
                            "-Command",
                            &format!(
                                "(New-Object -comObject Shell.Application).Namespace(17).ParseName('{}').InvokeVerb('Eject')",
                                drive_letter
                            ),
                        ])
                        .spawn()
                    {
                        if let Ok(status) = child.wait() {
                            if status.success() {
                                ejected_via_wsl_powershell = true;
                            }
                        }
                    }
                }
            }

            if !ejected_via_wsl_powershell {
                let _ = std::process::Command::new("eject")
                    .arg(mount_path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("sync").status();
            let _ = std::process::Command::new("diskutil")
                .args(["eject", &mount_path.to_string_lossy()])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        #[cfg(target_os = "windows")]
        {
            let drive_path = mount_path.to_string_lossy();
            let drive_name = if drive_path.len() >= 2 && &drive_path[1..2] == ":" {
                format!("{}:", &drive_path[0..1].to_ascii_uppercase())
            } else {
                drive_path.to_string()
            };
            let _ = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!(
                        "(New-Object -comObject Shell.Application).Namespace(17).ParseName('{}').InvokeVerb('Eject')",
                        drive_name
                    ),
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        Ok(())
    }
}

fn query_volume_space(mount_path: &Path) -> (u64, u64) {
    let output = std::process::Command::new("df")
        .arg("-kP")
        .arg(mount_path)
        .output();
    if let Ok(out) = output {
        if let Ok(text) = String::from_utf8(out.stdout) {
            if let Some(line) = text.lines().nth(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 4 {
                    let total_kb = cols[1].parse::<u64>().unwrap_or(0);
                    let avail_kb = cols[3].parse::<u64>().unwrap_or(0);
                    return (total_kb * 1000, avail_kb * 1000);
                }
            }
        }
    }
    (0, 0)
}

impl SystemHardwareDeviceAdapter {
    fn scan_apple_usb_serial_guid(&self) -> Option<String> {
        let usb_root = Path::new("/sys/bus/usb/devices");
        if let Ok(entries) = fs::read_dir(usb_root) {
            for entry in entries.flatten() {
                let dir = entry.path();
                let Ok(vendor) = fs::read_to_string(dir.join("idVendor")) else {
                    continue;
                };
                if vendor.trim().eq_ignore_ascii_case("05ac") {
                    if let Ok(serial) = fs::read_to_string(dir.join("serial")) {
                        let trimmed = serial.trim();
                        if trimmed.len() >= 16 {
                            let candidate = &trimmed[..16];
                            if candidate.chars().all(|c| c.is_ascii_hexdigit()) {
                                return Some(candidate.to_uppercase());
                            }
                        }
                    }
                }
            }
        }

        // On WSL2 (when WSL_INTEROP socket is active), query Windows USB Enum registry for Apple iPod FireWire GUID (000A27...)
        if let Ok(interop) = std::env::var("WSL_INTEROP") {
            if Path::new(&interop).exists() && Path::new("/mnt/c/Windows/System32/reg.exe").is_file()
            {
                if let Ok(out) = std::process::Command::new("/mnt/c/Windows/System32/reg.exe")
                    .args([
                        "query",
                        r"HKLM\SYSTEM\CurrentControlSet\Enum\USB",
                        "/s",
                        "/f",
                        "000A27",
                        "/k",
                    ])
                    .output()
                {
                    let text = String::from_utf8_lossy(&out.stdout);
                    for line in text.lines() {
                        if let Some(last_seg) = line.trim().rsplit('\\').next() {
                            if last_seg.len() >= 16 {
                                let candidate = &last_seg[..16];
                                if candidate.to_uppercase().starts_with("000A27")
                                    && candidate.chars().all(|c| c.is_ascii_hexdigit())
                                {
                                    return Some(candidate.to_uppercase());
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceMetadata {
    pub mount_path: PathBuf,
    pub name: String,
    pub model_number: Option<String>,
    pub firewire_guid: Option<String>,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTrack {
    pub id: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_number: u32,
    pub disc_number: u32,
    pub genre: String,
    pub year: u32,
    pub duration_ms: u32,
    pub bitrate_kbps: u32,
    pub play_count: u32,
    pub rating: u8,
    pub last_played_timestamp: Option<u32>,
    pub bookmark_ms: u32,
    pub obfuscated_path: String,
    #[serde(default)]
    pub album_artist: Option<String>,
    #[serde(default)]
    pub is_compilation: bool,
}

impl DeviceTrack {
    pub fn matches_host_file(&self, host_file: &HostFile) -> bool {
        normalize_tag_key(&self.artist) == normalize_tag_key(&host_file.artist)
            && normalize_tag_key(&self.album) == normalize_tag_key(&host_file.album)
            && normalize_tag_key(&self.title) == normalize_tag_key(&host_file.title)
            && self.track_number == host_file.track_number
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaylistKind {
    Master,
    User,
    Smart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePlaylist {
    pub id: u64,
    pub name: String,
    pub kind: PlaylistKind,
    pub track_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLibrary {
    pub tracks: Vec<DeviceTrack>,
    pub playlists: Vec<DevicePlaylist>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DuplicateResolution {
    Skip,
    ImportAnyway,
    ReplaceExisting,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscodePreset {
    #[default]
    Alac,
    Aac,
    Mp3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedTrackAddition {
    pub staged_id: u64,
    pub host_file: HostFile,
    pub target_playlist_id: Option<u64>,
    pub is_duplicate: bool,
    pub matched_device_track_id: Option<u32>,
    pub duplicate_resolution: DuplicateResolution,
    pub requires_transcode: bool,
    pub target_format: Option<TranscodePreset>,
}

impl StagedTrackAddition {
    pub fn should_transfer(&self) -> bool {
        !self.is_duplicate || self.duplicate_resolution != DuplicateResolution::Skip
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackMetadataEdits {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub is_compilation: Option<bool>,
    pub rating: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedMetadataEdit {
    pub staged_id: u64,
    pub track_id: u32,
    pub edits: TrackMetadataEdits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedTrackRemoval {
    pub staged_id: u64,
    pub track_id: u32,
    pub track_title: String,
    pub track_artist: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanFile {
    pub path: PathBuf,
    pub obfuscated_path: String,
    pub file_name: String,
    pub file_size: u64,
    pub format: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_number: u32,
    pub disc_number: u32,
    pub genre: String,
    pub year: u32,
    pub duration_ms: u32,
    pub bitrate_kbps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedOrphanRecovery {
    pub staged_id: u64,
    pub orphan_file: OrphanFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedOrphanDeletion {
    pub staged_id: u64,
    pub orphan_file: OrphanFile,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncBatch {
    pub additions: Vec<StagedTrackAddition>,
    #[serde(default)]
    pub edits: Vec<StagedMetadataEdit>,
    #[serde(default)]
    pub removals: Vec<StagedTrackRemoval>,
    #[serde(default)]
    pub orphan_recoveries: Vec<StagedOrphanRecovery>,
    #[serde(default)]
    pub orphan_deletions: Vec<StagedOrphanDeletion>,
    pub required_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptedBatchSummary {
    pub completed_track_count: usize,
    pub remaining_track_count: usize,
    pub written_obfuscated_paths: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InterruptedResolution {
    CommitCompleted,
    RollBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub current_track_index: usize,
    pub total_tracks: usize,
    pub current_track_title: String,
    pub is_transcoding: bool,
    pub track_percent: u8,
    pub overall_percent: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", content = "data", rename_all = "camelCase")]
pub enum CommitOutcome {
    Completed(DeviceSession),
    Interrupted(InterruptedBatchSummary),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSession {
    pub device: DeviceMetadata,
    pub library: DeviceLibrary,
    pub has_pending_play_counts: bool,
    pub sync_batch: SyncBatch,
    pub interrupted_batch: Option<InterruptedBatchSummary>,
    pub transcode_preset: TranscodePreset,
    pub ffmpeg_available: bool,
    #[serde(default)]
    pub orphan_files: Vec<OrphanFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "camelCase")]
pub enum DeviceConnectionState {
    Disconnected,
    UninitializedVolume(MountedVolume),
    Connected(DeviceSession),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub exported_count: usize,
    pub exported_files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct InterruptedJournal {
    summary: InterruptedBatchSummary,
    mount_path: PathBuf,
    original_db_bytes: Vec<u8>,
    partial_db: ItunesDb,
    completed_staged_ids: Vec<u64>,
    created_disk_paths: Vec<PathBuf>,
    old_paths_to_remove: Vec<PathBuf>,
    firewire_guid: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct EngineState {
    active_mount_path: Option<PathBuf>,
    cached_session: Option<DeviceSession>,
    sync_batch: SyncBatch,
    next_staged_id: u64,
    next_obfuscated_counter: u64,
    interrupted_journal: Option<InterruptedJournal>,
    transcode_preset: TranscodePreset,
    ejected_mount_paths: Vec<PathBuf>,
}

pub struct PodLoaderEngine<A: HardwareDeviceAdapter, S: AudioSinkAdapter = HeadlessAudioSinkAdapter> {
    adapter: A,
    audio_sink: RefCell<S>,
    active_track: RefCell<Option<ActivePlaybackTrack>>,
    state: RefCell<EngineState>,
}

impl<A: HardwareDeviceAdapter> PodLoaderEngine<A, HeadlessAudioSinkAdapter> {
    pub fn new(adapter: A) -> Self {
        Self::new_with_audio_sink(adapter, HeadlessAudioSinkAdapter::default())
    }
}

impl<A: HardwareDeviceAdapter, S: AudioSinkAdapter> PodLoaderEngine<A, S> {
    pub fn new_with_audio_sink(adapter: A, audio_sink: S) -> Self {
        Self {
            adapter,
            audio_sink: RefCell::new(audio_sink),
            active_track: RefCell::new(None),
            state: RefCell::new(EngineState {
                active_mount_path: None,
                cached_session: None,
                sync_batch: SyncBatch::default(),
                next_staged_id: 1,
                next_obfuscated_counter: 1,
                interrupted_journal: None,
                transcode_preset: TranscodePreset::Alac,
                ejected_mount_paths: Vec::new(),
            }),
        }
    }

    pub fn scan_device(&self) -> Result<DeviceConnectionState, PodLoaderError> {
        let volumes = self.adapter.list_mounted_volumes()?;
        let st = self.state.borrow();
        let Some(vol) = volumes
            .into_iter()
            .find(|v| !st.ejected_mount_paths.contains(&v.mount_path))
        else {
            drop(st);
            let mut st = self.state.borrow_mut();
            st.active_mount_path = None;
            st.cached_session = None;
            return Ok(DeviceConnectionState::Disconnected);
        };
        drop(st);

        let itunesdb_path = vol
            .mount_path
            .join("iPod_Control")
            .join("iTunes")
            .join("iTunesDB");

        if !itunesdb_path.is_file() {
            let mut st = self.state.borrow_mut();
            st.active_mount_path = None;
            st.cached_session = None;
            return Ok(DeviceConnectionState::UninitializedVolume(vol));
        }

        self.load_connected_session(vol)
    }

    pub fn initialize_device(
        &self,
        mount_path: &Path,
        device_name: &str,
    ) -> Result<DeviceConnectionState, PodLoaderError> {
        self.state
            .borrow_mut()
            .ejected_mount_paths
            .retain(|p| p != mount_path);

        let volumes = self.adapter.list_mounted_volumes()?;
        let vol = volumes
            .into_iter()
            .find(|v| v.mount_path == mount_path)
            .ok_or_else(|| PodLoaderError::VolumeNotFound(mount_path.to_path_buf()))?;

        let ipod_control = mount_path.join("iPod_Control");
        let music_dir = ipod_control.join("Music");
        for idx in 0..50 {
            fs::create_dir_all(music_dir.join(format!("F{:02}", idx)))?;
        }

        let itunes_dir = ipod_control.join("iTunes");
        fs::create_dir_all(&itunes_dir)?;

        let device_dir = ipod_control.join("Device");
        fs::create_dir_all(&device_dir)?;

        let sysinfo_path = device_dir.join("SysInfo");
        let guid = self
            .adapter
            .lookup_usb_firewire_guid(mount_path)
            .or_else(|| vol.usb_firewire_guid.clone());

        let mut sysinfo_contents = String::new();
        if let Some(g) = &guid {
            sysinfo_contents.push_str(&format!(
                "FireWireGuid: 0x{}\n",
                normalize_firewire_guid(g)
            ));
        }
        fs::write(&sysinfo_path, sysinfo_contents)?;

        let initial_db = build_empty_itunesdb(device_name);
        fs::write(itunes_dir.join("iTunesDB"), initial_db)?;

        self.load_connected_session(vol)
    }

    pub fn reserialize_device_db(&self, mount_path: &Path) -> Result<Vec<u8>, PodLoaderError> {
        let itunesdb_path = mount_path
            .join("iPod_Control")
            .join("iTunes")
            .join("iTunesDB");
        let raw_bytes = fs::read(itunesdb_path)?;
        let parsed = ItunesDb::parse(&raw_bytes)?;
        Ok(parsed.to_bytes())
    }

    pub fn browse_host_directory(
        &self,
        dir_path: &Path,
    ) -> Result<HostDirectoryListing, PodLoaderError> {
        host::browse_directory(dir_path)
    }

    pub fn stage_host_paths(
        &mut self,
        paths: &[PathBuf],
        target_playlist_id: Option<u64>,
    ) -> Result<SyncBatch, PodLoaderError> {
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };

        let host_files = collect_host_files_from_paths(paths)?;
        let mut st = self.state.borrow_mut();

        for hf in host_files {
            let is_native = is_native_ipod_format(&hf.format);
            let is_transcodable = is_transcodable_format(&hf.format);
            if !is_native && !is_transcodable {
                continue;
            }
            let matched = current
                .library
                .tracks
                .iter()
                .find(|dt| dt.matches_host_file(&hf));
            let is_duplicate = matched.is_some();
            let staged_id = st.next_staged_id;
            st.next_staged_id += 1;
            let active_preset = st.transcode_preset;

            st.sync_batch.additions.push(StagedTrackAddition {
                staged_id,
                host_file: hf,
                target_playlist_id,
                is_duplicate,
                matched_device_track_id: matched.map(|t| t.id),
                duplicate_resolution: if is_duplicate {
                    DuplicateResolution::Skip
                } else {
                    DuplicateResolution::ImportAnyway
                },
                requires_transcode: is_transcodable,
                target_format: if is_transcodable {
                    Some(active_preset)
                } else {
                    None
                },
            });
        }

        recalculate_required_bytes(&mut st.sync_batch);
        Ok(st.sync_batch.clone())
    }

    pub fn set_duplicate_resolution(
        &mut self,
        staged_id: u64,
        resolution: DuplicateResolution,
    ) -> Result<SyncBatch, PodLoaderError> {
        let mut st = self.state.borrow_mut();
        if st.active_mount_path.is_none() {
            return Err(PodLoaderError::NoActiveDevice);
        }
        if let Some(item) = st
            .sync_batch
            .additions
            .iter_mut()
            .find(|a| a.staged_id == staged_id)
        {
            item.duplicate_resolution = resolution;
        }
        recalculate_required_bytes(&mut st.sync_batch);
        Ok(st.sync_batch.clone())
    }

    pub fn stage_track_metadata_edits(
        &mut self,
        track_ids: &[u32],
        edits: TrackMetadataEdits,
    ) -> Result<SyncBatch, PodLoaderError> {
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };
        let mut st = self.state.borrow_mut();
        for &tid in track_ids {
            if current.library.tracks.iter().any(|t| t.id == tid) {
                if let Some(existing) = st.sync_batch.edits.iter_mut().find(|e| e.track_id == tid) {
                    merge_edits(&mut existing.edits, &edits);
                } else {
                    let staged_id = st.next_staged_id;
                    st.next_staged_id += 1;
                    st.sync_batch.edits.push(StagedMetadataEdit {
                        staged_id,
                        track_id: tid,
                        edits: edits.clone(),
                    });
                }
            }
        }
        Ok(st.sync_batch.clone())
    }

    pub fn stage_track_removals(
        &mut self,
        track_ids: &[u32],
    ) -> Result<SyncBatch, PodLoaderError> {
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };
        let mut st = self.state.borrow_mut();
        for &tid in track_ids {
            if let Some(track) = current.library.tracks.iter().find(|t| t.id == tid) {
                if !st.sync_batch.removals.iter().any(|r| r.track_id == tid) {
                    let staged_id = st.next_staged_id;
                    st.next_staged_id += 1;
                    st.sync_batch.removals.push(StagedTrackRemoval {
                        staged_id,
                        track_id: tid,
                        track_title: track.title.clone(),
                        track_artist: track.artist.clone(),
                    });
                }
            }
        }
        Ok(st.sync_batch.clone())
    }

    pub fn stage_orphan_recovery(
        &mut self,
        obfuscated_path: &str,
    ) -> Result<SyncBatch, PodLoaderError> {
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };
        let orphan = current
            .orphan_files
            .iter()
            .find(|o| o.obfuscated_path == obfuscated_path)
            .ok_or_else(|| {
                PodLoaderError::DeviceOperationFailed(format!(
                    "Orphan file {} not found on device",
                    obfuscated_path
                ))
            })?
            .clone();

        let mut st = self.state.borrow_mut();
        st.sync_batch
            .orphan_deletions
            .retain(|d| d.orphan_file.obfuscated_path != obfuscated_path);
        if !st
            .sync_batch
            .orphan_recoveries
            .iter()
            .any(|r| r.orphan_file.obfuscated_path == obfuscated_path)
        {
            let staged_id = st.next_staged_id;
            st.next_staged_id += 1;
            st.sync_batch.orphan_recoveries.push(StagedOrphanRecovery {
                staged_id,
                orphan_file: orphan,
            });
        }
        Ok(st.sync_batch.clone())
    }

    pub fn stage_orphan_deletion(
        &mut self,
        obfuscated_path: &str,
    ) -> Result<SyncBatch, PodLoaderError> {
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };
        let orphan = current
            .orphan_files
            .iter()
            .find(|o| o.obfuscated_path == obfuscated_path)
            .ok_or_else(|| {
                PodLoaderError::DeviceOperationFailed(format!(
                    "Orphan file {} not found on device",
                    obfuscated_path
                ))
            })?
            .clone();

        let mut st = self.state.borrow_mut();
        st.sync_batch
            .orphan_recoveries
            .retain(|r| r.orphan_file.obfuscated_path != obfuscated_path);
        if !st
            .sync_batch
            .orphan_deletions
            .iter()
            .any(|d| d.orphan_file.obfuscated_path == obfuscated_path)
        {
            let staged_id = st.next_staged_id;
            st.next_staged_id += 1;
            st.sync_batch.orphan_deletions.push(StagedOrphanDeletion {
                staged_id,
                orphan_file: orphan,
            });
        }
        Ok(st.sync_batch.clone())
    }

    pub fn set_transcode_preset(
        &mut self,
        preset: TranscodePreset,
    ) -> Result<DeviceSession, PodLoaderError> {
        let _ = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };
        if matches!(preset, TranscodePreset::Aac | TranscodePreset::Mp3)
            && !self.adapter.is_ffmpeg_available()
        {
            return Err(PodLoaderError::FfmpegUnavailable);
        }
        {
            let mut st = self.state.borrow_mut();
            st.transcode_preset = preset;
            for item in &mut st.sync_batch.additions {
                if item.requires_transcode {
                    item.target_format = Some(preset);
                }
            }
        }
        match self.scan_device()? {
            DeviceConnectionState::Connected(updated) => Ok(updated),
            _ => Err(PodLoaderError::NoActiveDevice),
        }
    }

    pub fn commit_sync_batch(&mut self) -> Result<CommitOutcome, PodLoaderError> {
        self.commit_sync_batch_with_limit(None)
    }

    pub fn commit_sync_batch_with_limit(
        &mut self,
        max_tracks_before_interrupt: Option<usize>,
    ) -> Result<CommitOutcome, PodLoaderError> {
        self.commit_sync_batch_with_progress(max_tracks_before_interrupt, |_| {})
    }

    pub fn commit_sync_batch_with_progress<F>(
        &mut self,
        max_tracks_before_interrupt: Option<usize>,
        mut on_progress: F,
    ) -> Result<CommitOutcome, PodLoaderError>
    where
        F: FnMut(TransferProgress),
    {
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };

        let required_bytes = self.state.borrow().sync_batch.required_bytes;
        if required_bytes > current.device.free_bytes {
            return Err(PodLoaderError::InsufficientDiskSpace {
                required_bytes,
                available_bytes: current.device.free_bytes,
            });
        }

        let mount_path = current.device.mount_path.clone();
        let itunes_dir = mount_path.join("iPod_Control").join("iTunes");
        let itunesdb_path = itunes_dir.join("iTunesDB");
        let original_db_bytes = fs::read(&itunesdb_path)?;
        let mut parsed_db = ItunesDb::parse(&original_db_bytes)?;

        let play_counts_path = itunes_dir.join("Play Counts");
        if play_counts_path.is_file() {
            let pc_bytes = fs::read(&play_counts_path)?;
            let entries = parse_play_counts(&pc_bytes)?;
            parsed_db.merge_play_counts(&entries);
        }

        let active_additions: Vec<StagedTrackAddition> = self
            .state
            .borrow()
            .sync_batch
            .additions
            .iter()
            .filter(|a| a.should_transfer())
            .cloned()
            .collect();

        let total_active = active_additions.len();
        let mut completed_staged_ids: Vec<u64> = Vec::new();
        let mut created_disk_paths: Vec<PathBuf> = Vec::new();
        let mut written_obfuscated_paths: Vec<String> = Vec::new();
        let mut old_paths_to_remove: Vec<PathBuf> = Vec::new();
        let mut interruption_reason: Option<String> = None;

        for (idx, item) in active_additions.iter().enumerate() {
            if let Some(limit) = max_tracks_before_interrupt {
                if idx >= limit {
                    interruption_reason = Some("Transfer interrupted mid-batch".to_string());
                    break;
                }
            }

            let target_ext = if item.requires_transcode {
                match item.target_format.unwrap_or(TranscodePreset::Alac) {
                    TranscodePreset::Alac | TranscodePreset::Aac => Some("m4a"),
                    TranscodePreset::Mp3 => Some("mp3"),
                }
            } else {
                None
            };

            let (disk_path, obfuscated_path) = {
                let mut st = self.state.borrow_mut();
                allocate_obfuscated_path(
                    &mount_path,
                    &item.host_file.path,
                    target_ext,
                    &mut st.next_obfuscated_counter,
                )?
            };

            if let Some(parent) = disk_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let mut db_host_file = item.host_file.clone();
            let mut track_sample_rate_hz: Option<u32> = None;

            on_progress(TransferProgress {
                current_track_index: idx + 1,
                total_tracks: total_active,
                current_track_title: item.host_file.title.clone(),
                is_transcoding: item.requires_transcode,
                track_percent: 0,
                overall_percent: if total_active > 0 {
                    ((idx * 100) / total_active) as u8
                } else {
                    0
                },
            });

            if item.requires_transcode {
                let preset = item.target_format.unwrap_or(TranscodePreset::Alac);
                let title_for_cb = item.host_file.title.clone();
                let transcode_result = match preset {
                    TranscodePreset::Alac => transcode::transcode_to_alac_m4a(
                        &item.host_file,
                        &disk_path,
                        |done, total| {
                            let track_pct = if total > 0 {
                                ((done.saturating_mul(100)) / total).min(100) as u8
                            } else {
                                50
                            };
                            let overall_pct = if total_active > 0 {
                                (((idx * 100) + (track_pct as usize)) / total_active).min(100) as u8
                            } else {
                                track_pct
                            };
                            on_progress(TransferProgress {
                                current_track_index: idx + 1,
                                total_tracks: total_active,
                                current_track_title: title_for_cb.clone(),
                                is_transcoding: true,
                                track_percent: track_pct,
                                overall_percent: overall_pct,
                            });
                        },
                    ),
                    TranscodePreset::Aac | TranscodePreset::Mp3 => self
                        .adapter
                        .transcode_lossy(&item.host_file.path, &disk_path, preset, &item.host_file)
                        .and_then(|()| {
                            let written_size = fs::metadata(&disk_path)?.len();
                            let meta = transcode::inspect_symphonia_metadata(&disk_path, written_size)
                                .unwrap_or_default();
                            let dur = if meta.duration_ms > 0 {
                                meta.duration_ms
                            } else {
                                item.host_file.duration_ms.max(1000)
                            };
                            let br = if meta.bitrate_kbps > 0 {
                                meta.bitrate_kbps
                            } else {
                                ((written_size * 8) / (dur as u64)).max(32) as u32
                            };
                            let sr = if meta.sample_rate > 0 {
                                meta.sample_rate
                            } else {
                                44_100
                            };
                            Ok(transcode::TranscodedOutputInfo {
                                file_size: written_size,
                                duration_ms: dur,
                                bitrate_kbps: br,
                                sample_rate: sr,
                            })
                        }),
                };

                match transcode_result {
                    Ok(info) => {
                        db_host_file.format = match preset {
                            TranscodePreset::Alac => "ALAC".to_string(),
                            TranscodePreset::Aac => "AAC".to_string(),
                            TranscodePreset::Mp3 => "MP3".to_string(),
                        };
                        db_host_file.file_size = info.file_size;
                        db_host_file.duration_ms = info.duration_ms;
                        db_host_file.bitrate_kbps = info.bitrate_kbps;
                        track_sample_rate_hz = Some(info.sample_rate);
                    }
                    Err(err) => {
                        if disk_path.is_file() {
                            created_disk_paths.push(disk_path);
                        }
                        interruption_reason = Some(err.to_string());
                        break;
                    }
                }
            } else if let Err(err) = copy_file_bytes(&item.host_file.path, &disk_path) {
                if disk_path.is_file() {
                    created_disk_paths.push(disk_path);
                }
                interruption_reason = Some(err.to_string());
                break;
            }

            on_progress(TransferProgress {
                current_track_index: idx + 1,
                total_tracks: total_active,
                current_track_title: item.host_file.title.clone(),
                is_transcoding: item.requires_transcode,
                track_percent: 100,
                overall_percent: if total_active > 0 {
                    (((idx + 1) * 100) / total_active).min(100) as u8
                } else {
                    100
                },
            });

            let replace_id = if item.is_duplicate
                && item.duplicate_resolution == DuplicateResolution::ReplaceExisting
            {
                item.matched_device_track_id
            } else {
                None
            };

            let (_track_id, old_obfuscated) = parsed_db.upsert_transferred_track(
                &db_host_file,
                &obfuscated_path,
                replace_id,
                item.target_playlist_id,
                track_sample_rate_hz,
            );
            if let Some(old_obf) = old_obfuscated {
                let old_disk = obfuscated_to_disk_path(&mount_path, &old_obf);
                if old_disk != disk_path {
                    old_paths_to_remove.push(old_disk);
                }
            }

            completed_staged_ids.push(item.staged_id);
            created_disk_paths.push(disk_path);
            written_obfuscated_paths.push(obfuscated_path);
        }

        if let Some(reason) = interruption_reason {
            let summary = InterruptedBatchSummary {
                completed_track_count: completed_staged_ids.len(),
                remaining_track_count: total_active.saturating_sub(completed_staged_ids.len()),
                written_obfuscated_paths,
                reason,
            };
            self.state.borrow_mut().interrupted_journal = Some(InterruptedJournal {
                summary: summary.clone(),
                mount_path,
                original_db_bytes,
                partial_db: parsed_db,
                completed_staged_ids,
                created_disk_paths,
                old_paths_to_remove,
                firewire_guid: current.device.firewire_guid,
            });
            return Ok(CommitOutcome::Interrupted(summary));
        }

        // Apply staged metadata edits
        let staged_edits = self.state.borrow().sync_batch.edits.clone();
        for edit in &staged_edits {
            if let Ok(Some((obfuscated_path, updated_track))) =
                parsed_db.update_track_metadata(edit.track_id, &edit.edits)
            {
                let disk_path = obfuscated_to_disk_path(&mount_path, &obfuscated_path);
                if disk_path.is_file() {
                    let _ = tagging::update_audio_file_tags(&disk_path, &edit.edits, &updated_track);
                }
            }
        }

        // Apply staged track removals
        let staged_removals = self.state.borrow().sync_batch.removals.clone();
        if !staged_removals.is_empty() {
            let removal_ids: Vec<u32> = staged_removals.iter().map(|r| r.track_id).collect();
            let paths_to_delete = parsed_db.remove_tracks(&removal_ids)?;
            for obf in paths_to_delete {
                let disk_path = obfuscated_to_disk_path(&mount_path, &obf);
                old_paths_to_remove.push(disk_path);
            }
        }

        // Apply staged orphan recoveries into Master Playlist
        let staged_recoveries = self.state.borrow().sync_batch.orphan_recoveries.clone();
        for rec in &staged_recoveries {
            let hf = HostFile {
                path: rec.orphan_file.path.clone(),
                file_name: rec.orphan_file.file_name.clone(),
                format: rec.orphan_file.format.clone(),
                file_size: rec.orphan_file.file_size,
                title: rec.orphan_file.title.clone(),
                artist: rec.orphan_file.artist.clone(),
                album: rec.orphan_file.album.clone(),
                track_number: rec.orphan_file.track_number,
                disc_number: rec.orphan_file.disc_number,
                genre: rec.orphan_file.genre.clone(),
                year: rec.orphan_file.year,
                duration_ms: rec.orphan_file.duration_ms,
                bitrate_kbps: rec.orphan_file.bitrate_kbps,
            };
            parsed_db.upsert_transferred_track(
                &hf,
                &rec.orphan_file.obfuscated_path,
                None,
                None,
                None,
            );
        }

        // Apply staged orphan deletions
        let staged_deletions = self.state.borrow().sync_batch.orphan_deletions.clone();
        for del in &staged_deletions {
            old_paths_to_remove.push(del.orphan_file.path.clone());
        }

        finalize_itunesdb_commit(
            &mount_path,
            &original_db_bytes,
            &mut parsed_db,
            current.device.firewire_guid.as_deref(),
            &old_paths_to_remove,
        )?;

        {
            let mut st = self.state.borrow_mut();
            st.sync_batch = SyncBatch::default();
            st.interrupted_journal = None;
        }

        match self.scan_device()? {
            DeviceConnectionState::Connected(updated_session) => {
                Ok(CommitOutcome::Completed(updated_session))
            }
            _ => Err(PodLoaderError::NoActiveDevice),
        }
    }

    pub fn set_manual_firewire_guid(
        &mut self,
        guid: &str,
    ) -> Result<DeviceSession, PodLoaderError> {
        let _ = parse_firewire_guid_bytes(guid)?;
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };
        let sysinfo_path = current
            .device
            .mount_path
            .join("iPod_Control")
            .join("Device")
            .join("SysInfo");
        cache_guid_in_sysinfo(&sysinfo_path, guid)?;

        match self.scan_device()? {
            DeviceConnectionState::Connected(updated) => Ok(updated),
            _ => Err(PodLoaderError::NoActiveDevice),
        }
    }

    pub fn resolve_interrupted_batch(
        &mut self,
        resolution: InterruptedResolution,
    ) -> Result<DeviceSession, PodLoaderError> {
        let journal = self
            .state
            .borrow_mut()
            .interrupted_journal
            .take()
            .ok_or(PodLoaderError::NoInterruptedBatch)?;

        match resolution {
            InterruptedResolution::RollBack => {
                for disk_path in journal.created_disk_paths {
                    if disk_path.is_file() {
                        let _ = fs::remove_file(disk_path);
                    }
                }
            }
            InterruptedResolution::CommitCompleted => {
                let mut partial_db = journal.partial_db;
                finalize_itunesdb_commit(
                    &journal.mount_path,
                    &journal.original_db_bytes,
                    &mut partial_db,
                    journal.firewire_guid.as_deref(),
                    &journal.old_paths_to_remove,
                )?;
                let mut st = self.state.borrow_mut();
                st.sync_batch
                    .additions
                    .retain(|a| !journal.completed_staged_ids.contains(&a.staged_id));
                recalculate_required_bytes(&mut st.sync_batch);
            }
        }

        match self.scan_device()? {
            DeviceConnectionState::Connected(updated) => Ok(updated),
            _ => Err(PodLoaderError::NoActiveDevice),
        }
    }

    fn load_connected_session(
        &self,
        vol: MountedVolume,
    ) -> Result<DeviceConnectionState, PodLoaderError> {
        let sysinfo_path = vol
            .mount_path
            .join("iPod_Control")
            .join("Device")
            .join("SysInfo");

        let (mut sysinfo_guid, model_number) = parse_sysinfo(&sysinfo_path);
        if sysinfo_guid.is_none() {
            if let Some(usb_guid) = self
                .adapter
                .lookup_usb_firewire_guid(&vol.mount_path)
                .or_else(|| vol.usb_firewire_guid.clone())
            {
                let normalized = normalize_firewire_guid(&usb_guid);
                let _ = cache_guid_in_sysinfo(&sysinfo_path, &normalized);
                sysinfo_guid = Some(normalized);
            }
        }

        let itunesdb_path = vol
            .mount_path
            .join("iPod_Control")
            .join("iTunes")
            .join("iTunesDB");
        let db_bytes = fs::read(&itunesdb_path)?;
        let mut parsed_db = ItunesDb::parse(&db_bytes)?;

        let play_counts_path = vol
            .mount_path
            .join("iPod_Control")
            .join("iTunes")
            .join("Play Counts");
        let mut library = parsed_db.to_device_library();
        let has_pending_play_counts = if play_counts_path.is_file() {
            let pc_bytes = fs::read(&play_counts_path)?;
            let entries = parse_play_counts(&pc_bytes)?;
            parsed_db.merge_play_counts(&entries);
            merge_play_counts_into_tracks(&mut library.tracks, &entries);
            true
        } else {
            false
        };

        let master_name = library
            .playlists
            .iter()
            .find(|p| p.kind == PlaylistKind::Master)
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| vol.volume_name.clone());

        let used_bytes = vol.total_bytes.saturating_sub(vol.free_bytes);

        let (sync_batch, interrupted_batch, transcode_preset) = {
            let mut st = self.state.borrow_mut();
            if st.active_mount_path.as_ref() != Some(&vol.mount_path) {
                st.active_mount_path = Some(vol.mount_path.clone());
                st.sync_batch = SyncBatch::default();
                st.interrupted_journal = None;
                st.transcode_preset = TranscodePreset::Alac;
            }
            (
                st.sync_batch.clone(),
                st.interrupted_journal.as_ref().map(|j| j.summary.clone()),
                st.transcode_preset,
            )
        };

        let orphan_files = scan_orphan_files(&vol.mount_path, &library);

        let session = DeviceSession {
            device: DeviceMetadata {
                mount_path: vol.mount_path,
                name: master_name,
                model_number,
                firewire_guid: sysinfo_guid,
                total_bytes: vol.total_bytes,
                free_bytes: vol.free_bytes,
                used_bytes,
            },
            library,
            has_pending_play_counts,
            sync_batch,
            interrupted_batch,
            transcode_preset,
            ffmpeg_available: self.adapter.is_ffmpeg_available(),
            orphan_files,
        };
        self.state.borrow_mut().cached_session = Some(session.clone());
        Ok(DeviceConnectionState::Connected(session))
    }

    fn mutate_itunesdb<F, R>(&mut self, f: F) -> Result<R, PodLoaderError>
    where
        F: FnOnce(&mut ItunesDb) -> Result<R, PodLoaderError>,
    {
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };
        let mount_path = current.device.mount_path;
        let itunesdb_path = mount_path
            .join("iPod_Control")
            .join("iTunes")
            .join("iTunesDB");
        let original_db_bytes = fs::read(&itunesdb_path)?;
        let mut parsed_db = ItunesDb::parse(&original_db_bytes)?;
        let result = f(&mut parsed_db)?;
        finalize_itunesdb_commit(
            &mount_path,
            &original_db_bytes,
            &mut parsed_db,
            current.device.firewire_guid.as_deref(),
            &[],
        )?;
        let _ = self.scan_device()?;
        Ok(result)
    }

    pub fn create_device_playlist(&mut self, name: &str) -> Result<DevicePlaylist, PodLoaderError> {
        self.mutate_itunesdb(|db| db.create_user_playlist(name))
    }

    pub fn add_tracks_to_playlist(
        &mut self,
        playlist_id: u64,
        track_ids: &[u32],
    ) -> Result<DevicePlaylist, PodLoaderError> {
        self.mutate_itunesdb(|db| db.add_tracks_to_playlist(playlist_id, track_ids))
    }

    pub fn reorder_playlist_tracks(
        &mut self,
        playlist_id: u64,
        track_ids: &[u32],
    ) -> Result<DevicePlaylist, PodLoaderError> {
        self.mutate_itunesdb(|db| db.reorder_playlist_tracks(playlist_id, track_ids))
    }

    pub fn remove_tracks_from_playlist(
        &mut self,
        playlist_id: u64,
        track_ids: &[u32],
    ) -> Result<DevicePlaylist, PodLoaderError> {
        self.mutate_itunesdb(|db| db.remove_tracks_from_playlist(playlist_id, track_ids))
    }

    pub fn rename_device_playlist(
        &mut self,
        playlist_id: u64,
        new_name: &str,
    ) -> Result<DevicePlaylist, PodLoaderError> {
        self.mutate_itunesdb(|db| db.rename_device_playlist(playlist_id, new_name))
    }

    pub fn delete_device_playlist(&mut self, playlist_id: u64) -> Result<(), PodLoaderError> {
        self.mutate_itunesdb(|db| db.delete_device_playlist(playlist_id))
    }

    pub fn export_device_tracks(
        &self,
        track_ids: &[u32],
        destination_dir: &Path,
    ) -> Result<ExportResult, PodLoaderError> {
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };
        let mut exported_files = Vec::new();
        for &tid in track_ids {
            let Some(track) = current.library.tracks.iter().find(|t| t.id == tid) else {
                continue;
            };
            let source_disk_path =
                obfuscated_to_disk_path(&current.device.mount_path, &track.obfuscated_path);
            if !source_disk_path.is_file() {
                continue;
            }
            let ext = source_disk_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp3");
            let artist_dir = sanitize_filename_segment(&track.artist);
            let album_dir = sanitize_filename_segment(&track.album);
            let title_part = sanitize_filename_segment(&track.title);
            let file_name = format!("{:02} - {}.{}", track.track_number, title_part, ext);
            let target_dir = destination_dir.join(&artist_dir).join(&album_dir);
            fs::create_dir_all(&target_dir)?;
            let target_path = target_dir.join(&file_name);
            copy_file_bytes(&source_disk_path, &target_path)?;
            exported_files.push(target_path);
        }
        Ok(ExportResult {
            exported_count: exported_files.len(),
            exported_files,
        })
    }

    pub fn export_device_playlist(
        &self,
        playlist_id: u64,
        destination_dir: &Path,
    ) -> Result<ExportResult, PodLoaderError> {
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };
        let Some(playlist) = current
            .library
            .playlists
            .iter()
            .find(|p| p.id == playlist_id)
        else {
            return Err(PodLoaderError::DeviceOperationFailed(format!(
                "Playlist {} not found",
                playlist_id
            )));
        };
        self.export_device_tracks(&playlist.track_ids, destination_dir)
    }

    pub fn has_uncommitted_changes(&self) -> bool {
        let st = self.state.borrow();
        st.sync_batch.additions.iter().any(|a| a.should_transfer())
            || !st.sync_batch.edits.is_empty()
            || !st.sync_batch.removals.is_empty()
            || !st.sync_batch.orphan_recoveries.is_empty()
            || !st.sync_batch.orphan_deletions.is_empty()
    }

    pub fn eject_device(&mut self, force: bool) -> Result<(), PodLoaderError> {
        let current = match self.scan_device()? {
            DeviceConnectionState::Connected(session) => session,
            _ => return Err(PodLoaderError::NoActiveDevice),
        };
        if !force && self.has_uncommitted_changes() {
            return Err(PodLoaderError::UncommittedChangesPending);
        }
        let is_device_playing = self
            .active_track
            .borrow()
            .as_ref()
            .map(|a| a.source_kind == AudioSourceKind::Device)
            .unwrap_or(false);
        if is_device_playing {
            let _ = self.stop_playback();
        }
        let mount_path = current.device.mount_path.clone();
        self.adapter.eject_volume(&mount_path)?;

        let mut st = self.state.borrow_mut();
        st.ejected_mount_paths.push(mount_path);
        st.active_mount_path = None;
        st.cached_session = None;
        st.sync_batch = SyncBatch::default();
        st.interrupted_journal = None;
        Ok(())
    }

    pub fn get_playback_status(&self) -> PlaybackStatus {
        let sink = self.audio_sink.borrow();
        let track = self.active_track.borrow().clone();
        let state = if sink.is_playing() {
            PlaybackState::Playing
        } else if sink.is_paused() && track.is_some() {
            PlaybackState::Paused
        } else {
            PlaybackState::Stopped
        };
        let elapsed_ms = sink.get_position_ms();
        let duration_ms = track.as_ref().map(|t| t.duration_ms).unwrap_or(0);
        let volume = sink.get_volume();
        PlaybackStatus {
            state,
            track,
            elapsed_ms,
            duration_ms,
            volume,
        }
    }

    pub fn play_host_file(&self, host_file: &HostFile) -> Result<PlaybackStatus, PodLoaderError> {
        let mut sink = self.audio_sink.borrow_mut();
        let dur = sink.play_source_with_duration(&host_file.path, 0, Some(host_file.duration_ms))?;
        let active = ActivePlaybackTrack {
            source_kind: AudioSourceKind::Host,
            source_path: host_file.path.clone(),
            title: host_file.title.clone(),
            artist: host_file.artist.clone(),
            album: host_file.album.clone(),
            duration_ms: if dur > 0 { dur } else { host_file.duration_ms },
            track_id: None,
        };
        *self.active_track.borrow_mut() = Some(active.clone());
        Ok(PlaybackStatus {
            state: PlaybackState::Playing,
            track: Some(active),
            elapsed_ms: 0,
            duration_ms: dur,
            volume: sink.get_volume(),
        })
    }

    pub fn get_cached_session(&self) -> Option<DeviceSession> {
        self.state.borrow().cached_session.clone()
    }

    pub fn get_device_track_path(&self, track_id: u32) -> Result<PathBuf, PodLoaderError> {
        let (mount_path, track) = {
            let st = self.state.borrow();
            if let Some(ref session) = st.cached_session {
                if let Some(t) = session.library.tracks.iter().find(|tr| tr.id == track_id) {
                    (session.device.mount_path.clone(), t.clone())
                } else {
                    drop(st);
                    let current = match self.scan_device()? {
                        DeviceConnectionState::Connected(session) => session,
                        _ => return Err(PodLoaderError::NoActiveDevice),
                    };
                    let t = current
                        .library
                        .tracks
                        .iter()
                        .find(|tr| tr.id == track_id)
                        .ok_or_else(|| {
                            PodLoaderError::DeviceOperationFailed(format!("Track {} not found", track_id))
                        })?
                        .clone();
                    (current.device.mount_path.clone(), t)
                }
            } else {
                drop(st);
                let current = match self.scan_device()? {
                    DeviceConnectionState::Connected(session) => session,
                    _ => return Err(PodLoaderError::NoActiveDevice),
                };
                let t = current
                    .library
                    .tracks
                    .iter()
                    .find(|tr| tr.id == track_id)
                    .ok_or_else(|| {
                        PodLoaderError::DeviceOperationFailed(format!("Track {} not found", track_id))
                    })?
                    .clone();
                (current.device.mount_path.clone(), t)
            }
        };

        let disk_path = obfuscated_to_disk_path(&mount_path, &track.obfuscated_path);
        if !disk_path.is_file() {
            return Err(PodLoaderError::DeviceOperationFailed(format!(
                "Audio file at {:?} not found",
                disk_path
            )));
        }
        Ok(disk_path)
    }

    pub fn play_device_track(&self, track_id: u32) -> Result<PlaybackStatus, PodLoaderError> {
        let (mount_path, track) = {
            let st = self.state.borrow();
            if let Some(ref session) = st.cached_session {
                if let Some(t) = session.library.tracks.iter().find(|tr| tr.id == track_id) {
                    (session.device.mount_path.clone(), t.clone())
                } else {
                    drop(st);
                    let current = match self.scan_device()? {
                        DeviceConnectionState::Connected(session) => session,
                        _ => return Err(PodLoaderError::NoActiveDevice),
                    };
                    let t = current
                        .library
                        .tracks
                        .iter()
                        .find(|tr| tr.id == track_id)
                        .ok_or_else(|| {
                            PodLoaderError::DeviceOperationFailed(format!("Track {} not found", track_id))
                        })?
                        .clone();
                    (current.device.mount_path.clone(), t)
                }
            } else {
                drop(st);
                let current = match self.scan_device()? {
                    DeviceConnectionState::Connected(session) => session,
                    _ => return Err(PodLoaderError::NoActiveDevice),
                };
                let t = current
                    .library
                    .tracks
                    .iter()
                    .find(|tr| tr.id == track_id)
                    .ok_or_else(|| {
                        PodLoaderError::DeviceOperationFailed(format!("Track {} not found", track_id))
                    })?
                    .clone();
                (current.device.mount_path.clone(), t)
            }
        };

        let disk_path = obfuscated_to_disk_path(&mount_path, &track.obfuscated_path);
        if !disk_path.is_file() {
            return Err(PodLoaderError::DeviceOperationFailed(format!(
                "Audio file at {:?} not found",
                disk_path
            )));
        }
        let mut sink = self.audio_sink.borrow_mut();
        let dur = sink.play_source_with_duration(&disk_path, 0, Some(track.duration_ms))?;
        let active = ActivePlaybackTrack {
            source_kind: AudioSourceKind::Device,
            source_path: disk_path,
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_ms: if dur > 0 { dur } else { track.duration_ms },
            track_id: Some(track_id),
        };
        *self.active_track.borrow_mut() = Some(active.clone());
        Ok(PlaybackStatus {
            state: PlaybackState::Playing,
            track: Some(active),
            elapsed_ms: 0,
            duration_ms: dur,
            volume: sink.get_volume(),
        })
    }

    pub fn pause_playback(&self) -> Result<PlaybackStatus, PodLoaderError> {
        self.audio_sink.borrow_mut().pause()?;
        Ok(self.get_playback_status())
    }

    pub fn resume_playback(&self) -> Result<PlaybackStatus, PodLoaderError> {
        self.audio_sink.borrow_mut().resume()?;
        Ok(self.get_playback_status())
    }

    pub fn stop_playback(&self) -> Result<PlaybackStatus, PodLoaderError> {
        self.audio_sink.borrow_mut().stop()?;
        *self.active_track.borrow_mut() = None;
        Ok(self.get_playback_status())
    }

    pub fn seek_playback(&self, position_ms: u32) -> Result<PlaybackStatus, PodLoaderError> {
        self.audio_sink.borrow_mut().seek(position_ms)?;
        Ok(self.get_playback_status())
    }

    pub fn set_playback_volume(&self, volume: f32) -> Result<PlaybackStatus, PodLoaderError> {
        self.audio_sink.borrow_mut().set_volume(volume)?;
        Ok(self.get_playback_status())
    }
}

fn finalize_itunesdb_commit(
    mount_path: &Path,
    original_db_bytes: &[u8],
    parsed_db: &mut ItunesDb,
    firewire_guid: Option<&str>,
    old_paths_to_remove: &[PathBuf],
) -> Result<(), PodLoaderError> {
    let itunes_dir = mount_path.join("iPod_Control").join("iTunes");
    let itunesdb_path = itunes_dir.join("iTunesDB");
    let play_counts_path = itunes_dir.join("Play Counts");
    let bak_path = itunes_dir.join("iTunesDB.bak");

    let extracted_72 = extract_hash72_params(original_db_bytes).or_else(|| {
        fs::read(&bak_path)
            .ok()
            .and_then(|bak| extract_hash72_params(&bak))
    });

    fs::write(&bak_path, original_db_bytes)?;

    if firewire_guid.is_some() {
        parsed_db.ensure_mhbd_header_min_len(244);
    }
    let mut new_db_bytes = parsed_db.to_bytes();
    if let Some((iv, rand12)) = extracted_72 {
        resign_itunesdb_hash72_with_params(&mut new_db_bytes, &iv, &rand12);
    }
    if let Some(guid) = firewire_guid {
        sign_itunesdb(&mut new_db_bytes, guid)?;
    }
    atomic_write_file(&itunesdb_path, &new_db_bytes)?;

    if play_counts_path.is_file() {
        let _ = fs::remove_file(&play_counts_path);
    }
    for old_file in old_paths_to_remove {
        if old_file.is_file() {
            let _ = fs::remove_file(old_file);
        }
    }
    Ok(())
}

pub fn scan_orphan_files(mount_path: &Path, library: &DeviceLibrary) -> Vec<OrphanFile> {
    let music_root = mount_path.join("iPod_Control").join("Music");
    if !music_root.is_dir() {
        return Vec::new();
    }
    let mut orphans = Vec::new();
    for idx in 0..50 {
        let folder_name = format!("F{:02}", idx);
        let folder_path = music_root.join(&folder_name);
        if let Ok(entries) = fs::read_dir(&folder_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && is_supported_audio_path(&path) {
                    let file_name = entry.file_name().to_string_lossy().to_string();
                    let obfuscated_path = format!(":iPod_Control:Music:{}:{}", folder_name, file_name);
                    let is_indexed = library
                        .tracks
                        .iter()
                        .any(|t| t.obfuscated_path.eq_ignore_ascii_case(&obfuscated_path));
                    if !is_indexed {
                        if let Ok(hf) = host::inspect_host_file(&path) {
                            orphans.push(OrphanFile {
                                path,
                                obfuscated_path,
                                file_name: hf.file_name,
                                file_size: hf.file_size,
                                format: hf.format,
                                title: hf.title,
                                artist: hf.artist,
                                album: hf.album,
                                track_number: hf.track_number,
                                disc_number: hf.disc_number,
                                genre: hf.genre,
                                year: hf.year,
                                duration_ms: hf.duration_ms,
                                bitrate_kbps: hf.bitrate_kbps,
                            });
                        }
                    }
                }
            }
        }
    }
    orphans.sort_by(|a, b| a.obfuscated_path.cmp(&b.obfuscated_path));
    orphans
}

pub fn sanitize_filename_segment(val: &str) -> String {
    let sanitized: String = val
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    let trimmed = sanitized.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "Unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_tag_key(val: &str) -> String {
    val.trim().to_lowercase()
}

fn recalculate_required_bytes(batch: &mut SyncBatch) {
    batch.required_bytes = batch
        .additions
        .iter()
        .filter(|a| a.should_transfer())
        .map(|a| a.host_file.file_size)
        .sum();
}

fn allocate_obfuscated_path(
    mount_path: &Path,
    source_path: &Path,
    override_ext: Option<&str>,
    counter: &mut u64,
) -> Result<(PathBuf, String), PodLoaderError> {
    let ext = override_ext
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_else(|| {
            source_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp3")
                .to_ascii_lowercase()
        });

    let time_entropy = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);

    let mut path_hash = 0xcbf29ce484222325u64;
    for b in source_path.to_string_lossy().as_bytes() {
        path_hash ^= *b as u64;
        path_hash = path_hash.wrapping_mul(0x100000001b3);
    }

    loop {
        let n = *counter;
        *counter += 1;

        let mut z = time_entropy
            .wrapping_add(path_hash)
            .wrapping_add(n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;

        let folder_idx = (z % 50) as u32;
        let folder_name = format!("F{:02}", folder_idx);

        let mut code = z >> 8;
        let mut letters = [b'A'; 4];
        for slot in &mut letters {
            *slot = b'A' + ((code % 26) as u8);
            code /= 26;
        }
        let stem = std::str::from_utf8(&letters).unwrap_or("AAAA");
        let file_name = format!("{}.{}", stem, ext);
        let disk_path = mount_path
            .join("iPod_Control")
            .join("Music")
            .join(&folder_name)
            .join(&file_name);

        if !disk_path.exists() {
            let obfuscated = format!(":iPod_Control:Music:{}:{}", folder_name, file_name);
            return Ok((disk_path, obfuscated));
        }
    }
}

fn obfuscated_to_disk_path(mount_path: &Path, obfuscated_path: &str) -> PathBuf {
    let rel = obfuscated_path.trim_start_matches(':').replace(':', "/");
    mount_path.join(rel)
}

fn copy_file_bytes(src: &Path, dst: &Path) -> std::io::Result<u64> {
    let mut reader = fs::File::open(src)?;
    let mut writer = fs::File::create(dst)?;
    let copied = std::io::copy(&mut reader, &mut writer)?;
    let _ = writer.sync_all();
    Ok(copied)
}

fn atomic_write_file(target_path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp_path = target_path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(bytes)?;
        let _ = file.sync_all();
    }
    if fs::rename(&tmp_path, target_path).is_err() {
        fs::write(target_path, bytes)?;
        let _ = fs::remove_file(&tmp_path);
    }
    Ok(())
}

fn merge_edits(target: &mut TrackMetadataEdits, incoming: &TrackMetadataEdits) {
    if incoming.title.is_some() {
        target.title = incoming.title.clone();
    }
    if incoming.artist.is_some() {
        target.artist = incoming.artist.clone();
    }
    if incoming.album.is_some() {
        target.album = incoming.album.clone();
    }
    if incoming.album_artist.is_some() {
        target.album_artist = incoming.album_artist.clone();
    }
    if incoming.track_number.is_some() {
        target.track_number = incoming.track_number;
    }
    if incoming.disc_number.is_some() {
        target.disc_number = incoming.disc_number;
    }
    if incoming.genre.is_some() {
        target.genre = incoming.genre.clone();
    }
    if incoming.year.is_some() {
        target.year = incoming.year;
    }
    if incoming.is_compilation.is_some() {
        target.is_compilation = incoming.is_compilation;
    }
    if incoming.rating.is_some() {
        target.rating = incoming.rating;
    }
}
