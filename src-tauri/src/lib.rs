//! Tauri v2 IPC command adapter around `PodLoaderEngine` (`ADR-0001`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use podloader_core::{
    CommitOutcome, DeviceConnectionState, DeviceSession, DuplicateResolution,
    HardwareDeviceAdapter, HostDirectoryListing, InterruptedResolution, PodLoaderEngine,
    SyncBatch, SystemHardwareDeviceAdapter, TranscodePreset,
};

pub struct AppState<A: HardwareDeviceAdapter = SystemHardwareDeviceAdapter> {
    engine: Mutex<PodLoaderEngine<A>>,
}

impl Default for AppState<SystemHardwareDeviceAdapter> {
    fn default() -> Self {
        Self::new(SystemHardwareDeviceAdapter::default())
    }
}

impl<A: HardwareDeviceAdapter> AppState<A> {
    pub fn new(adapter: A) -> Self {
        Self {
            engine: Mutex::new(PodLoaderEngine::new(adapter)),
        }
    }

    pub fn scan_device(&self) -> Result<DeviceConnectionState, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard.scan_device().map_err(|e| e.to_string())
    }

    pub fn initialize_device(
        &self,
        mount_path: &str,
        device_name: &str,
    ) -> Result<DeviceConnectionState, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .initialize_device(Path::new(mount_path), device_name)
            .map_err(|e| e.to_string())
    }

    pub fn browse_host_directory(&self, dir_path: &str) -> Result<HostDirectoryListing, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .browse_host_directory(Path::new(dir_path))
            .map_err(|e| e.to_string())
    }

    pub fn stage_host_paths(
        &self,
        paths: Vec<String>,
        target_playlist_id: Option<u64>,
    ) -> Result<SyncBatch, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        let path_bufs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        guard
            .stage_host_paths(&path_bufs, target_playlist_id)
            .map_err(|e| e.to_string())
    }

    pub fn set_duplicate_resolution(
        &self,
        staged_id: u64,
        resolution: DuplicateResolution,
    ) -> Result<SyncBatch, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .set_duplicate_resolution(staged_id, resolution)
            .map_err(|e| e.to_string())
    }

    pub fn commit_sync_batch(
        &self,
        max_tracks_before_interrupt: Option<usize>,
    ) -> Result<CommitOutcome, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .commit_sync_batch_with_limit(max_tracks_before_interrupt)
            .map_err(|e| e.to_string())
    }

    pub fn resolve_interrupted_batch(
        &self,
        resolution: InterruptedResolution,
    ) -> Result<DeviceSession, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .resolve_interrupted_batch(resolution)
            .map_err(|e| e.to_string())
    }

    pub fn set_manual_firewire_guid(&self, guid: &str) -> Result<DeviceSession, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .set_manual_firewire_guid(guid)
            .map_err(|e| e.to_string())
    }

    pub fn set_transcode_preset(&self, preset: TranscodePreset) -> Result<DeviceSession, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .set_transcode_preset(preset)
            .map_err(|e| e.to_string())
    }
}
