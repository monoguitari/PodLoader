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

export interface DeviceSession {
  device: DeviceMetadata;
  library: DeviceLibrary;
  hasPendingPlayCounts: boolean;
}

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
}
