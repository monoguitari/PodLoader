//! Core domain engine for PodLoader (`ADR-0001`, `ADR-0004`, `ADR-0006`).

pub mod host;
pub mod itunesdb;
pub mod playcounts;
pub mod signing;
pub mod sysinfo;
pub mod transcode;

use std::cell::RefCell;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::host::{
    collect_host_files_from_paths, is_native_ipod_format, is_transcodable_format,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDirectoryEntry {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct SyncBatch {
    pub additions: Vec<StagedTrackAddition>,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "camelCase")]
pub enum DeviceConnectionState {
    Disconnected,
    UninitializedVolume(MountedVolume),
    Connected(DeviceSession),
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
    sync_batch: SyncBatch,
    next_staged_id: u64,
    next_obfuscated_counter: u64,
    interrupted_journal: Option<InterruptedJournal>,
    transcode_preset: TranscodePreset,
}

pub struct PodLoaderEngine<A: HardwareDeviceAdapter> {
    adapter: A,
    state: RefCell<EngineState>,
}

impl<A: HardwareDeviceAdapter> PodLoaderEngine<A> {
    pub fn new(adapter: A) -> Self {
        Self {
            adapter,
            state: RefCell::new(EngineState {
                active_mount_path: None,
                sync_batch: SyncBatch::default(),
                next_staged_id: 1,
                next_obfuscated_counter: 1,
                interrupted_journal: None,
                transcode_preset: TranscodePreset::Alac,
            }),
        }
    }

    pub fn scan_device(&self) -> Result<DeviceConnectionState, PodLoaderError> {
        let volumes = self.adapter.list_mounted_volumes()?;
        let Some(vol) = volumes.into_iter().next() else {
            self.state.borrow_mut().active_mount_path = None;
            return Ok(DeviceConnectionState::Disconnected);
        };

        let itunesdb_path = vol
            .mount_path
            .join("iPod_Control")
            .join("iTunes")
            .join("iTunesDB");

        if !itunesdb_path.is_file() {
            self.state.borrow_mut().active_mount_path = None;
            return Ok(DeviceConnectionState::UninitializedVolume(vol));
        }

        self.load_connected_session(vol)
    }

    pub fn initialize_device(
        &self,
        mount_path: &Path,
        device_name: &str,
    ) -> Result<DeviceConnectionState, PodLoaderError> {
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

        Ok(DeviceConnectionState::Connected(DeviceSession {
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
        }))
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
