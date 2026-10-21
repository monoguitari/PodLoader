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
  TrackMetadataEdits,
  TranscodePreset,
  TransferProgress,
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
  canManage?: boolean;
  onRename?: (id: number, name: string) => void;
  onDelete?: (id: number) => void;
  onCreate?: (name: string) => void;
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
  canManage,
  onRename,
  onDelete,
  onCreate,
}: PlaylistSectionProps) {
  const [isCreating, setIsCreating] = useState(false);
  const [newPlaylistName, setNewPlaylistName] = useState("");
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");

  const handleStartCreate = () => {
    setIsCreating(true);
    setNewPlaylistName("");
  };

  const handleFinishCreate = () => {
    if (newPlaylistName.trim() && onCreate) {
      onCreate(newPlaylistName.trim());
    }
    setIsCreating(false);
    setNewPlaylistName("");
  };

  const handleStartRename = (pl: DevicePlaylist) => {
    setRenamingId(pl.id);
    setRenameValue(pl.name);
  };

  const handleFinishRename = (id: number) => {
    if (renameValue.trim() && onRename) {
      onRename(id, renameValue.trim());
    }
    setRenamingId(null);
    setRenameValue("");
  };

  return (
    <section>
      <h2>{title}</h2>
      {canManage && (
        <div className="playlist-create-bar">
          {!isCreating ? (
            <button type="button" onClick={handleStartCreate}>
              New Playlist
            </button>
          ) : (
            <div className="playlist-create-form">
              <input
                aria-label="New Playlist Name"
                placeholder="Playlist name"
                value={newPlaylistName}
                onChange={(e) => setNewPlaylistName(e.target.value)}
                autoFocus
              />
              <button type="button" onClick={handleFinishCreate}>
                Create Playlist
              </button>
              <button type="button" onClick={() => setIsCreating(false)}>
                Cancel
              </button>
            </div>
          )}
        </div>
      )}
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
            {renamingId === pl.id ? (
              <div className="playlist-rename-form">
                <input
                  aria-label="Rename Playlist"
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  autoFocus
                />
                <button type="button" onClick={() => handleFinishRename(pl.id)}>
                  Save Rename
                </button>
                <button type="button" onClick={() => setRenamingId(null)}>
                  Cancel
                </button>
              </div>
            ) : (
              <div className="playlist-item-row">
                <button
                  type="button"
                  aria-pressed={selectedPlaylistId === pl.id}
                  onClick={() => onSelect(pl.id)}
                >
                  {pl.name} ({pl.trackIds.length})
                </button>
                {canManage && (
                  <div className="playlist-item-actions">
                    <button
                      type="button"
                      aria-label="Rename"
                      title={`Rename ${pl.name}`}
                      onClick={() => handleStartRename(pl)}
                    >
                      Rename
                    </button>
                    <button
                      type="button"
                      aria-label="Delete"
                      title={`Delete ${pl.name}`}
                      onClick={() => onDelete && onDelete(pl.id)}
                    >
                      Delete
                    </button>
                  </div>
                )}
              </div>
            )}
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
  const [transferProgress, setTransferProgress] =
    useState<TransferProgress | null>(null);
  const [manualGuidInput, setManualGuidInput] = useState<string>("");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const [selectedTrackIds, setSelectedTrackIds] = useState<number[]>([]);
  const [showEditModal, setShowEditModal] = useState<boolean>(false);
  const [editTitle, setEditTitle] = useState<string>("");
  const [editArtist, setEditArtist] = useState<string>("");
  const [editAlbum, setEditAlbum] = useState<string>("");
  const [editAlbumArtist, setEditAlbumArtist] = useState<string>("");
  const [editTrackNumber, setEditTrackNumber] = useState<string>("");
  const [editDiscNumber, setEditDiscNumber] = useState<string>("");
  const [editGenre, setEditGenre] = useState<string>("");
  const [editYear, setEditYear] = useState<string>("");
  const [editRating, setEditRating] = useState<string>("");
  const [editIsCompilation, setEditIsCompilation] = useState<boolean>(false);
  const [compilationModified, setCompilationModified] = useState<boolean>(false);
  const [targetPlaylistId, setTargetPlaylistId] = useState<number | null>(null);

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
          "No supported audio files (MP3, M4A, AAC, ALAC, WAV, FLAC, OGG, Opus) were found to stage in the selected path.",
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

  const handleTranscodePresetChange = async (preset: TranscodePreset) => {
    if (!bridge.setTranscodePreset) return;
    try {
      const updatedSession = await bridge.setTranscodePreset(preset);
      applyConnectionState({ status: "connected", data: updatedSession });
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleCommitBatch = async (maxTracksBeforeInterrupt?: number) => {
    if (!bridge.commitSyncBatch) return;
    try {
      const outcome = await bridge.commitSyncBatch({
        ...(maxTracksBeforeInterrupt !== undefined
          ? { maxTracksBeforeInterrupt }
          : {}),
        onProgress: (progress) => {
          setTransferProgress(progress);
        },
      });
      setTransferProgress(null);
      if (outcome.outcome === "completed") {
        applyConnectionState({ status: "connected", data: outcome.data });
      } else {
        setInterruptedBatch(outcome.data);
      }
      setErrorMessage(null);
    } catch (err) {
      setTransferProgress(null);
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

  const openEditModal = () => {
    if (selectedTrackIds.length === 0 || connection.status !== "connected") return;
    if (selectedTrackIds.length === 1) {
      const tr = connection.data.library.tracks.find((t) => t.id === selectedTrackIds[0]);
      if (tr) {
        setEditTitle(tr.title);
        setEditArtist(tr.artist);
        setEditAlbum(tr.album);
        setEditAlbumArtist(tr.albumArtist ?? tr.artist);
        setEditTrackNumber(tr.trackNumber ? String(tr.trackNumber) : "");
        setEditDiscNumber(tr.discNumber ? String(tr.discNumber) : "");
        setEditGenre(tr.genre ?? "");
        setEditYear(tr.year ? String(tr.year) : "");
        const normalizedRating =
          tr.rating !== undefined
            ? tr.rating > 5
              ? Math.round(tr.rating / 20)
              : tr.rating
            : 0;
        setEditRating(String(Math.max(0, Math.min(5, normalizedRating))));
        setEditIsCompilation(Boolean(tr.isCompilation));
      }
    } else {
      setEditTitle("");
      setEditArtist("");
      setEditAlbum("");
      setEditAlbumArtist("");
      setEditTrackNumber("");
      setEditDiscNumber("");
      setEditGenre("");
      setEditYear("");
      setEditRating("");
      setEditIsCompilation(false);
    }
    setCompilationModified(false);
    setShowEditModal(true);
  };

  const handleSaveMetadataEdits = async () => {
    if (!bridge.stageTrackMetadataEdits || selectedTrackIds.length === 0) return;
    const edits: TrackMetadataEdits = {};
    if (editTitle.trim()) edits.title = editTitle.trim();
    if (editArtist.trim()) edits.artist = editArtist.trim();
    if (editAlbum.trim()) edits.album = editAlbum.trim();
    if (editAlbumArtist.trim()) edits.albumArtist = editAlbumArtist.trim();
    if (editTrackNumber.trim()) edits.trackNumber = parseInt(editTrackNumber, 10);
    if (editDiscNumber.trim()) edits.discNumber = parseInt(editDiscNumber, 10);
    if (editGenre.trim()) edits.genre = editGenre.trim();
    if (editYear.trim()) edits.year = parseInt(editYear, 10);
    if (editRating.trim()) edits.rating = Math.max(0, Math.min(5, parseInt(editRating, 10)));
    if (selectedTrackIds.length === 1 || compilationModified) {
      edits.isCompilation = editIsCompilation;
    }

    const payload = selectedTrackIds.map((tid) => ({
      trackId: tid,
      edits,
    }));
    try {
      const updatedBatch = await bridge.stageTrackMetadataEdits(payload);
      setSyncBatch(updatedBatch);
      setShowEditModal(false);
      setSelectedTrackIds([]);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleAddToPlaylist = async (playlistId: number) => {
    if (!bridge.addTracksToPlaylist || selectedTrackIds.length === 0) return;
    try {
      await bridge.addTracksToPlaylist(playlistId, selectedTrackIds);
      setSelectedTrackIds([]);
      const updated = await bridge.scanDevice();
      applyConnectionState(updated);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleStageRemovals = async () => {
    if (!bridge.stageTrackRemovals || selectedTrackIds.length === 0) return;
    try {
      const updatedBatch = await bridge.stageTrackRemovals(selectedTrackIds);
      setSyncBatch(updatedBatch);
      setSelectedTrackIds([]);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleCreatePlaylist = async (name: string) => {
    if (!bridge.createDevicePlaylist || !name.trim()) return;
    try {
      const created = await bridge.createDevicePlaylist(name.trim());
      const updated = await bridge.scanDevice();
      applyConnectionState(updated);
      setSelectedPlaylistId(created.id);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleRenamePlaylist = async (id: number, newName: string) => {
    if (!bridge.renameDevicePlaylist || !newName.trim()) return;
    try {
      await bridge.renameDevicePlaylist(id, newName.trim());
      const updated = await bridge.scanDevice();
      applyConnectionState(updated);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleDeletePlaylist = async (id: number) => {
    if (!bridge.deleteDevicePlaylist) return;
    try {
      await bridge.deleteDevicePlaylist(id);
      const updated = await bridge.scanDevice();
      applyConnectionState(updated);
      if (selectedPlaylistId === id) {
        setSelectedPlaylistId(null);
      }
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleMoveTrack = async (fromIdx: number, toIdx: number) => {
    if (
      !bridge.reorderPlaylistTracks ||
      !selectedPlaylist ||
      selectedPlaylist.kind !== "user"
    ) {
      return;
    }
    const currentIds = [...selectedPlaylist.trackIds];
    if (
      fromIdx < 0 ||
      fromIdx >= currentIds.length ||
      toIdx < 0 ||
      toIdx >= currentIds.length
    ) {
      return;
    }
    const [moved] = currentIds.splice(fromIdx, 1);
    currentIds.splice(toIdx, 0, moved);
    try {
      await bridge.reorderPlaylistTracks(selectedPlaylist.id, currentIds);
      const updated = await bridge.scanDevice();
      applyConnectionState(updated);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleRemoveFromCurrentPlaylist = async () => {
    if (
      !bridge.removeTracksFromPlaylist ||
      !selectedPlaylist ||
      selectedPlaylist.kind !== "user" ||
      selectedTrackIds.length === 0
    ) {
      return;
    }
    try {
      await bridge.removeTracksFromPlaylist(
        selectedPlaylist.id,
        selectedTrackIds,
      );
      setSelectedTrackIds([]);
      const updated = await bridge.scanDevice();
      applyConnectionState(updated);
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
          <div className="transcode-preset-controls">
            <label htmlFor="transcode-preset-select">Transcode Preset</label>
            <select
              id="transcode-preset-select"
              aria-label="Transcode Preset"
              value={connection.data.transcodePreset ?? "alac"}
              onChange={(e) =>
                handleTranscodePresetChange(e.target.value as TranscodePreset)
              }
            >
              <option value="alac">Lossless ALAC (Pure Rust)</option>
              <option
                value="aac"
                disabled={connection.data.ffmpegAvailable === false}
              >
                Lossy AAC (ffmpeg)
              </option>
              <option
                value="mp3"
                disabled={connection.data.ffmpegAvailable === false}
              >
                Lossy MP3 (ffmpeg)
              </option>
            </select>
            {connection.data.ffmpegAvailable === false && (
              <span className="ffmpeg-unavailable-hint">
                ffmpeg not detected on PATH; lossy AAC/MP3 presets disabled
              </span>
            )}
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
            canManage
            onCreate={handleCreatePlaylist}
            onRename={handleRenamePlaylist}
            onDelete={handleDeletePlaylist}
          />
          <PlaylistSection
            title="Smart Playlists"
            playlists={smartPlaylists}
            selectedPlaylistId={selectedPlaylist?.id}
            onSelect={setSelectedPlaylistId}
          />
        </nav>

        {(() => {
          const hasStagedItems =
            syncBatch.additions.length > 0 ||
            (syncBatch.edits && syncBatch.edits.length > 0) ||
            (syncBatch.removals && syncBatch.removals.length > 0);
          const totalStagedCount =
            syncBatch.additions.length +
            (syncBatch.edits?.length ?? 0) +
            (syncBatch.removals?.length ?? 0);

          return (
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
          {hasStagedItems && (
            <section aria-label="Sync Batch" className="sync-batch-panel">
              <h2>
                Sync Batch ({totalStagedCount} staged
                {syncBatch.requiredBytes > 0
                  ? `, ${formatGigabytes(syncBatch.requiredBytes)} required`
                  : ""})
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

              {transferProgress && (
                <div role="status" className="transfer-progress-banner">
                  <p>
                    {transferProgress.isTranscoding
                      ? "Transcoding"
                      : "Transferring"}{" "}
                    Track {transferProgress.currentTrackIndex} of{" "}
                    {transferProgress.totalTracks}:{" "}
                    {transferProgress.currentTrackTitle} (
                    {transferProgress.trackPercent}%)
                  </p>
                  <p>Overall Progress: {transferProgress.overallPercent}%</p>
                </div>
              )}

              <ul>
                {syncBatch.additions.map((item) => {
                  const targetFmt = (
                    item.targetFormat ??
                    connection.data.transcodePreset ??
                    "alac"
                  ).toUpperCase();
                  return (
                    <li key={`addition-${item.stagedId}`}>
                      <span>
                        {item.hostFile.artist} — {item.hostFile.title} (
                        {item.requiresTranscode
                          ? `${item.hostFile.format} → ${targetFmt}`
                          : item.hostFile.format}
                        )
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
                  );
                })}
                {syncBatch.edits?.map((item) => {
                  const tr =
                    connection.status === "connected"
                      ? connection.data.library.tracks.find(
                          (t) => t.id === item.trackId,
                        )
                      : null;
                  const label =
                    item.edits.title ?? tr?.title ?? `Track #${item.trackId}`;
                  return (
                    <li key={`edit-${item.stagedId}`}>
                      <span>Edit: {label}</span>
                    </li>
                  );
                })}
                {syncBatch.removals?.map((item) => (
                  <li key={`removal-${item.stagedId}`}>
                    <span>
                      Remove: {item.trackTitle} ({item.trackArtist})
                    </span>
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

          <div
            className="track-toolbar"
            role="toolbar"
            aria-label="Track Actions"
          >
            <button
              type="button"
              disabled={selectedTrackIds.length === 0}
              onClick={openEditModal}
            >
              Edit Metadata
            </button>
            <button
              type="button"
              disabled={selectedTrackIds.length === 0}
              onClick={handleStageRemovals}
            >
              Remove from iPod
            </button>
            {selectedPlaylist?.kind === "user" && (
              <button
                type="button"
                disabled={selectedTrackIds.length === 0}
                onClick={handleRemoveFromCurrentPlaylist}
              >
                Remove from Playlist
              </button>
            )}
            {userPlaylists.length > 0 && (
              <div className="add-to-playlist-group">
                <select
                  aria-label="Target Playlist"
                  value={targetPlaylistId ?? userPlaylists[0]?.id}
                  onChange={(e) => setTargetPlaylistId(Number(e.target.value))}
                >
                  {userPlaylists.map((pl) => (
                    <option key={pl.id} value={pl.id}>
                      {pl.name}
                    </option>
                  ))}
                </select>
                <button
                  type="button"
                  disabled={selectedTrackIds.length === 0}
                  onClick={() =>
                    handleAddToPlaylist(
                      targetPlaylistId ?? userPlaylists[0]?.id,
                    )
                  }
                >
                  Add to Playlist
                </button>
              </div>
            )}
          </div>

          <table aria-label="Device Tracks">
            <thead>
              <tr>
                <th>
                  <input
                    type="checkbox"
                    aria-label="Select all tracks"
                    checked={
                      orderedTracks.length > 0 &&
                      selectedTrackIds.length === orderedTracks.length
                    }
                    onChange={(e) => {
                      if (e.target.checked) {
                        setSelectedTrackIds(orderedTracks.map((t) => t.id));
                      } else {
                        setSelectedTrackIds([]);
                      }
                    }}
                  />
                </th>
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
                {selectedPlaylist?.kind === "user" && <th>Order</th>}
              </tr>
            </thead>
            <tbody>
              {orderedTracks.map((track, idx) => (
                <tr key={track.id}>
                  <td>
                    <input
                      type="checkbox"
                      aria-label={`Select ${track.title}`}
                      checked={selectedTrackIds.includes(track.id)}
                      onChange={(e) => {
                        if (e.target.checked) {
                          setSelectedTrackIds((prev) => [...prev, track.id]);
                        } else {
                          setSelectedTrackIds((prev) =>
                            prev.filter((id) => id !== track.id),
                          );
                        }
                      }}
                    />
                  </td>
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
                  {selectedPlaylist?.kind === "user" && (
                    <td>
                      <button
                        type="button"
                        aria-label={`Move Up ${track.title}`}
                        disabled={idx === 0}
                        onClick={() => handleMoveTrack(idx, idx - 1)}
                      >
                        ↑
                      </button>
                      <button
                        type="button"
                        aria-label={`Move Down ${track.title}`}
                        disabled={idx === orderedTracks.length - 1}
                        onClick={() => handleMoveTrack(idx, idx + 1)}
                      >
                        ↓
                      </button>
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </main>
      );
    })()}
      </div>

      {showEditModal && (
        <div
          role="dialog"
          aria-label="Edit Track Metadata"
          className="modal-overlay"
        >
          <div className="modal-content">
            <h3>
              Edit Track Metadata{" "}
              {selectedTrackIds.length > 1
                ? `(${selectedTrackIds.length} tracks)`
                : ""}
            </h3>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                handleSaveMetadataEdits();
              }}
            >
              <div>
                <label htmlFor="edit-title">Title</label>
                <input
                  id="edit-title"
                  aria-label="Title"
                  value={editTitle}
                  onChange={(e) => setEditTitle(e.target.value)}
                />
              </div>
              <div>
                <label htmlFor="edit-artist">Artist</label>
                <input
                  id="edit-artist"
                  aria-label="Artist"
                  value={editArtist}
                  onChange={(e) => setEditArtist(e.target.value)}
                />
              </div>
              <div>
                <label htmlFor="edit-album">Album</label>
                <input
                  id="edit-album"
                  aria-label="Album"
                  value={editAlbum}
                  onChange={(e) => setEditAlbum(e.target.value)}
                />
              </div>
              <div>
                <label htmlFor="edit-album-artist">Album Artist</label>
                <input
                  id="edit-album-artist"
                  aria-label="Album Artist"
                  value={editAlbumArtist}
                  onChange={(e) => setEditAlbumArtist(e.target.value)}
                />
              </div>
              <div>
                <label htmlFor="edit-track-number">Track Number</label>
                <input
                  id="edit-track-number"
                  aria-label="Track Number"
                  type="number"
                  value={editTrackNumber}
                  onChange={(e) => setEditTrackNumber(e.target.value)}
                />
              </div>
              <div>
                <label htmlFor="edit-disc-number">Disc Number</label>
                <input
                  id="edit-disc-number"
                  aria-label="Disc Number"
                  type="number"
                  value={editDiscNumber}
                  onChange={(e) => setEditDiscNumber(e.target.value)}
                />
              </div>
              <div>
                <label htmlFor="edit-genre">Genre</label>
                <input
                  id="edit-genre"
                  aria-label="Genre"
                  value={editGenre}
                  onChange={(e) => setEditGenre(e.target.value)}
                />
              </div>
              <div>
                <label htmlFor="edit-year">Year</label>
                <input
                  id="edit-year"
                  aria-label="Year"
                  type="number"
                  value={editYear}
                  onChange={(e) => setEditYear(e.target.value)}
                />
              </div>
              <div>
                <label htmlFor="edit-rating">Rating (0-5 stars)</label>
                <input
                  id="edit-rating"
                  aria-label="Rating"
                  type="number"
                  min="0"
                  max="5"
                  placeholder="0 to 5"
                  value={editRating}
                  onChange={(e) => setEditRating(e.target.value)}
                />
              </div>
              <div>
                <label htmlFor="edit-is-compilation">Compilation</label>
                <input
                  id="edit-is-compilation"
                  aria-label="Compilation"
                  type="checkbox"
                  checked={editIsCompilation}
                  onChange={(e) => {
                    setEditIsCompilation(e.target.checked);
                    setCompilationModified(true);
                  }}
                />
              </div>
              <div className="modal-actions">
                <button type="submit">Stage Edits</button>
                <button
                  type="button"
                  onClick={() => setShowEditModal(false)}
                >
                  Cancel
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
}
