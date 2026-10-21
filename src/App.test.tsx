import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";
import { App } from "./App";
import type {
  CommitOutcome,
  DeviceConnectionState,
  DevicePlaylist,
  DeviceSession,
  DuplicateResolution,
  ExportResult,
  HostDirectoryListing,
  HostFile,
  InterruptedResolution,
  OrphanFile,
  PlaybackStatus,
  PodLoaderBridge,
  SyncBatch,
  TrackMetadataEdits,
  TranscodePreset,
  TransferProgress,
} from "./bridge";

class InMemoryPodLoaderBridge implements PodLoaderBridge {
  public state: DeviceConnectionState;
  private hostListing: HostDirectoryListing;
  private nextStagedId = 1;
  public shouldInterruptNextCommit = false;
  public playbackStatus: PlaybackStatus = {
    state: "stopped",
    track: null,
    elapsedMs: 0,
    durationMs: 0,
    volume: 1.0,
  };
  public exportedBatches: Array<{
    trackIds?: number[];
    playlistId?: number;
    destinationDir: string;
  }> = [];
  public ejectCallCount = 0;

  constructor(
    initialState: DeviceConnectionState,
    hostListing?: HostDirectoryListing,
  ) {
    this.state = initialState;
    this.hostListing = hostListing ?? {
      currentPath: "/music",
      parentPath: "/",
      subdirectories: [{ name: "OK Computer", path: "/music/OK Computer" }],
      files: [],
    };
  }

  async scanDevice(): Promise<DeviceConnectionState> {
    return this.state;
  }

  async initializeDevice(
    mountPath: string,
    deviceName: string,
  ): Promise<DeviceConnectionState> {
    this.state = {
      status: "connected",
      data: {
        device: {
          mountPath,
          name: deviceName,
          modelNumber: "MA147",
          firewireGuid: "000A27001A2B3C4D",
          totalBytes: 30_000_000_000,
          freeBytes: 29_950_000_000,
          usedBytes: 50_000_000,
        },
        library: {
          tracks: [],
          playlists: [
            {
              id: 1,
              name: deviceName,
              kind: "master",
              trackIds: [],
            },
          ],
        },
        hasPendingPlayCounts: false,
        syncBatch: { additions: [], requiredBytes: 0 },
        interruptedBatch: null,
      },
    };
    return this.state;
  }

  async browseHostDirectory(_dirPath: string): Promise<HostDirectoryListing> {
    return this.hostListing;
  }

  async stageHostPaths(
    paths: string[],
    targetPlaylistId?: number | null,
  ): Promise<SyncBatch> {
    if (this.state.status !== "connected") {
      throw new Error("No active device");
    }
    const session = this.state.data;
    const batch: SyncBatch = session.syncBatch ?? {
      additions: [],
      requiredBytes: 0,
    };

    const matchedFiles: HostFile[] = this.hostListing.files.filter((f) =>
      paths.includes(f.path),
    );
    for (const hf of matchedFiles) {
      const existing = session.library.tracks.find(
        (dt) =>
          dt.artist.toLowerCase() === hf.artist.toLowerCase() &&
          dt.album.toLowerCase() === hf.album.toLowerCase() &&
          dt.title.toLowerCase() === hf.title.toLowerCase() &&
          dt.trackNumber === hf.trackNumber,
      );
      const isDuplicate = Boolean(existing);
      const requiresTranscode = ["FLAC", "OGG", "OPUS"].includes(
        hf.format.toUpperCase(),
      );
      const activePreset = session.transcodePreset ?? "alac";
      batch.additions.push({
        stagedId: this.nextStagedId++,
        hostFile: hf,
        targetPlaylistId: targetPlaylistId ?? null,
        isDuplicate,
        matchedDeviceTrackId: existing?.id ?? null,
        duplicateResolution: isDuplicate ? "skip" : "importAnyway",
        requiresTranscode,
        targetFormat: requiresTranscode ? activePreset : null,
      });
    }

    batch.requiredBytes = batch.additions
      .filter((a) => !a.isDuplicate || a.duplicateResolution !== "skip")
      .reduce((sum, a) => sum + a.hostFile.fileSize, 0);
    session.syncBatch = { ...batch, additions: [...batch.additions] };
    return session.syncBatch;
  }

  async setDuplicateResolution(
    stagedId: number,
    resolution: DuplicateResolution,
  ): Promise<SyncBatch> {
    if (this.state.status !== "connected") {
      throw new Error("No active device");
    }
    const session = this.state.data;
    const batch = session.syncBatch ?? { additions: [], requiredBytes: 0 };
    const additions = batch.additions.map((a) =>
      a.stagedId === stagedId ? { ...a, duplicateResolution: resolution } : a,
    );
    const requiredBytes = additions
      .filter((a) => !a.isDuplicate || a.duplicateResolution !== "skip")
      .reduce((sum, a) => sum + a.hostFile.fileSize, 0);
    session.syncBatch = { additions, requiredBytes };
    return session.syncBatch;
  }

