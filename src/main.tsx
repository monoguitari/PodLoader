import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import type {
  CommitOutcome,
  DeviceConnectionState,
  DeviceSession,
  DuplicateResolution,
  HostDirectoryListing,
  InterruptedResolution,
  PodLoaderBridge,
  SyncBatch,
} from "./bridge";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
    };
  }
}

async function postJson<T>(url: string, body: unknown): Promise<T> {
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const data = (await res.json()) as T & { error?: string };
  if (data && typeof data === "object" && typeof data.error === "string") {
    throw new Error(data.error);
  }
  return data;
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
        const data = (await res.json()) as DeviceConnectionState & {
          error?: string;
        };
        if (data && typeof data === "object" && typeof data.error === "string") {
          throw new Error(data.error);
        }
        return data;
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
    return postJson<DeviceConnectionState>("/api/initialize_device", {
      mountPath,
      deviceName,
    });
  },
  async browseHostDirectory(dirPath: string): Promise<HostDirectoryListing> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<HostDirectoryListing>(
        "browse_host_directory",
        { dirPath },
      );
    }
    return postJson<HostDirectoryListing>("/api/browse_host_directory", {
      dirPath,
    });
  },
  async stageHostPaths(
    paths: string[],
    targetPlaylistId?: number | null,
  ): Promise<SyncBatch> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<SyncBatch>("stage_host_paths", {
        paths,
        targetPlaylistId,
      });
    }
    return postJson<SyncBatch>("/api/stage_host_paths", {
      paths,
      targetPlaylistId,
    });
  },
  async setDuplicateResolution(
    stagedId: number,
    resolution: DuplicateResolution,
  ): Promise<SyncBatch> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<SyncBatch>(
        "set_duplicate_resolution",
        { stagedId, resolution },
      );
    }
    return postJson<SyncBatch>("/api/set_duplicate_resolution", {
      stagedId,
      resolution,
    });
  },
  async commitSyncBatch(options?: {
    maxTracksBeforeInterrupt?: number;
  }): Promise<CommitOutcome> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<CommitOutcome>(
        "commit_sync_batch",
        options ?? {},
      );
    }
    return postJson<CommitOutcome>("/api/commit_sync_batch", options ?? {});
  },
  async resolveInterruptedBatch(
    resolution: InterruptedResolution,
  ): Promise<DeviceSession> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<DeviceSession>(
        "resolve_interrupted_batch",
        { resolution },
      );
    }
    return postJson<DeviceSession>("/api/resolve_interrupted_batch", {
      resolution,
    });
  },
  async setManualFirewireGuid(guid: string): Promise<DeviceSession> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<DeviceSession>(
        "set_manual_firewire_guid",
        { guid },
      );
    }
    return postJson<DeviceSession>("/api/set_manual_firewire_guid", { guid });
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
