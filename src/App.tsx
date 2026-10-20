import { useCallback, useEffect, useMemo, useState } from "react";
import type {
  DeviceConnectionState,
  DevicePlaylist,
  DeviceTrack,
  DuplicateResolution,
  HostDirectoryListing,
  InterruptedBatchSummary,
  InterruptedResolution,
  PodLoaderBridge,
  SyncBatch,
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
  onDropPaths?: (paths: string[], targetPlaylistId: number | null) => void;
}

function extractDroppedPaths(dataTransfer: DataTransfer): string[] {
  const custom = dataTransfer.getData("text/podloader-path");
  if (custom) {
    return custom
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);
  }
  const plain = dataTransfer.getData("text/plain");
  if (plain) {
    return plain
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);
  }
  const files = Array.from(dataTransfer.files ?? []);
  return files
    .map((f) => (f as File & { path?: string }).path || f.name)
    .filter(Boolean);
}

function PlaylistSection({
  title,
  playlists,
  selectedPlaylistId,
  onSelect,
  onDropPaths,
}: PlaylistSectionProps) {
  return (
    <section>
      <h2>{title}</h2>
      <ul>
        {playlists.map((pl) => (
          <li
            key={pl.id}
            onDragOver={(e) => {
              e.preventDefault();
            }}
            onDrop={(e) => {
              e.preventDefault();
              if (!onDropPaths) return;
              const paths = extractDroppedPaths(e.dataTransfer);
              if (paths.length > 0) {
                onDropPaths(paths, pl.kind === "user" ? pl.id : null);
              }
            }}
          >
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
  const [hostPathInput, setHostPathInput] = useState<string>("/music");
  const [hostListing, setHostListing] = useState<HostDirectoryListing | null>(
    null,
  );
  const [syncBatch, setSyncBatch] = useState<SyncBatch>({
    additions: [],
    requiredBytes: 0,
  });
  const [interruptedBatch, setInterruptedBatch] =
    useState<InterruptedBatchSummary | null>(null);
  const [manualGuidInput, setManualGuidInput] = useState<string>("");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const applyConnectionState = useCallback((state: DeviceConnectionState) => {
    setConnection(state);
    if (state.status === "uninitializedVolume") {
      setInitDeviceName(state.data.volumeName || "My iPod");
    } else if (state.status === "connected") {
      setSyncBatch(
        state.data.syncBatch ?? { additions: [], requiredBytes: 0 },
      );
      setInterruptedBatch(state.data.interruptedBatch ?? null);
      const master =
        state.data.library.playlists.find((p) => p.kind === "master") ??
        state.data.library.playlists[0];
      if (master) {
        setSelectedPlaylistId((prev) => prev ?? master.id);
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
    if (bridge.browseHostDirectory) {
      const browse = bridge.browseHostDirectory.bind(bridge);
      browse("/music")
        .catch(() => browse("."))
        .then((listing) => {
          if (active && listing && Array.isArray(listing.files)) {
            setHostListing(listing);
            setHostPathInput(listing.currentPath);
          }
        })
        .catch(() => {
          // Ignore initial browse error if default folder doesn't exist
        });
    }
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

  const handleBrowseHost = async (targetPath: string) => {
    if (!bridge.browseHostDirectory) return;
    try {
      const listing = await bridge.browseHostDirectory(targetPath);
      setHostListing(listing);
      setHostPathInput(listing.currentPath);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleStagePaths = async (
    paths: string[],
    explicitPlaylistId?: number | null,
  ) => {
    if (!bridge.stageHostPaths || paths.length === 0) return;
    try {
      const prevCount = syncBatch.additions.length;
      const targetId =
        explicitPlaylistId !== undefined
          ? explicitPlaylistId
          : selectedPlaylist && selectedPlaylist.kind === "user"
            ? selectedPlaylist.id
            : null;
      const updatedBatch = await bridge.stageHostPaths(paths, targetId);
      setSyncBatch(updatedBatch);
      if (updatedBatch.additions.length === prevCount) {
        setErrorMessage(
          "No new iPod-native audio files (MP3, M4A, AAC, ALAC, WAV) were found to stage in the selected path.",
        );
      } else {
        setErrorMessage(null);
      }
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleDuplicateResolutionChange = async (
    stagedId: number,
    resolution: DuplicateResolution,
  ) => {
    if (!bridge.setDuplicateResolution) return;
    const updatedBatch = await bridge.setDuplicateResolution(
      stagedId,
      resolution,
    );
    setSyncBatch(updatedBatch);
  };

  const handleSaveManualGuid = async () => {
    if (!bridge.setManualFirewireGuid || !manualGuidInput.trim()) return;
    try {
      const updatedSession = await bridge.setManualFirewireGuid(
        manualGuidInput.trim(),
      );
      applyConnectionState({ status: "connected", data: updatedSession });
      setManualGuidInput("");
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleCommitBatch = async (maxTracksBeforeInterrupt?: number) => {
    if (!bridge.commitSyncBatch) return;
    try {
      const outcome = await bridge.commitSyncBatch(
        maxTracksBeforeInterrupt !== undefined
          ? { maxTracksBeforeInterrupt }
          : undefined,
      );
      if (outcome.outcome === "completed") {
        applyConnectionState({ status: "connected", data: outcome.data });
      } else {
        setInterruptedBatch(outcome.data);
      }
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleResolveInterrupted = async (resolution: InterruptedResolution) => {
    if (!bridge.resolveInterruptedBatch) return;
    try {
      const updatedSession = await bridge.resolveInterruptedBatch(resolution);
      applyConnectionState({ status: "connected", data: updatedSession });
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

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
          <div className="guid-controls">
            <label htmlFor="manual-guid-input">Manual FireWire GUID</label>
            <input
              id="manual-guid-input"
              aria-label="Manual FireWire GUID"
              type="text"
              placeholder="16-char hex GUID"
              value={manualGuidInput}
              onChange={(e) => setManualGuidInput(e.target.value)}
            />
            <button type="button" onClick={handleSaveManualGuid}>
              Save GUID
            </button>
          </div>
        </div>
        <div className="storage-summary">
          <span>{formatGigabytes(device.usedBytes)} used</span>
          <span> / </span>
          <span>{formatGigabytes(device.freeBytes)} free</span>
          <span> ({formatGigabytes(device.totalBytes)} total)</span>
        </div>
      </header>

      {errorMessage && (
        <div role="alert" className="error-banner">
          {errorMessage}
        </div>
      )}

      <div className="workspace">
        <aside aria-label="Host File Browser" className="host-browser-pane">
          <h2>Host Files</h2>
          <div className="host-path-bar">
            <label htmlFor="host-path-input">Host Folder</label>
            <input
              id="host-path-input"
              type="text"
              value={hostPathInput}
              onChange={(e) => setHostPathInput(e.target.value)}
            />
            <button
              type="button"
              onClick={() => handleBrowseHost(hostPathInput)}
            >
              Browse
            </button>
            {hostListing?.parentPath && (
              <button
                type="button"
                onClick={() =>
                  hostListing.parentPath &&
                  handleBrowseHost(hostListing.parentPath)
                }
              >
                ⬆️ Parent Folder
              </button>
            )}
            {(hostListing?.files?.length ?? 0) > 0 && (
              <button
                type="button"
                onClick={() =>
                  handleStagePaths(
                    (hostListing?.files ?? []).map((f) => f.path),
                  )
                }
              >
                Stage All to Sync Batch
              </button>
            )}
          </div>

          {(hostListing?.subdirectories?.length ?? 0) > 0 && (
            <ul className="host-subdirs">
              {(hostListing?.subdirectories ?? []).map((dir) => (
                <li
                  key={dir.path}
                  draggable
                  onDragStart={(e) => {
                    e.dataTransfer.setData("text/podloader-path", dir.path);
                    e.dataTransfer.setData("text/plain", dir.path);
                  }}
                >
                  <button
                    type="button"
                    onClick={() => handleBrowseHost(dir.path)}
                  >
                    📁 {dir.name}
                  </button>
                  <button
                    type="button"
                    onClick={() => handleStagePaths([dir.path])}
                  >
                    Stage Folder
                  </button>
                </li>
              ))}
            </ul>
          )}

          <table aria-label="Host Files">
            <thead>
              <tr>
                <th>Track #</th>
                <th>Title</th>
                <th>Artist</th>
                <th>Album</th>
                <th>Duration</th>
                <th>Format</th>
                <th>Action</th>
              </tr>
            </thead>
            <tbody>
              {(hostListing?.files ?? []).map((hf) => (
                <tr
                  key={hf.path}
                  draggable
                  onDragStart={(e) => {
                    e.dataTransfer.setData("text/podloader-path", hf.path);
                    e.dataTransfer.setData("text/plain", hf.path);
                  }}
                >
                  <td>{hf.trackNumber}</td>
                  <td>{hf.title}</td>
                  <td>{hf.artist}</td>
                  <td>{hf.album}</td>
                  <td>{formatDuration(hf.durationMs)}</td>
                  <td>{hf.format}</td>
                  <td>
                    <button
                      type="button"
                      onClick={() => handleStagePaths([hf.path])}
                    >
                      Stage
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </aside>

        <nav aria-label="Device Playlists" className="playlist-sidebar">
          <PlaylistSection
            title="Master Playlist"
            playlists={masterPlaylists}
            selectedPlaylistId={selectedPlaylist?.id}
            onSelect={setSelectedPlaylistId}
            onDropPaths={(paths, targetId) =>
              handleStagePaths(paths, targetId)
            }
          />
          <PlaylistSection
            title="Device Playlists"
            playlists={userPlaylists}
            selectedPlaylistId={selectedPlaylist?.id}
            onSelect={setSelectedPlaylistId}
            onDropPaths={(paths, targetId) =>
              handleStagePaths(paths, targetId)
            }
          />
          <PlaylistSection
            title="Smart Playlists"
            playlists={smartPlaylists}
            selectedPlaylistId={selectedPlaylist?.id}
            onSelect={setSelectedPlaylistId}
          />
        </nav>

        <main
          className="track-pane"
          aria-label="Device Library"
          onDragOver={(e) => {
            e.preventDefault();
          }}
          onDrop={(e) => {
            e.preventDefault();
            const paths = extractDroppedPaths(e.dataTransfer);
            if (paths.length > 0) {
              handleStagePaths(paths);
            }
          }}
        >
          {syncBatch.additions.length > 0 && (
            <section aria-label="Sync Batch" className="sync-batch-panel">
              <h2>
                Sync Batch ({syncBatch.additions.length} staged,{" "}
                {formatGigabytes(syncBatch.requiredBytes)} required)
              </h2>

              {interruptedBatch && (
                <div role="alert" className="interrupted-batch-prompt">
                  <p>
                    Transfer paused: {interruptedBatch.reason} (
                    {interruptedBatch.completedTrackCount} completed,{" "}
                    {interruptedBatch.remainingTrackCount} remaining)
                  </p>
                  <button
                    type="button"
                    onClick={() =>
                      handleResolveInterrupted("commitCompleted")
                    }
                  >
                    Commit Completed Tracks
                  </button>
                  <button
                    type="button"
                    onClick={() => handleResolveInterrupted("rollBack")}
                  >
                    Roll Back Batch
                  </button>
                </div>
              )}

              <ul>
                {syncBatch.additions.map((item) => (
                  <li key={item.stagedId}>
                    <span>
                      {item.hostFile.artist} — {item.hostFile.title} (
                      {item.hostFile.format})
                    </span>
                    {item.isDuplicate && (
                      <>
                        <span className="duplicate-badge">Duplicate</span>
                        <select
                          aria-label={`Duplicate resolution for ${item.hostFile.title}`}
                          value={item.duplicateResolution}
                          onChange={(e) =>
                            handleDuplicateResolutionChange(
                              item.stagedId,
                              e.target.value as DuplicateResolution,
                            )
                          }
                        >
                          <option value="skip">Skip</option>
                          <option value="importAnyway">Import Anyway</option>
                          <option value="replaceExisting">
                            Replace Existing
                          </option>
                        </select>
                      </>
                    )}
                  </li>
                ))}
              </ul>

              <div className="sync-batch-actions">
                <button type="button" onClick={() => handleCommitBatch()}>
                  Commit Sync Batch
                </button>
                {import.meta.env.DEV && (
                  <button
                    type="button"
                    className="dev-only-button"
                    title="Developer test trigger: simulate mid-batch pause"
                    onClick={() => handleCommitBatch(1)}
                  >
                    Pause After First Track
                  </button>
                )}
              </div>
            </section>
          )}

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
