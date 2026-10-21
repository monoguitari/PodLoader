use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

use crate::PodLoaderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioSourceKind {
    Host,
    Device,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePlaybackTrack {
    pub source_kind: AudioSourceKind,
    pub source_path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
    pub track_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStatus {
    pub state: PlaybackState,
    pub track: Option<ActivePlaybackTrack>,
    pub elapsed_ms: u32,
    pub duration_ms: u32,
    pub volume: f32,
}

pub trait AudioSinkAdapter: Send + Sync {
    fn play_source(&mut self, source_path: &Path, start_position_ms: u32) -> Result<u32, PodLoaderError>;
    fn pause(&mut self) -> Result<(), PodLoaderError>;
    fn resume(&mut self) -> Result<(), PodLoaderError>;
    fn stop(&mut self) -> Result<(), PodLoaderError>;
    fn seek(&mut self, position_ms: u32) -> Result<(), PodLoaderError>;
    fn set_volume(&mut self, volume: f32) -> Result<(), PodLoaderError>;
    fn get_position_ms(&self) -> u32;
    fn is_playing(&self) -> bool;
    fn is_paused(&self) -> bool;
    fn get_volume(&self) -> f32;
}

#[derive(Debug, Clone)]
pub struct HeadlessAudioSinkAdapter {
    is_playing: bool,
    is_paused: bool,
    position_ms: u32,
    duration_ms: u32,
    volume: f32,
}

impl Default for HeadlessAudioSinkAdapter {
    fn default() -> Self {
        Self {
            is_playing: false,
            is_paused: false,
            position_ms: 0,
            duration_ms: 0,
            volume: 1.0,
        }
    }
}

impl AudioSinkAdapter for HeadlessAudioSinkAdapter {
    fn play_source(&mut self, source_path: &Path, start_position_ms: u32) -> Result<u32, PodLoaderError> {
        let dur = if let Ok(hf) = crate::host::inspect_host_file(source_path) {
            hf.duration_ms.max(1000)
        } else {
            180_000
        };
        self.duration_ms = dur;
        self.position_ms = start_position_ms.min(dur);
        self.is_playing = true;
        self.is_paused = false;
        Ok(dur)
    }

    fn pause(&mut self) -> Result<(), PodLoaderError> {
        if self.is_playing {
            self.is_playing = false;
            self.is_paused = true;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PodLoaderError> {
        if self.is_paused {
            self.is_playing = true;
            self.is_paused = false;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PodLoaderError> {
        self.is_playing = false;
        self.is_paused = false;
        self.position_ms = 0;
        Ok(())
    }

    fn seek(&mut self, position_ms: u32) -> Result<(), PodLoaderError> {
        self.position_ms = position_ms.min(self.duration_ms);
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), PodLoaderError> {
        self.volume = volume.clamp(0.0, 1.0);
        Ok(())
    }

    fn get_position_ms(&self) -> u32 {
        self.position_ms
    }

    fn is_playing(&self) -> bool {
        self.is_playing
    }

    fn is_paused(&self) -> bool {
        self.is_paused
    }

    fn get_volume(&self) -> f32 {
        self.volume
    }
}