  async commitSyncBatch(options?: {
    maxTracksBeforeInterrupt?: number;
    onProgress?: (progress: TransferProgress) => void;
  }): Promise<CommitOutcome> {
    if (this.state.status !== "connected") {
      throw new Error("No active device");
    }
    const session = this.state.data;
    const batch = session.syncBatch ?? { additions: [], requiredBytes: 0 };

    if (batch.requiredBytes > session.device.freeBytes) {
      throw new Error(
        `InsufficientDiskSpace: required ${batch.requiredBytes} bytes, available ${session.device.freeBytes} bytes`,
      );
    }

    if (
      this.shouldInterruptNextCommit ||
      options?.maxTracksBeforeInterrupt !== undefined
    ) {
      this.shouldInterruptNextCommit = false;
      const summary = {
        completedTrackCount: 1,
        remainingTrackCount: Math.max(0, batch.additions.length - 1),
        writtenObfuscatedPaths: [":iPod_Control:Music:F01:ABCD.mp3"],
        reason: "Transfer paused after 1 track",
      };
      session.interruptedBatch = summary;
      return { outcome: "interrupted", data: summary };
    }

    for (const item of batch.additions) {
      if (item.isDuplicate && item.duplicateResolution === "skip") {
        continue;
      }
      if (
        item.isDuplicate &&
        item.duplicateResolution === "replaceExisting" &&
        item.matchedDeviceTrackId != null
      ) {
        const idx = session.library.tracks.findIndex(
          (t) => t.id === item.matchedDeviceTrackId,
        );
        if (idx >= 0) {
          session.library.tracks[idx] = {
            ...session.library.tracks[idx],
            title: item.hostFile.title,
            artist: item.hostFile.artist,
            album: item.hostFile.album,
            trackNumber: item.hostFile.trackNumber,
            discNumber: item.hostFile.discNumber,
            genre: item.hostFile.genre,
            year: item.hostFile.year,
            durationMs: item.hostFile.durationMs,
            bitrateKbps: item.hostFile.bitrateKbps,
            obfuscatedPath: `:iPod_Control:Music:F00:R${item.stagedId}.mp3`,
          };
          continue;
        }
      }

      const newId = 500 + item.stagedId;
      const targetPreset = item.targetFormat ?? session.transcodePreset ?? "alac";
      const ext = item.requiresTranscode
        ? targetPreset === "mp3"
          ? "mp3"
          : "m4a"
        : item.hostFile.format.toLowerCase() === "wav"
          ? "wav"
          : item.hostFile.format.toLowerCase() === "m4a"
            ? "m4a"
            : "mp3";
      session.library.tracks.push({
        id: newId,
        title: item.hostFile.title,
        artist: item.hostFile.artist,
        album: item.hostFile.album,
        trackNumber: item.hostFile.trackNumber,
        discNumber: item.hostFile.discNumber,
        genre: item.hostFile.genre,
        year: item.hostFile.year,
        durationMs: item.hostFile.durationMs,
        bitrateKbps: item.hostFile.bitrateKbps,
        playCount: 0,
        rating: 0,
        bookmarkMs: 0,
        obfuscatedPath: `:iPod_Control:Music:F00:T${item.stagedId}.${ext}`,
      });
      for (const pl of session.library.playlists) {
        if (pl.kind === "master" || pl.id === item.targetPlaylistId) {
          if (!pl.trackIds.includes(newId)) {
            pl.trackIds.push(newId);
          }
        }
      }
    }

    if (batch.edits) {
      for (const edit of batch.edits) {
        const tr = session.library.tracks.find((t) => t.id === edit.trackId);
        if (tr) {
          if (edit.edits.title !== undefined && edit.edits.title !== null) tr.title = edit.edits.title;
          if (edit.edits.artist !== undefined && edit.edits.artist !== null) tr.artist = edit.edits.artist;
          if (edit.edits.album !== undefined && edit.edits.album !== null) tr.album = edit.edits.album;
          if (edit.edits.albumArtist !== undefined) tr.albumArtist = edit.edits.albumArtist;
          if (edit.edits.trackNumber !== undefined && edit.edits.trackNumber !== null) tr.trackNumber = edit.edits.trackNumber;
          if (edit.edits.discNumber !== undefined && edit.edits.discNumber !== null) tr.discNumber = edit.edits.discNumber;
          if (edit.edits.genre !== undefined && edit.edits.genre !== null) tr.genre = edit.edits.genre;
          if (edit.edits.year !== undefined && edit.edits.year !== null) tr.year = edit.edits.year;
          if (edit.edits.isCompilation !== undefined && edit.edits.isCompilation !== null) tr.isCompilation = edit.edits.isCompilation;
          if (edit.edits.rating !== undefined && edit.edits.rating !== null) tr.rating = edit.edits.rating;
        }
      }
    }

    if (batch.removals) {
      const removalIds = batch.removals.map((r) => r.trackId);
      session.library.tracks = session.library.tracks.filter(
        (t) => !removalIds.includes(t.id),
      );
      for (const pl of session.library.playlists) {
        pl.trackIds = pl.trackIds.filter((id) => !removalIds.includes(id));
      }
    }

    if (batch.orphanRecoveries) {
      for (const rec of batch.orphanRecoveries) {
        const newId = 800 + rec.stagedId;
        session.library.tracks.push({
          id: newId,
          title: rec.orphanFile.title,
          artist: rec.orphanFile.artist,
          album: rec.orphanFile.album,
          trackNumber: rec.orphanFile.trackNumber,
          discNumber: rec.orphanFile.discNumber,
          genre: rec.orphanFile.genre,
          year: rec.orphanFile.year,
          durationMs: rec.orphanFile.durationMs,
          bitrateKbps: rec.orphanFile.bitrateKbps,
          playCount: 0,
          rating: 0,
          bookmarkMs: 0,
          obfuscatedPath: rec.orphanFile.obfuscatedPath,
        });
        session.library.playlists[0].trackIds.push(newId);
        if (session.orphanFiles) {
          session.orphanFiles = session.orphanFiles.filter(
            (o) => o.obfuscatedPath !== rec.orphanFile.obfuscatedPath,
          );
        }
      }
    }

    if (batch.orphanDeletions) {
      for (const del of batch.orphanDeletions) {
        if (session.orphanFiles) {
          session.orphanFiles = session.orphanFiles.filter(
            (o) => o.obfuscatedPath !== del.orphanFile.obfuscatedPath,
          );
        }
      }
    }

    session.syncBatch = {
      additions: [],
      edits: [],
      removals: [],
      orphanRecoveries: [],
      orphanDeletions: [],
      requiredBytes: 0,
    };
    session.interruptedBatch = null;
    return { outcome: "completed", data: { ...session } };
  }

  async resolveInterruptedBatch(
    resolution: InterruptedResolution,
  ): Promise<DeviceSession> {
    if (this.state.status !== "connected") {
      throw new Error("No active device");
    }
    const session = this.state.data;
    if (resolution === "commitCompleted") {
      const first = session.syncBatch?.additions[0];
      if (first) {
        session.library.tracks.push({
          id: 901,
          title: first.hostFile.title,
          artist: first.hostFile.artist,
          album: first.hostFile.album,
          trackNumber: first.hostFile.trackNumber,
          discNumber: first.hostFile.discNumber,
          genre: first.hostFile.genre,
          year: first.hostFile.year,
          durationMs: first.hostFile.durationMs,
          bitrateKbps: first.hostFile.bitrateKbps,
          playCount: 0,
          rating: 0,
          bookmarkMs: 0,
          obfuscatedPath: ":iPod_Control:Music:F01:ABCD.mp3",
        });
        session.library.playlists[0].trackIds.push(901);
        session.syncBatch = {
          additions: session.syncBatch?.additions.slice(1) ?? [],
          requiredBytes: 0,
        };
      }
    }
    session.interruptedBatch = null;
    return { ...session };
  }

  async setManualFirewireGuid(guid: string): Promise<DeviceSession> {
    if (this.state.status !== "connected") {
      throw new Error("No active device");
    }
    this.state.data.device.firewireGuid = guid;
    return { ...this.state.data };
  }

  async setTranscodePreset(preset: TranscodePreset): Promise<DeviceSession> {
    if (this.state.status !== "connected") {
      throw new Error("No active device");
    }
    if (
      (preset === "aac" || preset === "mp3") &&
      this.state.data.ffmpegAvailable === false
    ) {
      throw new Error("System ffmpeg is required for lossy AAC/MP3 transcoding");
    }
    this.state.data.transcodePreset = preset;
    if (this.state.data.syncBatch) {
      this.state.data.syncBatch = {
        ...this.state.data.syncBatch,
        additions: this.state.data.syncBatch.additions.map((a) =>
          a.requiresTranscode ? { ...a, targetFormat: preset } : a,
        ),
      };
    }
    return { ...this.state.data };
  }

