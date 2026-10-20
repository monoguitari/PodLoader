//! Core domain engine for PodLoader (`ADR-0001`, `ADR-0004`, `ADR-0006`).

pub mod itunesdb;
pub mod playcounts;
pub mod sysinfo;

use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::itunesdb::{build_empty_itunesdb, ItunesDb};
use crate::playcounts::{merge_play_counts_into_tracks, parse_play_counts};
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
        let entries = fs::read_dir(usb_root).ok()?;
        for entry in entries.flatten() {
            let dir = entry.path();
            let vendor = fs::read_to_string(dir.join("idVendor")).ok()?;
            if vendor.trim().eq_ignore_ascii_case("05ac") {
                if let Ok(serial) = fs::read_to_string(dir.join("serial")) {
                    let trimmed = serial.trim();
                    if trimmed.len() >= 16 {
                        let candidate = &trimmed[trimmed.len() - 16..];
                        if candidate.chars().all(|c| c.is_ascii_hexdigit()) {
                            return Some(candidate.to_uppercase());
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSession {
    pub device: DeviceMetadata,
    pub library: DeviceLibrary,
    pub has_pending_play_counts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "camelCase")]
pub enum DeviceConnectionState {
    Disconnected,
    UninitializedVolume(MountedVolume),
    Connected(DeviceSession),
}

pub struct PodLoaderEngine<A: HardwareDeviceAdapter> {
    adapter: A,
}

impl<A: HardwareDeviceAdapter> PodLoaderEngine<A> {
    pub fn new(adapter: A) -> Self {
        Self { adapter }
    }

    pub fn scan_device(&self) -> Result<DeviceConnectionState, PodLoaderError> {
        let volumes = self.adapter.list_mounted_volumes()?;
        let Some(vol) = volumes.into_iter().next() else {
            return Ok(DeviceConnectionState::Disconnected);
        };

        let itunesdb_path = vol
            .mount_path
            .join("iPod_Control")
            .join("iTunes")
            .join("iTunesDB");

        if !itunesdb_path.is_file() {
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
        }))
    }
}
