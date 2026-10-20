//! Tauri v2 IPC command adapter around `PodLoaderEngine` (`ADR-0001`).

use std::path::Path;
use std::sync::Mutex;
use podloader_core::{
    DeviceConnectionState, HardwareDeviceAdapter, PodLoaderEngine, SystemHardwareDeviceAdapter,
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
}