  async stageTrackMetadataEdits(
    edits: Array<{ trackId: number; edits: TrackMetadataEdits }>,
  ): Promise<SyncBatch> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    const batch = session.syncBatch ?? { additions: [], requiredBytes: 0 };
    batch.edits = batch.edits ?? [];
    for (const item of edits) {
      const existing = batch.edits.find((e) => e.trackId === item.trackId);
      if (existing) {
        existing.edits = { ...existing.edits, ...item.edits };
      } else {
        batch.edits.push({
          stagedId: this.nextStagedId++,
          trackId: item.trackId,
          edits: item.edits,
        });
      }
    }
    session.syncBatch = { ...batch };
    return session.syncBatch;
  }

  async stageTrackRemovals(trackIds: number[]): Promise<SyncBatch> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    const batch = session.syncBatch ?? { additions: [], requiredBytes: 0 };
    batch.removals = batch.removals ?? [];
    for (const tid of trackIds) {
      const tr = session.library.tracks.find((t) => t.id === tid);
      if (tr && !batch.removals.some((r) => r.trackId === tid)) {
        batch.removals.push({
          stagedId: this.nextStagedId++,
          trackId: tid,
          trackTitle: tr.title,
          trackArtist: tr.artist,
        });
      }
    }
    session.syncBatch = { ...batch };
    return session.syncBatch;
  }

  async createDevicePlaylist(name: string): Promise<DevicePlaylist> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    const newId =
      (session.library.playlists
        .map((p) => p.id)
        .reduce((max, id) => Math.max(max, id), 1000) ?? 1000) + 1;
    const pl: DevicePlaylist = {
      id: newId,
      name,
      kind: "user",
      trackIds: [],
    };
    session.library.playlists.push(pl);
    this.state = {
      ...this.state,
      data: {
        ...session,
        library: {
          ...session.library,
          playlists: [...session.library.playlists],
        },
      },
    };
    return pl;
  }

  async addTracksToPlaylist(
    playlistId: number,
    trackIds: number[],
  ): Promise<DevicePlaylist> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    const pl = session.library.playlists.find((p) => p.id === playlistId);
    if (!pl) throw new Error("Playlist not found");
    for (const tid of trackIds) {
      pl.trackIds.push(tid);
    }
    this.state = {
      ...this.state,
      data: {
        ...session,
        library: {
          ...session.library,
          playlists: [...session.library.playlists],
        },
      },
    };
    return pl;
  }

  async reorderPlaylistTracks(
    playlistId: number,
    trackIds: number[],
  ): Promise<DevicePlaylist> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    const pl = session.library.playlists.find((p) => p.id === playlistId);
    if (!pl) throw new Error("Playlist not found");
    pl.trackIds = [...trackIds];
    this.state = {
      ...this.state,
      data: {
        ...session,
        library: {
          ...session.library,
          playlists: [...session.library.playlists],
        },
      },
    };
    return pl;
  }

  async removeTracksFromPlaylist(
    playlistId: number,
    trackIds: number[],
  ): Promise<DevicePlaylist> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    const pl = session.library.playlists.find((p) => p.id === playlistId);
    if (!pl) throw new Error("Playlist not found");
    pl.trackIds = pl.trackIds.filter((id) => !trackIds.includes(id));
    this.state = {
      ...this.state,
      data: {
        ...session,
        library: {
          ...session.library,
          playlists: [...session.library.playlists],
        },
      },
    };
    return pl;
  }

  async renameDevicePlaylist(
    playlistId: number,
    newName: string,
  ): Promise<DevicePlaylist> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    const pl = session.library.playlists.find((p) => p.id === playlistId);
    if (!pl) throw new Error("Playlist not found");
    pl.name = newName;
    this.state = {
      ...this.state,
      data: {
        ...session,
        library: {
          ...session.library,
          playlists: [...session.library.playlists],
        },
      },
    };
    return pl;
  }

  async deleteDevicePlaylist(playlistId: number): Promise<void> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    session.library.playlists = session.library.playlists.filter(
      (p) => p.id !== playlistId,
    );
    this.state = {
      ...this.state,
      data: {
        ...session,
        library: {
          ...session.library,
          playlists: [...session.library.playlists],
        },
      },
    };
  }

  async stageOrphanRecovery(obfuscatedPath: string): Promise<SyncBatch> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    const batch = session.syncBatch ?? { additions: [], requiredBytes: 0 };
    batch.orphanRecoveries = batch.orphanRecoveries ?? [];
    batch.orphanDeletions = (batch.orphanDeletions ?? []).filter(
      (d) => d.orphanFile.obfuscatedPath !== obfuscatedPath,
    );
    const orphan = session.orphanFiles?.find(
      (o) => o.obfuscatedPath === obfuscatedPath,
    );
    if (
      orphan &&
      !batch.orphanRecoveries.some(
        (r) => r.orphanFile.obfuscatedPath === obfuscatedPath,
      )
    ) {
      batch.orphanRecoveries.push({
        stagedId: this.nextStagedId++,
        orphanFile: orphan,
      });
    }
    session.syncBatch = { ...batch };
    return session.syncBatch;
  }

  async stageOrphanDeletion(obfuscatedPath: string): Promise<SyncBatch> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    const batch = session.syncBatch ?? { additions: [], requiredBytes: 0 };
    batch.orphanDeletions = batch.orphanDeletions ?? [];
    batch.orphanRecoveries = (batch.orphanRecoveries ?? []).filter(
      (r) => r.orphanFile.obfuscatedPath !== obfuscatedPath,
    );
    const orphan = session.orphanFiles?.find(
      (o) => o.obfuscatedPath === obfuscatedPath,
    );
    if (
      orphan &&
      !batch.orphanDeletions.some(
        (d) => d.orphanFile.obfuscatedPath === obfuscatedPath,
      )
    ) {
      batch.orphanDeletions.push({
        stagedId: this.nextStagedId++,
        orphanFile: orphan,
      });
    }
    session.syncBatch = { ...batch };
    return session.syncBatch;
  }

  async exportDeviceTracks(
    trackIds: number[],
    destinationDir: string,
  ): Promise<ExportResult> {
    this.exportedBatches.push({ trackIds, destinationDir });
    return {
      exportedCount: trackIds.length,
      exportedFiles: trackIds.map((id) => `${destinationDir}/${id}.mp3`),
    };
  }

  async exportDevicePlaylist(
    playlistId: number,
    destinationDir: string,
  ): Promise<ExportResult> {
    this.exportedBatches.push({ playlistId, destinationDir });
    return {
      exportedCount: 1,
      exportedFiles: [`${destinationDir}/track.mp3`],
    };
  }

  async ejectDevice(force?: boolean): Promise<void> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const session = this.state.data;
    const hasUncommitted =
      (session.syncBatch?.additions?.length ?? 0) > 0 ||
      (session.syncBatch?.edits?.length ?? 0) > 0 ||
      (session.syncBatch?.removals?.length ?? 0) > 0 ||
      (session.syncBatch?.orphanRecoveries?.length ?? 0) > 0 ||
      (session.syncBatch?.orphanDeletions?.length ?? 0) > 0;
    if (!force && hasUncommitted) {
      throw new Error("UncommittedChangesPending");
    }
    if (this.playbackStatus.track?.sourceKind === "device") {
      this.playbackStatus = {
        state: "stopped",
        track: null,
        elapsedMs: 0,
        durationMs: 0,
        volume: this.playbackStatus.volume,
      };
    }
    this.ejectCallCount++;
    this.state = { status: "disconnected" };
  }

  async playHostFile(hostFile: HostFile): Promise<PlaybackStatus> {
    this.playbackStatus = {
      state: "playing",
      track: {
        sourceKind: "host",
        sourcePath: hostFile.path,
        title: hostFile.title,
        artist: hostFile.artist,
        album: hostFile.album,
        durationMs: hostFile.durationMs,
        trackId: null,
      },
      elapsedMs: 0,
      durationMs: hostFile.durationMs,
      volume: this.playbackStatus.volume,
    };
    return this.playbackStatus;
  }

  async playDeviceTrack(trackId: number): Promise<PlaybackStatus> {
    if (this.state.status !== "connected") throw new Error("No active device");
    const tr = this.state.data.library.tracks.find((t) => t.id === trackId);
    if (!tr) throw new Error("Track not found");
    this.playbackStatus = {
      state: "playing",
      track: {
        sourceKind: "device",
        sourcePath: tr.obfuscatedPath,
        title: tr.title,
        artist: tr.artist,
        album: tr.album,
        durationMs: tr.durationMs,
        trackId: tr.id,
      },
      elapsedMs: 0,
      durationMs: tr.durationMs,
      volume: this.playbackStatus.volume,
    };
    return this.playbackStatus;
  }

  async pausePlayback(): Promise<PlaybackStatus> {
    if (this.playbackStatus.state === "playing") {
      this.playbackStatus = { ...this.playbackStatus, state: "paused" };
    }
    return this.playbackStatus;
  }

  async resumePlayback(): Promise<PlaybackStatus> {
    if (this.playbackStatus.state === "paused") {
      this.playbackStatus = { ...this.playbackStatus, state: "playing" };
    }
    return this.playbackStatus;
  }

  async stopPlayback(): Promise<PlaybackStatus> {
    this.playbackStatus = {
      state: "stopped",
      track: null,
      elapsedMs: 0,
      durationMs: 0,
      volume: this.playbackStatus.volume,
    };
    return this.playbackStatus;
  }

  async seekPlayback(positionMs: number): Promise<PlaybackStatus> {
    this.playbackStatus = {
      ...this.playbackStatus,
      elapsedMs: Math.min(positionMs, this.playbackStatus.durationMs),
    };
    return this.playbackStatus;
  }

  async setPlaybackVolume(volume: number): Promise<PlaybackStatus> {
    this.playbackStatus = {
      ...this.playbackStatus,
      volume: Math.max(0, Math.min(1, volume)),
    };
    return this.playbackStatus;
  }

  async getPlaybackStatus(): Promise<PlaybackStatus> {
    return this.playbackStatus;
  }
}

