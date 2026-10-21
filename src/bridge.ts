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
  albumArtist?: string | null;
  isCompilation?: boolean;
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

export type TranscodePreset = "alac" | "aac" | "mp3";

export interface StagedTrackAddition {
  stagedId: number;
  hostFile: HostFile;
  targetPlaylistId?: number | null;
  isDuplicate: boolean;
  matchedDeviceTrackId?: number | null;
  duplicateResolution: DuplicateResolution;
  requiresTranscode?: boolean;
  targetFormat?: TranscodePreset | null;
}

export interface TransferProgress {
  currentTrackIndex: number;
  totalTracks: number;
  currentTrackTitle: string;
  isTranscoding: boolean;
  trackPercent: number;
  overallPercent: number;
}

export interface TrackMetadataEdits {
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  albumArtist?: string | null;
  trackNumber?: number | null;
  discNumber?: number | null;
  genre?: string | null;
  year?: number | null;
  isCompilation?: boolean | null;
  rating?: number | null;
}

export interface StagedMetadataEdit {
  stagedId: number;
  trackId: number;
  edits: TrackMetadataEdits;
}

export interface StagedTrackRemoval {
  stagedId: number;
  trackId: number;
  trackTitle: string;
  trackArtist: string;
}

export interface OrphanFile {
  path: string;
  obfuscatedPath: string;
  fileName: string;
  fileSize: number;
  format: string;
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

export interface StagedOrphanRecovery {
  stagedId: number;
  orphanFile: OrphanFile;
}

export interface StagedOrphanDeletion {
  stagedId: number;
  orphanFile: OrphanFile;
}

export interface SyncBatch {
  additions: StagedTrackAddition[];
  edits?: StagedMetadataEdit[];
  removals?: StagedTrackRemoval[];
  orphanRecoveries?: StagedOrphanRecovery[];
  orphanDeletions?: StagedOrphanDeletion[];
  requiredBytes: number;
}

export interface InterruptedBatchSummary {
  completedTrackCount: number;
  remainingTrackCount: number;
  writtenObfuscatedPaths: string[];
  reason: string;
}

export type InterruptedResolution = "commitCompleted" | "rollBack";

export interface ExportResult {
  exportedCount: number;
  exportedFiles: string[];
}

export type AudioSourceKind = "host" | "device";
export type PlaybackState = "stopped" | "playing" | "paused";

export interface ActivePlaybackTrack {
  sourceKind: AudioSourceKind;
  sourcePath: string;
  title: string;
  artist: string;
  album: string;
  durationMs: number;
  trackId?: number | null;
}

export interface PlaybackStatus {
  state: PlaybackState;
  track?: ActivePlaybackTrack | null;
  elapsedMs: number;
  durationMs: number;
  volume: number;
}

export interface DeviceSession {
  device: DeviceMetadata;
  library: DeviceLibrary;
  hasPendingPlayCounts: boolean;
  syncBatch?: SyncBatch;
  interruptedBatch?: InterruptedBatchSummary | null;
  transcodePreset?: TranscodePreset;
  ffmpegAvailable?: boolean;
  orphanFiles?: OrphanFile[];
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
    onProgress?: (progress: TransferProgress) => void;
  }): Promise<CommitOutcome>;
  resolveInterruptedBatch?(
    resolution: InterruptedResolution,
  ): Promise<DeviceSession>;
  setManualFirewireGuid?(guid: string): Promise<DeviceSession>;
  setTranscodePreset?(preset: TranscodePreset): Promise<DeviceSession>;
  stageTrackMetadataEdits?(
    edits: Array<{ trackId: number; edits: TrackMetadataEdits }>,
  ): Promise<SyncBatch>;
  stageTrackRemovals?(trackIds: number[]): Promise<SyncBatch>;
  createDevicePlaylist?(name: string): Promise<DevicePlaylist>;
  addTracksToPlaylist?(
    playlistId: number,
    trackIds: number[],
  ): Promise<DevicePlaylist>;
  reorderPlaylistTracks?(
    playlistId: number,
    trackIds: number[],
  ): Promise<DevicePlaylist>;
  removeTracksFromPlaylist?(
    playlistId: number,
    trackIds: number[],
  ): Promise<DevicePlaylist>;
  renameDevicePlaylist?(
    playlistId: number,
    newName: string,
  ): Promise<DevicePlaylist>;
  deleteDevicePlaylist?(playlistId: number): Promise<void>;
  stageOrphanRecovery?(obfuscatedPath: string): Promise<SyncBatch>;
  stageOrphanDeletion?(obfuscatedPath: string): Promise<SyncBatch>;
  exportDeviceTracks?(
    trackIds: number[],
    destinationDir: string,
  ): Promise<ExportResult>;
  exportDevicePlaylist?(
    playlistId: number,
    destinationDir: string,
  ): Promise<ExportResult>;
  ejectDevice?(force?: boolean): Promise<void>;
  playHostFile?(hostFile: HostFile): Promise<PlaybackStatus>;
  playDeviceTrack?(trackId: number): Promise<PlaybackStatus>;
  pausePlayback?(): Promise<PlaybackStatus>;
  resumePlayback?(): Promise<PlaybackStatus>;
  stopPlayback?(): Promise<PlaybackStatus>;
  seekPlayback?(positionMs: number): Promise<PlaybackStatus>;
  setPlaybackVolume?(volume: number): Promise<PlaybackStatus>;
  getPlaybackStatus?(): Promise<PlaybackStatus>;
}
