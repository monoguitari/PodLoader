import { useCallback, useEffect, useMemo, useState } from "react";
import type {
  DeviceConnectionState,
  DevicePlaylist,
  DeviceTrack,
  PodLoaderBridge,
} from "./bridge";

export interface AppProps {
  bridge: PodLoaderBridge;
}

function formatGigabytes(bytes: number): string {
  return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
}

function formatDuration(durationMs: number): string {
  const totalSeconds = Math.floor(durationMs / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function formatStarRating(rating: number): string {
  const clamped = Math.max(0, Math.min(5, Math.round(rating)));
  return "★".repeat(clamped) + "☆".repeat(5 - clamped);
}

interface PlaylistSectionProps {
  title: string;
  playlists: DevicePlaylist[];
  selectedPlaylistId?: number;
  onSelect: (id: number) => void;
}

function PlaylistSection({
  title,
  playlists,
  selectedPlaylistId,
  onSelect,
}: PlaylistSectionProps) {
  return (
    <section>
      <h2>{title}</h2>
      <ul>
        {playlists.map((pl) => (
          <li key={pl.id}>
            <button
              type="button"
              aria-pressed={selectedPlaylistId === pl.id}
              onClick={() => onSelect(pl.id)}
            >
              {pl.name} ({pl.trackIds.length})
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

export function App({ bridge }: AppProps) {
  const [connection, setConnection] = useState<DeviceConnectionState>({
    status: "disconnected",
  });
  const [selectedPlaylistId, setSelectedPlaylistId] = useState<number | null>(
    null,
  );
  const [initDeviceName, setInitDeviceName] = useState<string>("My iPod");

  const applyConnectionState = useCallback((state: DeviceConnectionState) => {
    setConnection(state);
    if (state.status === "uninitializedVolume") {
      setInitDeviceName(state.data.volumeName || "My iPod");
    } else if (state.status === "connected") {
      const master =
        state.data.library.playlists.find((p) => p.kind === "master") ??
        state.data.library.playlists[0];
      if (master) {
        setSelectedPlaylistId(master.id);
      }
    }
  }, []);

  useEffect(() => {
    let active = true;
    bridge.scanDevice().then((state) => {
      if (active) {
        applyConnectionState(state);
      }
    });
    return () => {
      active = false;
    };
  }, [bridge, applyConnectionState]);

  const activeSession =
    connection.status === "connected" ? connection.data : null;

  const selectedPlaylist: DevicePlaylist | undefined = useMemo(() => {
    if (!activeSession) return undefined;
    return (
      activeSession.library.playlists.find(
        (p) => p.id === selectedPlaylistId,
      ) ?? activeSession.library.playlists[0]
    );
  }, [activeSession, selectedPlaylistId]);

  const orderedTracks: DeviceTrack[] = useMemo(() => {
    if (!activeSession || !selectedPlaylist) return [];
    const trackById = new Map<number, DeviceTrack>(
      activeSession.library.tracks.map((t) => [t.id, t]),
    );
    return selectedPlaylist.trackIds
      .map((id) => trackById.get(id))
      .filter((t): t is DeviceTrack => t !== undefined);
  }, [activeSession, selectedPlaylist]);

  if (connection.status === "disconnected") {
    return (
      <main>
        <h1>PodLoader</h1>
        <p>Connect a USB mass-storage Device to view its Device Library.</p>
      </main>
    );
  }

  if (connection.status === "uninitializedVolume") {
    const vol = connection.data;
    const handleInitialize = async () => {
      const nextState = await bridge.initializeDevice(
        vol.mountPath,
        initDeviceName.trim() || vol.volumeName || "My iPod",
      );
      applyConnectionState(nextState);
    };

    return (
      <main>
        <h1>PodLoader</h1>
        <p>Uninitialized volume detected: {vol.volumeName}</p>
        <p>
          Mount Path: {vol.mountPath} ({formatGigabytes(vol.freeBytes)} free)
        </p>
        <div>
          <label htmlFor="device-name-input">Device Name</label>
          <input
            id="device-name-input"
            type="text"
            value={initDeviceName}
            onChange={(e) => setInitDeviceName(e.target.value)}
          />
          <button type="button" onClick={handleInitialize}>
            Initialize Device
          </button>
        </div>
      </main>
    );
  }

  const { device, library } = connection.data;
  const masterPlaylists = library.playlists.filter((p) => p.kind === "master");
  const userPlaylists = library.playlists.filter((p) => p.kind === "user");
  const smartPlaylists = library.playlists.filter((p) => p.kind === "smart");

  return (
    <div className="podloader-shell">
      <header className="device-header">
        <div>
          <h1>{device.name}</h1>
          <p>
            {device.modelNumber && <span>Model: {device.modelNumber} </span>}
            {device.firewireGuid && (
              <span>FireWire GUID: {device.firewireGuid}</span>
            )}
          </p>
        </div>
        <div className="storage-summary">
          <span>{formatGigabytes(device.usedBytes)} used</span>
          <span> / </span>
          <span>{formatGigabytes(device.freeBytes)} free</span>
          <span> ({formatGigabytes(device.totalBytes)} total)</span>
        </div>
      </header>

      <div className="workspace">
        <nav aria-label="Device Playlists" className="playlist-sidebar">
          <PlaylistSection
            title="Master Playlist"
            playlists={masterPlaylists}
            selectedPlaylistId={selectedPlaylist?.id}
            onSelect={setSelectedPlaylistId}
          />
          <PlaylistSection
            title="Device Playlists"
            playlists={userPlaylists}
            selectedPlaylistId={selectedPlaylist?.id}
            onSelect={setSelectedPlaylistId}
          />
          <PlaylistSection
            title="Smart Playlists"
            playlists={smartPlaylists}
            selectedPlaylistId={selectedPlaylist?.id}
            onSelect={setSelectedPlaylistId}
          />
        </nav>

        <main className="track-pane">
          <table aria-label="Device Tracks">
            <thead>
              <tr>
                <th>Title</th>
                <th>Artist</th>
                <th>Album</th>
                <th>Track #</th>
                <th>Disc #</th>
                <th>Genre</th>
                <th>Year</th>
                <th>Duration</th>
                <th>Bitrate</th>
                <th>Play Count</th>
                <th>Star Rating</th>
              </tr>
            </thead>
            <tbody>
              {orderedTracks.map((track) => (
                <tr key={track.id}>
                  <td>{track.title}</td>
                  <td>{track.artist}</td>
                  <td>{track.album}</td>
                  <td>{track.trackNumber}</td>
                  <td>{track.discNumber}</td>
                  <td>{track.genre}</td>
                  <td>{track.year}</td>
                  <td>{formatDuration(track.durationMs)}</td>
                  <td>{track.bitrateKbps} kbps</td>
                  <td>{track.playCount}</td>
                  <td>{formatStarRating(track.rating)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </main>
      </div>
    </div>
  );
}