describe("PodLoaderBridge React UI Seam", () => {
  test("renders connected Device metadata, storage capacity, playlists sidebar, and ordered Device Tracks on playlist selection", async () => {
    const user = userEvent.setup();
    const bridge = new InMemoryPodLoaderBridge({
      status: "connected",
      data: {
        device: {
          mountPath: "/Volumes/IPOD",
          name: "Jesus's Classic",
          modelNumber: "MA147",
          firewireGuid: "000A2700AABBCCDD",
          totalBytes: 80_000_000_000,
          freeBytes: 60_000_000_000,
          usedBytes: 20_000_000_000,
        },
        library: {
          tracks: [
            {
              id: 101,
              title: "Paranoid Android",
              artist: "Radiohead",
              album: "OK Computer",
              trackNumber: 2,
              discNumber: 1,
              genre: "Alternative",
              year: 1997,
              durationMs: 383_000,
              bitrateKbps: 945,
              playCount: 14,
              rating: 5,
              lastPlayedTimestamp: 3_800_000_000,
              bookmarkMs: 0,
              obfuscatedPath: ":iPod_Control:Music:F04:ABCD.m4a",
            },
            {
              id: 102,
              title: "Everything In Its Right Place",
              artist: "Radiohead",
              album: "Kid A",
              trackNumber: 1,
              discNumber: 2,
              genre: "Electronic",
              year: 2000,
              durationMs: 251_000,
              bitrateKbps: 320,
              playCount: 5,
              rating: 4,
              lastPlayedTimestamp: 3_800_005_000,
              bookmarkMs: 0,
              obfuscatedPath: ":iPod_Control:Music:F12:EFGH.mp3",
            },
          ],
          playlists: [
            {
              id: 1000,
              name: "Jesus's Classic",
              kind: "master",
              trackIds: [101, 102],
            },
            {
              id: 2000,
              name: "Road Trip",
              kind: "user",
              trackIds: [102, 101],
            },
            {
              id: 3000,
              name: "Top Rated",
              kind: "smart",
              trackIds: [101],
            },
          ],
        },
        hasPendingPlayCounts: true,
      },
    });

    render(<App bridge={bridge} />);

    // Device metadata & storage summary
    expect(await screen.findByText(/MA147/)).toBeInTheDocument();
    expect(screen.getByText(/000A2700AABBCCDD/)).toBeInTheDocument();
    expect(screen.getByText(/20\.0 GB used/i)).toBeInTheDocument();
    expect(screen.getByText(/60\.0 GB free/i)).toBeInTheDocument();

    // Sidebar sections: Master Playlist, Device Playlists, Smart Playlists
    const sidebar = screen.getByRole("navigation", { name: /Device Playlists/i });
    expect(within(sidebar).getByRole("button", { name: /Jesus's Classic/i })).toBeInTheDocument();
    expect(within(sidebar).getByRole("button", { name: /Road Trip/i })).toBeInTheDocument();
    expect(within(sidebar).getByRole("button", { name: /Top Rated/i })).toBeInTheDocument();

    // Default Master Playlist shows track 101 then track 102 with all required columns
    const table = screen.getByRole("table", { name: /Device Tracks/i });
    const initialRows = within(table).getAllByRole("row").slice(1); // skip header row
    expect(initialRows).toHaveLength(2);
    expect(within(initialRows[0]).getByText("Paranoid Android")).toBeInTheDocument();
    expect(within(initialRows[0]).getByText("Radiohead")).toBeInTheDocument();
    expect(within(initialRows[0]).getByText("OK Computer")).toBeInTheDocument();
    expect(within(initialRows[0]).getByText("Alternative")).toBeInTheDocument();
    expect(within(initialRows[0]).getByText("1997")).toBeInTheDocument();
    expect(within(initialRows[0]).getByText("6:23")).toBeInTheDocument();
    expect(within(initialRows[0]).getByText("945 kbps")).toBeInTheDocument();
    expect(within(initialRows[0]).getByText("14")).toBeInTheDocument();
    expect(within(initialRows[0]).getByText("★★★★★")).toBeInTheDocument();

    expect(within(initialRows[1]).getByText("Everything In Its Right Place")).toBeInTheDocument();
    expect(within(initialRows[1]).getByText("4:11")).toBeInTheDocument();
    expect(within(initialRows[1]).getByText("320 kbps")).toBeInTheDocument();
    expect(within(initialRows[1]).getByText("★★★★☆")).toBeInTheDocument();

    // Selecting "Road Trip" user Device Playlist reorders rows to [102, 101]
    await user.click(within(sidebar).getByRole("button", { name: /Road Trip/i }));
    const roadTripRows = within(table).getAllByRole("row").slice(1);
    expect(roadTripRows).toHaveLength(2);
    expect(within(roadTripRows[0]).getByText("Everything In Its Right Place")).toBeInTheDocument();
    expect(within(roadTripRows[1]).getByText("Paranoid Android")).toBeInTheDocument();

    // Selecting "Top Rated" Smart Playlist filters rows to [101]
    await user.click(within(sidebar).getByRole("button", { name: /Top Rated/i }));
    const smartRows = within(table).getAllByRole("row").slice(1);
    expect(smartRows).toHaveLength(1);
    expect(within(smartRows[0]).getByText("Paranoid Android")).toBeInTheDocument();
  });

  test("allows user to trigger Device Initialization when an uninitialized volume is connected", async () => {
    const user = userEvent.setup();
    const bridge = new InMemoryPodLoaderBridge({
      status: "uninitializedVolume",
      data: {
        mountPath: "/Volumes/NEW_IPOD",
        volumeName: "NEW_IPOD",
        totalBytes: 30_000_000_000,
        freeBytes: 29_950_000_000,
        usbFirewireGuid: "000A27001A2B3C4D",
      },
    });

    render(<App bridge={bridge} />);

    expect(
      await screen.findByText(/Uninitialized volume detected: NEW_IPOD/i),
    ).toBeInTheDocument();

    const nameInput = screen.getByLabelText(/Device Name/i);
    await user.clear(nameInput);
    await user.type(nameInput, "Fresh Classic");

    const initButton = screen.getByRole("button", {
      name: /Initialize Device/i,
    });
    await user.click(initButton);

    // Transitions to Connected view with the newly initialized Master Playlist
    const sidebar = await screen.findByRole("navigation", {
      name: /Device Playlists/i,
    });
    expect(
      within(sidebar).getByRole("button", { name: /Fresh Classic/i }),
    ).toBeInTheDocument();
    expect(screen.getByText(/29\.9 GB free/i)).toBeInTheDocument();
  });

  test("browses Host Files, stages tracks into Sync Batch with duplicate resolution toggles, handles mid-batch recovery, and commits to Device Library", async () => {
    const user = userEvent.setup();
    const bridge = new InMemoryPodLoaderBridge(
      {
        status: "connected",
        data: {
          device: {
            mountPath: "/Volumes/IPOD",
            name: "Classic 160GB",
            modelNumber: "MC297",
            firewireGuid: null,
            totalBytes: 160_000_000_000,
            freeBytes: 120_000_000_000,
            usedBytes: 40_000_000_000,
          },
          library: {
            tracks: [
              {
                id: 101,
                title: "Paranoid Android",
                artist: "Radiohead",
                album: "OK Computer",
                trackNumber: 2,
                discNumber: 1,
                genre: "Alternative",
                year: 1997,
                durationMs: 383_000,
                bitrateKbps: 320,
                playCount: 10,
                rating: 5,
                bookmarkMs: 0,
                obfuscatedPath: ":iPod_Control:Music:F04:ABCD.mp3",
              },
            ],
            playlists: [
              {
                id: 1000,
                name: "Classic 160GB",
                kind: "master",
                trackIds: [101],
              },
              {
                id: 2000,
                name: "Favorites",
                kind: "user",
                trackIds: [],
              },
            ],
          },
          hasPendingPlayCounts: false,
          syncBatch: { additions: [], requiredBytes: 0 },
          interruptedBatch: null,
        },
      },
      {
        currentPath: "/music/OK Computer",
        parentPath: "/music",
        subdirectories: [],
        files: [
          {
            path: "/music/OK Computer/02 - Paranoid Android.mp3",
            fileName: "02 - Paranoid Android.mp3",
            format: "MP3",
            fileSize: 8_000_000,
            title: "Paranoid Android",
            artist: "Radiohead",
            album: "OK Computer",
            trackNumber: 2,
            discNumber: 1,
            genre: "Alternative",
            year: 1997,
            durationMs: 383_000,
            bitrateKbps: 320,
          },
          {
            path: "/music/OK Computer/06 - Karma Police.mp3",
            fileName: "06 - Karma Police.mp3",
            format: "MP3",
            fileSize: 6_500_000,
            title: "Karma Police",
            artist: "Radiohead",
            album: "OK Computer",
            trackNumber: 6,
            discNumber: 1,
            genre: "Alternative",
            year: 1997,
            durationMs: 264_000,
            bitrateKbps: 320,
          },
        ],
      },
    );

    render(<App bridge={bridge} />);

    // 1. Host File Browser pane displays local audio files and their tags
    const hostTable = await screen.findByRole("table", { name: /Host Files/i });
    expect(within(hostTable).getByText("Karma Police")).toBeInTheDocument();
    expect(within(hostTable).getAllByText("MP3").length).toBeGreaterThan(0);

    // 2. Manual FireWire GUID entry when Device lacks one
    const guidInput = screen.getByLabelText(/Manual FireWire GUID/i);
    await user.type(guidInput, "000A270011223344");
    await user.click(screen.getByRole("button", { name: /Save GUID/i }));
    expect(await screen.findByText(/000A270011223344/)).toBeInTheDocument();

    // 3. Drag a Host File onto the "Favorites" Device Playlist to stage it
    const favoritesPlaylistButton = screen.getByRole("button", {
      name: /Favorites \(0\)/i,
    });
    const favoritesPlaylistItem = favoritesPlaylistButton.closest("li")!;
    fireEvent.drop(favoritesPlaylistItem, {
      dataTransfer: {
        getData: (type: string) =>
          type === "text/podloader-path"
            ? "/music/OK Computer/02 - Paranoid Android.mp3"
            : "",
        files: [],
      },
    });

    // Also stage the remaining Host File ("Karma Police")
    const karmaRow = within(hostTable)
      .getByText("Karma Police")
      .closest("tr")!;
    await user.click(within(karmaRow).getByRole("button", { name: /Stage/i }));

    const syncBatchRegion = await screen.findByRole("region", {
      name: /Sync Batch/i,
    });
    expect(
      within(syncBatchRegion).getByText(/Duplicate/i),
    ).toBeInTheDocument();

    // Change duplicate resolution from "Skip" to "Replace Existing"
    const resolutionSelect = within(syncBatchRegion).getByLabelText(
      /Duplicate resolution for Paranoid Android/i,
    );
    expect(resolutionSelect).toHaveValue("skip");
    await user.selectOptions(resolutionSelect, "replaceExisting");
    expect(resolutionSelect).toHaveValue("replaceExisting");

    // 4. Verify pre-flight free-space rejection when Device lacks space
    if (bridge.state.status === "connected") {
      bridge.state.data.device.freeBytes = 1_000_000; // 1 MB free (< 14.5 MB required)
    }
    await user.click(
      within(syncBatchRegion).getByRole("button", {
        name: /Commit Sync Batch/i,
      }),
    );
    expect(
      await screen.findByRole("alert"),
    ).toHaveTextContent(/InsufficientDiskSpace/i);

    // Restore sufficient free space on Device
    if (bridge.state.status === "connected") {
      bridge.state.data.device.freeBytes = 60_000_000_000;
    }

    // 5. Pause mid-batch after first track -> test Roll Back Batch first
    await user.click(
      within(syncBatchRegion).getByRole("button", {
        name: /Pause After First Track/i,
      }),
    );
    expect(
      await screen.findByText(/Transfer paused after 1 track/i),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: /Roll Back Batch/i }),
    );

    // 6. Pause mid-batch again -> test Commit Completed Tracks
    await user.click(
      within(syncBatchRegion).getByRole("button", {
        name: /Pause After First Track/i,
      }),
    );
    expect(
      await screen.findByText(/Transfer paused after 1 track/i),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: /Commit Completed Tracks/i }),
    );

    // 7. Commit remaining track and verify Device Tracks table shows the newly added track
    await user.click(
      within(syncBatchRegion).getByRole("button", {
        name: /Commit Sync Batch/i,
      }),
    );

    const deviceTable = screen.getByRole("table", { name: /Device Tracks/i });
    expect(
      await within(deviceTable).findByText("Karma Police"),
    ).toBeInTheDocument();
  });

  test("stages FLAC/OGG files for transcoding, manages per-Device transcoding presets with ffmpeg availability, and displays live per-track and overall progress during commit", async () => {
    const user = userEvent.setup();
    let resolvePendingCommit: ((outcome: CommitOutcome) => void) | null = null;

    const bridge = new InMemoryPodLoaderBridge(
      {
        status: "connected",
        data: {
          device: {
            mountPath: "/Volumes/IPOD_CLASSIC",
            name: "Classic 160GB",
            modelNumber: "MC297",
            firewireGuid: "000A270011223344",
            totalBytes: 160_000_000_000,
            freeBytes: 120_000_000_000,
            usedBytes: 40_000_000_000,
          },
          library: {
            tracks: [],
            playlists: [
              {
                id: 1000,
                name: "Classic 160GB",
                kind: "master",
                trackIds: [],
              },
            ],
          },
          hasPendingPlayCounts: false,
          syncBatch: { additions: [], requiredBytes: 0 },
          interruptedBatch: null,
          transcodePreset: "alac",
          ffmpegAvailable: false,
        },
      },
      {
        currentPath: "/music/Kid A",
        parentPath: "/music",
        subdirectories: [],
        files: [
          {
            path: "/music/Kid A/01 - Everything In Its Right Place.flac",
            fileName: "01 - Everything In Its Right Place.flac",
            format: "FLAC",
            fileSize: 24_000_000,
            title: "Everything In Its Right Place",
            artist: "Radiohead",
            album: "Kid A",
            trackNumber: 1,
            discNumber: 1,
            genre: "Electronic",
            year: 2000,
            durationMs: 251_000,
            bitrateKbps: 850,
          },
          {
            path: "/music/Kid A/02 - Kid A.ogg",
            fileName: "02 - Kid A.ogg",
            format: "OGG",
            fileSize: 8_000_000,
            title: "Kid A",
            artist: "Radiohead",
            album: "Kid A",
            trackNumber: 2,
            discNumber: 1,
            genre: "Electronic",
            year: 2000,
            durationMs: 284_000,
            bitrateKbps: 225,
          },
          {
            path: "/music/Kid A/04 - How to Disappear Completely.mp3",
            fileName: "04 - How to Disappear Completely.mp3",
            format: "MP3",
            fileSize: 9_000_000,
            title: "How to Disappear Completely",
            artist: "Radiohead",
            album: "Kid A",
            trackNumber: 4,
            discNumber: 1,
            genre: "Electronic",
            year: 2000,
            durationMs: 356_000,
            bitrateKbps: 320,
          },
        ],
      },
    );

    const originalCommit = bridge.commitSyncBatch.bind(bridge);
    bridge.commitSyncBatch = (options) => {
      options?.onProgress?.({
        currentTrackIndex: 1,
        totalTracks: 3,
        currentTrackTitle: "Everything In Its Right Place",
        isTranscoding: true,
        trackPercent: 50,
        overallPercent: 17,
      });
      return new Promise<CommitOutcome>((resolve) => {
        resolvePendingCommit = async () => {
          const outcome = await originalCommit(options);
          resolve(outcome);
        };
      });
    };

    render(<App bridge={bridge} />);

    // 1. Verify per-Device Transcoding Preset selector defaults to ALAC and disables AAC/MP3 with explanatory message when ffmpeg is absent
    const presetSelect = await screen.findByLabelText(/Transcode Preset/i);
    expect(presetSelect).toHaveValue("alac");
    const aacOption = within(presetSelect).getByRole("option", {
      name: /AAC/i,
    }) as HTMLOptionElement;
    const mp3Option = within(presetSelect).getByRole("option", {
      name: /MP3/i,
    }) as HTMLOptionElement;
    expect(aacOption.disabled).toBe(true);
    expect(mp3Option.disabled).toBe(true);
    expect(
      screen.getByText(/ffmpeg not detected on PATH/i),
    ).toBeInTheDocument();

    // 2. Stage FLAC, OGG, and native MP3 files
    await user.click(
      screen.getByRole("button", { name: /Stage All to Sync Batch/i }),
    );
    const syncBatchRegion = await screen.findByRole("region", {
      name: /Sync Batch/i,
    });
    expect(
      within(syncBatchRegion).getByText(/FLAC → ALAC/i),
    ).toBeInTheDocument();
    expect(
      within(syncBatchRegion).getByText(/OGG → ALAC/i),
    ).toBeInTheDocument();

    // 3. Enable ffmpeg on the Device and switch preset to AAC -> staged transcode badges update to AAC
    if (bridge.state.status === "connected") {
      bridge.state.data.ffmpegAvailable = true;
    }
    aacOption.disabled = false;
    await user.selectOptions(presetSelect, "aac");
    expect(
      await within(syncBatchRegion).findByText(/FLAC → AAC/i),
    ).toBeInTheDocument();
    expect(
      within(syncBatchRegion).getByText(/OGG → AAC/i),
    ).toBeInTheDocument();

    // 4. Trigger Commit Sync Batch and verify live per-track & overall progress UI
    await user.click(
      within(syncBatchRegion).getByRole("button", {
        name: /Commit Sync Batch/i,
      }),
    );

    expect(
      await within(syncBatchRegion).findByText(
        /Transcoding Track 1 of 3: Everything In Its Right Place \(50%\)/i,
      ),
    ).toBeInTheDocument();
    expect(
      within(syncBatchRegion).getByText(/Overall Progress: 17%/i),
    ).toBeInTheDocument();

    // Finish commit and verify tracks are added to Device Library
    expect(resolvePendingCommit).not.toBeNull();
    await act(async () => {
      await resolvePendingCommit!(null as unknown as CommitOutcome);
    });

    const deviceTable = screen.getByRole("table", { name: /Device Tracks/i });
    expect(
      await within(deviceTable).findByText("Everything In Its Right Place"),
    ).toBeInTheDocument();
    expect(within(deviceTable).getAllByText("Kid A")).toHaveLength(4);

    // Verify transcoded FLAC and OGG tracks were assigned .m4a container paths on the Device
    if (bridge.state.status === "connected") {
      const addedTracks = bridge.state.data.library.tracks;
      const flacTranscoded = addedTracks.find(
        (t) => t.title === "Everything In Its Right Place",
      );
      const oggTranscoded = addedTracks.find((t) => t.title === "Kid A");
      expect(flacTranscoded?.obfuscatedPath).toMatch(/\.m4a$/);
      expect(oggTranscoded?.obfuscatedPath).toMatch(/\.m4a$/);
    }
  });

  test("allows user to edit track metadata, stage removals, commit edits/removals, and manage user playlists (create, rename, delete)", async () => {
    const user = userEvent.setup();
    const bridge = new InMemoryPodLoaderBridge({
      status: "connected",
      data: {
        device: {
          mountPath: "/mnt/ipod",
          name: "Classic 80GB",
          modelNumber: "MB147",
          firewireGuid: "000A27001358762E",
          totalBytes: 80_000_000_000,
          freeBytes: 70_000_000_000,
          usedBytes: 10_000_000_000,
        },
        library: {
          tracks: [
            {
              id: 101,
              title: "Original Title",
              artist: "Original Artist",
              album: "Original Album",
              trackNumber: 1,
              discNumber: 1,
              genre: "Rock",
              year: 2001,
              durationMs: 240_000,
              bitrateKbps: 320,
              playCount: 5,
              rating: 60,
              bookmarkMs: 0,
              obfuscatedPath: ":iPod_Control:Music:F00:ABCD.mp3",
            },
            {
              id: 102,
              title: "Second Song",
              artist: "Second Artist",
              album: "Second Album",
              trackNumber: 2,
              discNumber: 1,
              genre: "Pop",
              year: 2002,
              durationMs: 180_000,
              bitrateKbps: 256,
              playCount: 10,
              rating: 80,
              bookmarkMs: 0,
              obfuscatedPath: ":iPod_Control:Music:F00:EFGH.mp3",
            },
          ],
          playlists: [
            {
              id: 1,
              name: "Classic 80GB",
              kind: "master",
              trackIds: [101, 102],
            },
          ],
        },
        hasPendingPlayCounts: false,
        syncBatch: { additions: [], edits: [], removals: [], requiredBytes: 0 },
        interruptedBatch: null,
      },
    });

    render(<App bridge={bridge} />);

    // Verify initial tracks rendered
    const tracksTable = await screen.findByRole("table", { name: /Device Tracks/i });
    expect(within(tracksTable).getByText("Original Title")).toBeInTheDocument();
    expect(within(tracksTable).getByText("Second Song")).toBeInTheDocument();

    // 1. Single Track Metadata Editing
    const selectTrack1 = screen.getByRole("checkbox", { name: /Select Original Title/i });
    await user.click(selectTrack1);

    const editMetadataBtn = screen.getByRole("button", { name: /Edit Metadata/i });
    await user.click(editMetadataBtn);

    const editModal = await screen.findByRole("dialog", { name: /Edit Track Metadata/i });
    const titleInput = within(editModal).getByLabelText(/Title/i);
    await user.clear(titleInput);
    await user.type(titleInput, "Edited Title");

    const yearInput = within(editModal).getByLabelText(/Year/i);
    await user.clear(yearInput);
    await user.type(yearInput, "2024");

    const stageEditsBtn = within(editModal).getByRole("button", { name: /Stage Edits/i });
    await user.click(stageEditsBtn);

    // Verify Staged Edits appear in Sync Batch
    const syncBatchRegion = await screen.findByRole("region", { name: /Sync Batch/i });
    expect(within(syncBatchRegion).getByText(/Edit: Edited Title/i)).toBeInTheDocument();

    // 2. Track Removal
    const selectTrack2 = screen.getByRole("checkbox", { name: /Select Second Song/i });
    await user.click(selectTrack2);

    const removeBtn = screen.getByRole("button", { name: /Remove from iPod/i });
    await user.click(removeBtn);

    expect(within(syncBatchRegion).getByText(/Remove: Second Song/i)).toBeInTheDocument();

    // 3. Commit Edits and Removals
    const commitBtn = within(syncBatchRegion).getByRole("button", { name: /Commit Sync Batch/i });
    await user.click(commitBtn);

    // Verify table updated
    expect(within(tracksTable).getByText("Edited Title")).toBeInTheDocument();
    expect(within(tracksTable).getByText("2024")).toBeInTheDocument();
    expect(within(tracksTable).queryByText("Second Song")).not.toBeInTheDocument();

    // 4. User Playlist Management (Create, Add Track, Rename, Delete)
    const newPlaylistBtn = screen.getByRole("button", { name: /New Playlist/i });
    await user.click(newPlaylistBtn);

    const playlistNameInput = await screen.findByLabelText(/New Playlist Name/i);
    await user.type(playlistNameInput, "Road Trip");
    await user.click(screen.getByRole("button", { name: /Create Playlist/i }));

    // Verify Playlist in sidebar
    const playlistsSection = screen.getByRole("navigation", { name: /Device Playlists/i });
    expect(within(playlistsSection).getByText(/Road Trip/i)).toBeInTheDocument();

    // Select Master playlist to browse tracks and add to Road Trip
    await user.click(screen.getByRole("button", { name: /Classic 80GB/i }));

    // Add selected track to playlist via toolbar
    const selectTrackEdited = screen.getByRole("checkbox", { name: /Select Edited Title/i });
    await user.click(selectTrackEdited);
    const addToPlaylistBtn = screen.getByRole("button", { name: /Add to Playlist/i });
    await user.click(addToPlaylistBtn);

    // Verify track added to playlist
    expect(within(playlistsSection).getByText(/Road Trip \(1\)/i)).toBeInTheDocument();

    // Rename Playlist
    const renameBtn = screen.getByRole("button", { name: /^Rename$/i });
    await user.click(renameBtn);
    const renameInput = await screen.findByLabelText(/Rename Playlist/i);
    await user.clear(renameInput);
    await user.type(renameInput, "Summer Road Trip");
    await user.click(screen.getByRole("button", { name: /Save Rename/i }));

    expect(within(playlistsSection).getByText(/Summer Road Trip/i)).toBeInTheDocument();

    // Delete Playlist
    const deleteBtn = screen.getByRole("button", { name: /^Delete$/i });
    await user.click(deleteBtn);

    expect(within(playlistsSection).queryByText(/Summer Road Trip/i)).not.toBeInTheDocument();
  });

  test("surfaces orphan files, stages them for recovery or deletion, exports device tracks and playlists, and safely ejects device with uncommitted changes warning", async () => {
    const user = userEvent.setup();
    const orphan1: OrphanFile = {
      path: "/Volumes/IPOD/iPod_Control/Music/F05/GHOST.mp3",
      obfuscatedPath: ":iPod_Control:Music:F05:GHOST.mp3",
      fileName: "GHOST.mp3",
      fileSize: 4_500_000,
      format: "MP3",
      title: "Ghost Track",
      artist: "Unknown Ghost",
      album: "Lost Songs",
      trackNumber: 1,
      discNumber: 1,
      genre: "Ambient",
      year: 2005,
      durationMs: 180_000,
      bitrateKbps: 320,
    };
    const orphan2: OrphanFile = {
      path: "/Volumes/IPOD/iPod_Control/Music/F12/CORRUPT.mp3",
      obfuscatedPath: ":iPod_Control:Music:F12:CORRUPT.mp3",
      fileName: "CORRUPT.mp3",
      fileSize: 2_000_000,
      format: "MP3",
      title: "Junk File",
      artist: "Junk",
      album: "Junk",
      trackNumber: 2,
      discNumber: 1,
      genre: "Noise",
      year: 2002,
      durationMs: 60_000,
      bitrateKbps: 128,
    };

    const bridge = new InMemoryPodLoaderBridge({
      status: "connected",
      data: {
        device: {
          mountPath: "/Volumes/IPOD",
          name: "My iPod Classic",
          modelNumber: "MA147",
          firewireGuid: "000A270011223344",
          totalBytes: 30_000_000_000,
          freeBytes: 25_000_000_000,
          usedBytes: 5_000_000_000,
        },
        library: {
          tracks: [
            {
              id: 101,
              title: "Airbag",
              artist: "Radiohead",
              album: "OK Computer",
              trackNumber: 1,
              discNumber: 1,
              genre: "Alternative",
              year: 1997,
              durationMs: 284_000,
              bitrateKbps: 320,
              playCount: 10,
              rating: 5,
              bookmarkMs: 0,
              obfuscatedPath: ":iPod_Control:Music:F01:AIRB.mp3",
            },
          ],
          playlists: [
            { id: 1, name: "My iPod Classic", kind: "master", trackIds: [101] },
            { id: 2, name: "Favorites", kind: "user", trackIds: [101] },
          ],
        },
        hasPendingPlayCounts: false,
        syncBatch: { additions: [], requiredBytes: 0 },
        orphanFiles: [orphan1, orphan2],
      },
    });

    render(<App bridge={bridge} />);

    // 1. Verify Orphan Files section is surfaced in UI
    const orphanSection = await screen.findByRole("region", { name: /Orphan Files/i });
    expect(within(orphanSection).getByText("Ghost Track")).toBeInTheDocument();
    expect(within(orphanSection).getByText("Junk File")).toBeInTheDocument();

    // Stage Orphan 1 for Recovery
    const recoverBtn = within(orphanSection).getByRole("button", { name: /Recover Ghost Track/i });
    await user.click(recoverBtn);

    // Stage Orphan 2 for Deletion
    const deleteOrphanBtn = within(orphanSection).getByRole("button", { name: /Delete Junk File/i });
    await user.click(deleteOrphanBtn);

    // Verify both staged in Sync Batch
    const syncBatchRegion = screen.getByRole("region", { name: /Sync Batch/i });
    expect(within(syncBatchRegion).getByText(/Recover: Ghost Track/i)).toBeInTheDocument();
    expect(within(syncBatchRegion).getByText(/Delete Orphan: Junk File/i)).toBeInTheDocument();

    // 2. Device Export: select track and export
    const selectAirbag = screen.getByRole("checkbox", { name: /Select Airbag/i });
    await user.click(selectAirbag);
    const exportTracksBtn = screen.getByRole("button", { name: /Export Selected Tracks/i });
    await user.click(exportTracksBtn);
    expect(bridge.exportedBatches.length).toBe(1);
    expect(bridge.exportedBatches[0].trackIds).toEqual([101]);

    // Export Playlist
    const exportPlaylistBtn = screen.getByRole("button", { name: /Export Playlist/i });
    await user.click(exportPlaylistBtn);
    expect(bridge.exportedBatches.length).toBe(2);

    // 3. Safe Eject with uncommitted changes warning
    const ejectBtn = screen.getByRole("button", { name: /Eject Device/i });
    await user.click(ejectBtn);

    // Warning dialog should appear because sync batch has staged changes
    const warningDialog = await screen.findByRole("dialog", { name: /Uncommitted Changes/i });
    expect(warningDialog).toBeInTheDocument();
    expect(within(warningDialog).getByText(/Sync Batch has uncommitted changes/i)).toBeInTheDocument();

    // Dismiss or cancel warning
    const cancelEjectBtn = within(warningDialog).getByRole("button", { name: /Cancel/i });
    await user.click(cancelEjectBtn);
    expect(bridge.ejectCallCount).toBe(0);

    // Commit Sync Batch
    const commitBtn = within(syncBatchRegion).getByRole("button", { name: /Commit Sync Batch/i });
    await user.click(commitBtn);

    // Now batch is clean, click Eject Device again
    await user.click(ejectBtn);
    expect(bridge.ejectCallCount).toBe(1);
    expect(await screen.findByText(/No iPod Device Connected/i)).toBeInTheDocument();
  });

  test("mini audio player bar previews host files and device tracks with play, pause, seek, volume, and stops on eject", async () => {
    const user = userEvent.setup();
    const hostFile: HostFile = {
      path: "/music/OK Computer/02 - Paranoid Android.flac",
      fileName: "02 - Paranoid Android.flac",
      format: "FLAC",
      fileSize: 35_000_000,
      title: "Paranoid Android",
      artist: "Radiohead",
      album: "OK Computer",
      trackNumber: 2,
      discNumber: 1,
      genre: "Alternative",
      year: 1997,
      durationMs: 383_000,
      bitrateKbps: 900,
    };

    const bridge = new InMemoryPodLoaderBridge(
      {
        status: "connected",
        data: {
          device: {
            mountPath: "/Volumes/IPOD",
            name: "Classic",
            totalBytes: 30_000_000_000,
            freeBytes: 25_000_000_000,
            usedBytes: 5_000_000_000,
          },
          library: {
            tracks: [
              {
                id: 101,
                title: "Airbag",
                artist: "Radiohead",
                album: "OK Computer",
                trackNumber: 1,
                discNumber: 1,
                genre: "Alternative",
                year: 1997,
                durationMs: 284_000,
                bitrateKbps: 320,
                playCount: 1,
                rating: 4,
                bookmarkMs: 0,
                obfuscatedPath: ":iPod_Control:Music:F01:AIRB.mp3",
              },
            ],
            playlists: [
              { id: 1, name: "Classic", kind: "master", trackIds: [101] },
            ],
          },
          hasPendingPlayCounts: false,
          syncBatch: { additions: [], requiredBytes: 0 },
        },
      },
      {
        currentPath: "/music/OK Computer",
        parentPath: "/music",
        subdirectories: [],
        files: [hostFile],
      },
    );

    render(<App bridge={bridge} />);

    // Wait for device to connect and UI to settle
    await screen.findByRole("heading", { name: "Classic" });

    // 1. Mini Audio Player bar should be visible at bottom
    const playerBar = await screen.findByRole("region", { name: /Audio Player/i });
    expect(playerBar).toBeInTheDocument();
    expect(within(playerBar).getByText(/No track playing/i)).toBeInTheDocument();

    // 2. Play Host File
    const playHostBtn = await screen.findByRole("button", { name: /Play Paranoid Android/i });
    await user.click(playHostBtn);

    // Verify track metadata and Host badge in player bar
    expect(within(playerBar).getByText("Paranoid Android")).toBeInTheDocument();
    expect(within(playerBar).getByText(/Radiohead — OK Computer/i)).toBeInTheDocument();
    expect(within(playerBar).getByText("Host")).toBeInTheDocument();

    // 3. Pause and Resume
    const playPauseBtn = within(playerBar).getByRole("button", { name: /Pause/i });
    await user.click(playPauseBtn);
    expect(within(playerBar).getByRole("button", { name: /Play/i })).toBeInTheDocument();

    await user.click(within(playerBar).getByRole("button", { name: /Play/i }));
    expect(within(playerBar).getByRole("button", { name: /Pause/i })).toBeInTheDocument();

    // 4. Seek and Volume
    const seekSlider = within(playerBar).getByRole("slider", { name: /Seek track/i });
    fireEvent.change(seekSlider, { target: { value: 65_000 } });
    expect(bridge.playbackStatus.elapsedMs).toBe(65_000);

    const volumeSlider = within(playerBar).getByRole("slider", { name: /Volume/i });
    fireEvent.change(volumeSlider, { target: { value: 0.8 } });
    expect(bridge.playbackStatus.volume).toBeCloseTo(0.8);

    // 5. Play Device Track
    const playDeviceBtn = screen.getByRole("button", { name: /Play Airbag/i });
    await user.click(playDeviceBtn);

    expect(within(playerBar).getByText("Airbag")).toBeInTheDocument();
    expect(within(playerBar).getByText("Device")).toBeInTheDocument();

    // 6. Safe Eject stops Device audio playback
    const ejectBtn = screen.getByRole("button", { name: /Eject Device/i });
    await user.click(ejectBtn);

    const playerBarAfterEject = await screen.findByRole("region", {
      name: /Audio Player/i,
    });
    expect(
      within(playerBarAfterEject).getByText(/No track playing/i),
    ).toBeInTheDocument();
  });
});
