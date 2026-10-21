import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import type {
  CommitOutcome,
  DeviceConnectionState,
  DevicePlaylist,
  DeviceSession,
  DuplicateResolution,
  HostDirectoryListing,
  InterruptedResolution,
  PodLoaderBridge,
  SyncBatch,
  TrackMetadataEdits,
  TranscodePreset,
  TransferProgress,
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
    onProgress?: (progress: TransferProgress) => void;
  }): Promise<CommitOutcome> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<CommitOutcome>(
        "commit_sync_batch",
        options ?? {},
      );
    }
    const res = await fetch("http://127.0.0.1:41731/api/commit_sync_batch", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(options ?? {}),
    });
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}: ${res.statusText}`);
    }
    if (!res.body) {
      return res.json() as Promise<CommitOutcome>;
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let finalOutcome: CommitOutcome | null = null;
    let finalError: string | null = null;

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";
      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const msg = JSON.parse(line);
          if (msg.type === "progress" && options?.onProgress) {
            options.onProgress(msg.data);
          } else if (msg.type === "outcome") {
            finalOutcome = msg.data;
          } else if (msg.type === "error") {
            finalError = msg.error;
          }
        } catch {
          // Ignore unparseable chunks
        }
      }
    }
    if (finalError) {
      throw new Error(finalError);
    }
    if (finalOutcome) {
      return finalOutcome;
    }
    throw new Error("No outcome received from commit");
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
  async setTranscodePreset(preset: TranscodePreset): Promise<DeviceSession> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<DeviceSession>(
        "set_transcode_preset",
        { preset },
      );
    }
    return postJson<DeviceSession>("/api/set_transcode_preset", { preset });
  },
  async stageTrackMetadataEdits(
    edits: Array<{ trackId: number; edits: TrackMetadataEdits }>,
  ): Promise<SyncBatch> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<SyncBatch>(
        "stage_track_metadata_edits",
        { edits },
      );
    }
    return postJson<SyncBatch>("/api/stage_track_metadata_edits", { edits });
  },
  async stageTrackRemovals(trackIds: number[]): Promise<SyncBatch> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<SyncBatch>(
        "stage_track_removals",
        { trackIds },
      );
    }
    return postJson<SyncBatch>("/api/stage_track_removals", { trackIds });
  },
  async createDevicePlaylist(name: string): Promise<DevicePlaylist> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<DevicePlaylist>(
        "create_device_playlist",
        { name },
      );
    }
    return postJson<DevicePlaylist>("/api/create_device_playlist", { name });
  },
  async addTracksToPlaylist(
    playlistId: number,
    trackIds: number[],
  ): Promise<DevicePlaylist> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<DevicePlaylist>(
        "add_tracks_to_playlist",
        { playlistId, trackIds },
      );
    }
    return postJson<DevicePlaylist>("/api/add_tracks_to_playlist", {
      playlistId,
      trackIds,
    });
  },
  async reorderPlaylistTracks(
    playlistId: number,
    trackIds: number[],
  ): Promise<DevicePlaylist> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<DevicePlaylist>(
        "reorder_playlist_tracks",
        { playlistId, trackIds },
      );
    }
    return postJson<DevicePlaylist>("/api/reorder_playlist_tracks", {
      playlistId,
      trackIds,
    });
  },
  async removeTracksFromPlaylist(
    playlistId: number,
    trackIds: number[],
  ): Promise<DevicePlaylist> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<DevicePlaylist>(
        "remove_tracks_from_playlist",
        { playlistId, trackIds },
      );
    }
    return postJson<DevicePlaylist>("/api/remove_tracks_from_playlist", {
      playlistId,
      trackIds,
    });
  },
  async renameDevicePlaylist(
    playlistId: number,
    newName: string,
  ): Promise<DevicePlaylist> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<DevicePlaylist>(
        "rename_device_playlist",
        { playlistId, newName },
      );
    }
    return postJson<DevicePlaylist>("/api/rename_device_playlist", {
      playlistId,
      newName,
    });
  },
  async deleteDevicePlaylist(playlistId: number): Promise<void> {
    if (window.__TAURI_INTERNALS__) {
      return window.__TAURI_INTERNALS__.invoke<void>(
        "delete_device_playlist",
        { playlistId },
      );
    }
    return postJson<void>("/api/delete_device_playlist", { playlistId });
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
