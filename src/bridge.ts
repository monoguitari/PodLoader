export interface MountedVolume {
  mountPath: string;
  volumeName: string;
  totalBytes: number;
  freeBytes: number;
  usbFirewireGuid?: string | null;
}

export interface DeviceMetadata {
  mountPath: string;
  name: string;
  modelNumber?: string | null;
  firewireGuid?: string | null;
  totalBytes: number;
  freeBytes: number;
  usedBytes: number;
}

export interface DeviceTrack {
  id: number;
  title: string;
  artist: string;
  album: string;
  trackNumber: number;
  discNumber: number;
  genre: string;
  year: number;
  durationMs: number;
  bitrateKbps: number;
  playCount: number;
  rating: number;
  lastPlayedTimestamp?: number | null;
  bookmarkMs: number;
  obfuscatedPath: string;
}

export type PlaylistKind = "master" | "user" | "smart";

export interface DevicePlaylist {
  id: number;
  name: string;
  kind: PlaylistKind;
  trackIds: number[];
}

export interface DeviceLibrary {
  tracks: DeviceTrack[];
  playlists: DevicePlaylist[];
}

export interface HostDirectoryEntry {
  name: string;
  path: string;
}

export interface HostFile {
  path: string;
  fileName: string;
  format: string;
  fileSize: number;
  title: string;
  artist: string;
  album: string;
  trackNumber: number;
  discNumber: number;
  genre: string;
  year: number;
  durationMs: number;
  bitrateKbps: number;
}

export interface HostDirectoryListing {
  currentPath: string;
  parentPath?: string | null;
  subdirectories: HostDirectoryEntry[];
  files: HostFile[];
}

export type DuplicateResolution = "skip" | "importAnyway" | "replaceExisting";

export interface StagedTrackAddition {
  stagedId: number;
  hostFile: HostFile;
  targetPlaylistId?: number | null;
  isDuplicate: boolean;
  matchedDeviceTrackId?: number | null;
  duplicateResolution: DuplicateResolution;
}

export interface SyncBatch {
  additions: StagedTrackAddition[];
  requiredBytes: number;
}

export interface InterruptedBatchSummary {
  completedTrackCount: number;
  remainingTrackCount: number;
  writtenObfuscatedPaths: string[];
  reason: string;
}

export type InterruptedResolution = "commitCompleted" | "rollBack";

export interface DeviceSession {
  device: DeviceMetadata;
  library: DeviceLibrary;
  hasPendingPlayCounts: boolean;
  syncBatch?: SyncBatch;
  interruptedBatch?: InterruptedBatchSummary | null;
}

export type CommitOutcome =
  | { outcome: "completed"; data: DeviceSession }
  | { outcome: "interrupted"; data: InterruptedBatchSummary };

export type DeviceConnectionState =
  | { status: "disconnected" }
  | { status: "uninitializedVolume"; data: MountedVolume }
  | { status: "connected"; data: DeviceSession };

export interface PodLoaderBridge {
  scanDevice(): Promise<DeviceConnectionState>;
  initializeDevice(
    mountPath: string,
    deviceName: string,
  ): Promise<DeviceConnectionState>;
  browseHostDirectory?(dirPath: string): Promise<HostDirectoryListing>;
  stageHostPaths?(
    paths: string[],
    targetPlaylistId?: number | null,
  ): Promise<SyncBatch>;
  setDuplicateResolution?(
    stagedId: number,
    resolution: DuplicateResolution,
  ): Promise<SyncBatch>;
  commitSyncBatch?(options?: {
    maxTracksBeforeInterrupt?: number;
  }): Promise<CommitOutcome>;
  resolveInterruptedBatch?(
    resolution: InterruptedResolution,
  ): Promise<DeviceSession>;
  setManualFirewireGuid?(guid: string): Promise<DeviceSession>;
}
