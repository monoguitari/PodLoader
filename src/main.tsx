import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import type { DeviceConnectionState, PodLoaderBridge } from "./bridge";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
    };
  }
}

const defaultBridge: PodLoaderBridge = {
  async scanDevice(): Promise<DeviceConnectionState> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<DeviceConnectionState>(
        "scan_device",
      );
    }
    try {
      const res = await fetch("/api/scan_device");
      if (res.ok) {
        return (await res.json()) as DeviceConnectionState;
      }
    } catch {
      // Fallback when backend server is not running
    }
    return { status: "disconnected" };
  },
  async initializeDevice(
    mountPath: string,
    deviceName: string,
  ): Promise<DeviceConnectionState> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<DeviceConnectionState>(
        "initialize_device",
        { mountPath, deviceName },
      );
    }
    const res = await fetch("/api/initialize_device", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ mountPath, deviceName }),
    });
    return (await res.json()) as DeviceConnectionState;
  },
};

const rootEl = document.getElementById("root");
if (rootEl) {
  createRoot(rootEl).render(
    <StrictMode>
      <App bridge={defaultBridge} />
    </StrictMode>,
  );
}
