//! Tauri v2 IPC command adapter around `PodLoaderEngine` (`ADR-0001`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use podloader_core::{
    CommitOutcome, DeviceConnectionState, DevicePlaylist, DeviceSession, DuplicateResolution,
    ExportResult, HardwareDeviceAdapter, HostDirectoryListing, HostFile, InterruptedResolution,
    PlaybackStatus, PodLoaderEngine, SyncBatch, SystemHardwareDeviceAdapter, TrackMetadataEdits,
    TranscodePreset,
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
        self.commit_sync_batch_with_progress(max_tracks_before_interrupt, |_| {})
    }

    pub fn commit_sync_batch_with_progress<F>(
        &self,
        max_tracks_before_interrupt: Option<usize>,
        on_progress: F,
    ) -> Result<CommitOutcome, String>
    where
        F: FnMut(podloader_core::TransferProgress),
    {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .commit_sync_batch_with_progress(max_tracks_before_interrupt, on_progress)
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

    pub fn stage_track_metadata_edits(
        &self,
        track_ids: &[u32],
        edits: TrackMetadataEdits,
    ) -> Result<SyncBatch, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .stage_track_metadata_edits(track_ids, edits)
            .map_err(|e| e.to_string())
    }

    pub fn stage_track_removals(&self, track_ids: &[u32]) -> Result<SyncBatch, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .stage_track_removals(track_ids)
            .map_err(|e| e.to_string())
    }

    pub fn create_device_playlist(&self, name: &str) -> Result<DevicePlaylist, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .create_device_playlist(name)
            .map_err(|e| e.to_string())
    }

    pub fn add_tracks_to_playlist(
        &self,
        playlist_id: u64,
        track_ids: &[u32],
    ) -> Result<DevicePlaylist, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .add_tracks_to_playlist(playlist_id, track_ids)
            .map_err(|e| e.to_string())
    }

    pub fn reorder_playlist_tracks(
        &self,
        playlist_id: u64,
        track_ids: &[u32],
    ) -> Result<DevicePlaylist, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .reorder_playlist_tracks(playlist_id, track_ids)
            .map_err(|e| e.to_string())
    }

    pub fn remove_tracks_from_playlist(
        &self,
        playlist_id: u64,
        track_ids: &[u32],
    ) -> Result<DevicePlaylist, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .remove_tracks_from_playlist(playlist_id, track_ids)
            .map_err(|e| e.to_string())
    }

    pub fn rename_device_playlist(
        &self,
        playlist_id: u64,
        new_name: &str,
    ) -> Result<DevicePlaylist, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .rename_device_playlist(playlist_id, new_name)
            .map_err(|e| e.to_string())
    }

    pub fn delete_device_playlist(&self, playlist_id: u64) -> Result<(), String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .delete_device_playlist(playlist_id)
            .map_err(|e| e.to_string())
    }

    pub fn stage_orphan_recovery(&self, obfuscated_path: &str) -> Result<SyncBatch, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .stage_orphan_recovery(obfuscated_path)
            .map_err(|e| e.to_string())
    }

    pub fn stage_orphan_deletion(&self, obfuscated_path: &str) -> Result<SyncBatch, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .stage_orphan_deletion(obfuscated_path)
            .map_err(|e| e.to_string())
    }

    pub fn export_device_tracks(
        &self,
        track_ids: Vec<u32>,
        destination_dir: &str,
    ) -> Result<ExportResult, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .export_device_tracks(&track_ids, Path::new(destination_dir))
            .map_err(|e| e.to_string())
    }

    pub fn export_device_playlist(
        &self,
        playlist_id: u64,
        destination_dir: &str,
    ) -> Result<ExportResult, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard
            .export_device_playlist(playlist_id, Path::new(destination_dir))
            .map_err(|e| e.to_string())
    }

    pub fn eject_device(&self, force: bool) -> Result<(), String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard.eject_device(force).map_err(|e| e.to_string())
    }

    pub fn play_host_file(&self, host_file: HostFile) -> Result<PlaybackStatus, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard.play_host_file(&host_file).map_err(|e| e.to_string())
    }

    pub fn get_cached_session(&self) -> Option<DeviceSession> {
        let guard = self.engine.lock().ok()?;
        guard.get_cached_session()
    }

    pub fn get_device_track_path(&self, track_id: u32) -> Result<PathBuf, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard.get_device_track_path(track_id).map_err(|e| e.to_string())
    }

    pub fn play_device_track(&self, track_id: u32) -> Result<PlaybackStatus, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard.play_device_track(track_id).map_err(|e| e.to_string())
    }

    pub fn pause_playback(&self) -> Result<PlaybackStatus, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard.pause_playback().map_err(|e| e.to_string())
    }

    pub fn resume_playback(&self) -> Result<PlaybackStatus, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard.resume_playback().map_err(|e| e.to_string())
    }

    pub fn stop_playback(&self) -> Result<PlaybackStatus, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard.stop_playback().map_err(|e| e.to_string())
    }

    pub fn seek_playback(&self, position_ms: u32) -> Result<PlaybackStatus, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard.seek_playback(position_ms).map_err(|e| e.to_string())
    }

    pub fn set_playback_volume(&self, volume: f32) -> Result<PlaybackStatus, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        guard.set_playback_volume(volume).map_err(|e| e.to_string())
    }

    pub fn get_playback_status(&self) -> Result<PlaybackStatus, String> {
        let guard = self.engine.lock().map_err(|e| e.to_string())?;
        Ok(guard.get_playback_status())
    }
}
