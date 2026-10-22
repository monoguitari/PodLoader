import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import "./index.css";
import { logger, LogEntry } from "./debugLogger";
import type {
  DeviceConnectionState,
  DevicePlaylist,
  DeviceTrack,
  DuplicateResolution,
  HostDirectoryListing,
  HostFile,
  InterruptedBatchSummary,
  InterruptedResolution,
  PlaybackStatus,
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
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [playbackStatus, setPlaybackStatus] = useState<PlaybackStatus>({
    state: "stopped",
    track: null,
    elapsedMs: 0,
    durationMs: 0,
    volume: 1.0,
  });
  const [showEjectWarning, setShowEjectWarning] = useState<boolean>(false);
  const [exportDestination, setExportDestination] = useState<string>(
    "/music/PodLoader_Export",
  );
  const [exportSuccessMessage, setExportSuccessMessage] = useState<string | null>(
    null,
  );
  const [loadingTrackId, setLoadingTrackId] = useState<number | null>(null);
  const [loadingHostPath, setLoadingHostPath] = useState<string | null>(null);
  const [isAudioBuffering, setIsAudioBuffering] = useState<boolean>(false);
  const [isEjecting, setIsEjecting] = useState<boolean>(false);
  const [isBrowsingHost, setIsBrowsingHost] = useState<boolean>(false);
  const [isExporting, setIsExporting] = useState<boolean>(false);
  const [isCommitting, setIsCommitting] = useState<boolean>(false);
  const [isInitializing, setIsInitializing] = useState<boolean>(false);
  const [searchQuery, setSearchQuery] = useState<string>("");
  const [isHostBrowserCollapsed, setIsHostBrowserCollapsed] =
    useState<boolean>(false);
  const [isDebugOpen, setIsDebugOpen] = useState<boolean>(false);
  const [logs, setLogs] = useState<LogEntry[]>(() => logger.getLogs());
  const [logFilter, setLogFilter] = useState<string>("ALL");
  const [logSearch, setLogSearch] = useState<string>("");
  const [isAutoScroll, setIsAutoScroll] = useState<boolean>(true);
  const [copiedLogs, setCopiedLogs] = useState<boolean>(false);
  const logsContainerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    return logger.subscribe(() => {
      setLogs([...logger.getLogs()]);
    });
  }, []);

  useEffect(() => {
    if (isAutoScroll && logsContainerRef.current) {
      logsContainerRef.current.scrollTop = logsContainerRef.current.scrollHeight;
    }
  }, [logs, isAutoScroll]);

  const filteredLogs = useMemo(() => {
    return logs.filter((item) => {
      if (logFilter === "ERROR" && item.level !== "error") return false;
      if (
        logFilter !== "ALL" &&
        logFilter !== "ERROR" &&
        item.category !== logFilter
      )
        return false;
      if (logSearch.trim()) {
        const q = logSearch.toLowerCase().trim();
        const matchMsg = item.message.toLowerCase().includes(q);
        const matchCat = item.category.toLowerCase().includes(q);
        const matchDet = item.details
          ? JSON.stringify(item.details).toLowerCase().includes(q)
          : false;
        return matchMsg || matchCat || matchDet;
      }
      return true;
    });
  }, [logs, logFilter, logSearch]);

  useEffect(() => {
    if (playbackStatus.state !== "playing") return;
    const interval = window.setInterval(() => {
      setPlaybackStatus((prev) => {
        if (prev.state !== "playing") return prev;
        if (
          audioRef.current &&
          !isNaN(audioRef.current.currentTime) &&
          audioRef.current.currentTime > 0
        ) {
          const curMs = Math.round(audioRef.current.currentTime * 1000);
          return {
            ...prev,
            elapsedMs: curMs,
          };
        }
        const nextElapsed = prev.elapsedMs + 250;
        if (prev.durationMs > 0 && nextElapsed >= prev.durationMs) {
          return {
            ...prev,
            state: "stopped",
            elapsedMs: prev.durationMs,
          };
        }
        return {
          ...prev,
          elapsedMs: nextElapsed,
        };
      });
    }, 250);
    return () => window.clearInterval(interval);
  }, [playbackStatus.state]);

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

  const filteredTracks: DeviceTrack[] = useMemo(() => {
    if (!searchQuery.trim()) return orderedTracks;
    const q = searchQuery.toLowerCase().trim();
    return orderedTracks.filter(
      (t) =>
        t.title.toLowerCase().includes(q) ||
        t.artist.toLowerCase().includes(q) ||
        t.album.toLowerCase().includes(q) ||
        t.genre.toLowerCase().includes(q) ||
        String(t.year).includes(q),
    );
  }, [orderedTracks, searchQuery]);

  const safePlayAudio = (audio: HTMLAudioElement | null) => {
    if (!audio) return;
    try {
      const res = audio.play();
      if (res && typeof res.catch === "function") {
        res.catch((err) => {
          if (err && err.name !== "AbortError") {
            logger.audioWarn("Audio play() error:", err.message || err);
          }
        });
      }
    } catch (err) {
      logger.audioWarn("Audio play() synchronous error:", err);
    }
  };

  const safePauseAudio = (audio: HTMLAudioElement | null) => {
    if (!audio) return;
    try {
      audio.pause();
    } catch (err) {
      logger.audioWarn("Audio pause error:", err);
    }
  };

  const handleRowClick = (trackId: number, e: React.MouseEvent) => {
    logger.ui(`Row click on track ID=${trackId}`);
    if (e.ctrlKey || e.metaKey) {
      setSelectedTrackIds((prev) =>
        prev.includes(trackId)
          ? prev.filter((id) => id !== trackId)
          : [...prev, trackId],
      );
    } else if (e.shiftKey && selectedTrackIds.length > 0) {
      const lastSelectedId = selectedTrackIds[selectedTrackIds.length - 1];
      const lastIdx = filteredTracks.findIndex((t) => t.id === lastSelectedId);
      const curIdx = filteredTracks.findIndex((t) => t.id === trackId);
      if (lastIdx !== -1 && curIdx !== -1) {
        const [start, end] = [
          Math.min(lastIdx, curIdx),
          Math.max(lastIdx, curIdx),
        ];
        const rangeIds = filteredTracks.slice(start, end + 1).map((t) => t.id);
        const union = Array.from(new Set([...selectedTrackIds, ...rangeIds]));
        setSelectedTrackIds(union);
      }
    } else {
      setSelectedTrackIds([trackId]);
    }
  };

  const handlePlayHostFile = async (hostFile: HostFile) => {
    logger.ui(`Play Host File: ${hostFile.path}`);
    if (!bridge.playHostFile) return;
    setLoadingHostPath(hostFile.path);
    setIsAudioBuffering(true);
    try {
      const status = await bridge.playHostFile(hostFile);
      logger.ipc("playHostFile response", status);
      setPlaybackStatus(status);
      setErrorMessage(null);
      if (audioRef.current) {
        const srcUrl = `/api/audio_stream?path=${encodeURIComponent(hostFile.path)}&t=${Date.now()}`;
        logger.audio(`Set audio.src to ${srcUrl}`);
        audioRef.current.src = srcUrl;
        audioRef.current.volume = status.volume;
        safePlayAudio(audioRef.current);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      logger.ipcError(`playHostFile error: ${msg}`);
      setErrorMessage(msg);
      setLoadingHostPath(null);
      setIsAudioBuffering(false);
    }
  };

  const handlePlayDeviceTrack = async (trackId: number) => {
    logger.ui(`Play Device Track: id=${trackId}`);
    if (!bridge.playDeviceTrack) return;
    setLoadingTrackId(trackId);
    setIsAudioBuffering(true);
    try {
      const status = await bridge.playDeviceTrack(trackId);
      logger.ipc("playDeviceTrack response", status);
      setPlaybackStatus(status);
      setErrorMessage(null);
      if (audioRef.current) {
        const srcUrl = `/api/audio_stream?trackId=${trackId}&t=${Date.now()}`;
        logger.audio(`Set audio.src to ${srcUrl}`);
        audioRef.current.src = srcUrl;
        audioRef.current.volume = status.volume;
        safePlayAudio(audioRef.current);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      logger.ipcError(`playDeviceTrack error: ${msg}`);
      setErrorMessage(msg);
      setLoadingTrackId(null);
      setIsAudioBuffering(false);
    }
  };

  const handleTogglePlayPause = async () => {
    logger.ui(`Toggle play/pause, current state: ${playbackStatus.state}`);
    if (playbackStatus.state === "playing" && bridge.pausePlayback) {
      safePauseAudio(audioRef.current);
      const status = await bridge.pausePlayback();
      setPlaybackStatus(status);
      setIsAudioBuffering(false);
    } else if (playbackStatus.state === "paused" && bridge.resumePlayback) {
      setIsAudioBuffering(true);
      safePlayAudio(audioRef.current);
      const status = await bridge.resumePlayback();
      setPlaybackStatus(status);
    }
  };

  const handleStopPlayback = async () => {
    logger.ui("Stop playback");
    setIsAudioBuffering(false);
    setLoadingTrackId(null);
    setLoadingHostPath(null);
    if (audioRef.current) {
      safePauseAudio(audioRef.current);
      audioRef.current.currentTime = 0;
      audioRef.current.removeAttribute("src");
      audioRef.current.load();
    }
    if (!bridge.stopPlayback) return;
    const status = await bridge.stopPlayback();
    setPlaybackStatus(status);
  };

  const handleSeekPlayback = async (positionMs: number) => {
    logger.ui(`Seek playback to ${positionMs}ms`);
    if (audioRef.current && isFinite(positionMs)) {
      audioRef.current.currentTime = positionMs / 1000;
    }
    if (!bridge.seekPlayback) return;
    const status = await bridge.seekPlayback(positionMs);
    setPlaybackStatus(status);
  };

  const handleVolumeChange = async (volume: number) => {
    logger.ui(`Change volume to ${volume}`);
    if (audioRef.current) {
      audioRef.current.volume = volume;
    }
    if (!bridge.setPlaybackVolume) return;
    const status = await bridge.setPlaybackVolume(volume);
    setPlaybackStatus(status);
  };

  const hasUncommittedChanges = useMemo(() => {
    return (
      (syncBatch.additions?.length ?? 0) > 0 ||
      (syncBatch.edits?.length ?? 0) > 0 ||
      (syncBatch.removals?.length ?? 0) > 0 ||
      (syncBatch.orphanRecoveries?.length ?? 0) > 0 ||
      (syncBatch.orphanDeletions?.length ?? 0) > 0
    );
  }, [syncBatch]);

  const handleEjectClick = () => {
    if (hasUncommittedChanges) {
      setShowEjectWarning(true);
    } else {
      executeEject(false);
    }
  };

  const executeEject = async (force: boolean) => {
    if (!bridge.ejectDevice || isEjecting) return;
    setIsEjecting(true);
    logger.ui(`Eject device: force=${force}`);
    try {
      if (audioRef.current) {
        safePauseAudio(audioRef.current);
        audioRef.current.currentTime = 0;
        audioRef.current.removeAttribute("src");
        audioRef.current.load();
      }
      setIsAudioBuffering(false);
      setLoadingTrackId(null);
      setLoadingHostPath(null);
      await bridge.ejectDevice(force);
      if (bridge.getPlaybackStatus) {
        const status = await bridge.getPlaybackStatus();
        setPlaybackStatus(status);
      } else {
        setPlaybackStatus((prev) => {
          if (prev.track?.sourceKind === "device") {
            return {
              state: "stopped",
              track: null,
              elapsedMs: 0,
              durationMs: 0,
              volume: prev.volume,
            };
          }
          return prev;
        });
      }
      setShowEjectWarning(false);
      applyConnectionState({ status: "disconnected" });
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    } finally {
      setIsEjecting(false);
    }
  };

  const handleStageOrphanRecovery = async (obfuscatedPath: string) => {
    if (!bridge.stageOrphanRecovery) return;
    try {
      const updated = await bridge.stageOrphanRecovery(obfuscatedPath);
      setSyncBatch(updated);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleStageOrphanDeletion = async (obfuscatedPath: string) => {
    if (!bridge.stageOrphanDeletion) return;
    try {
      const updated = await bridge.stageOrphanDeletion(obfuscatedPath);
      setSyncBatch(updated);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const handleExportSelectedTracks = async () => {
    if (!bridge.exportDeviceTracks || selectedTrackIds.length === 0 || isExporting) return;
    setIsExporting(true);
    logger.ui(`Export selected tracks count=${selectedTrackIds.length}`);
    try {
      const dest = exportDestination.trim() || "/music/PodLoader_Export";
      const res = await bridge.exportDeviceTracks(selectedTrackIds, dest);
      setExportSuccessMessage(`Exported ${res.exportedCount} track(s)`);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    } finally {
      setIsExporting(false);
    }
  };

  const handleExportPlaylist = async () => {
    if (!bridge.exportDevicePlaylist || !selectedPlaylist || isExporting) return;
    setIsExporting(true);
    logger.ui(`Export playlist: ${selectedPlaylist.name}`);
    try {
      const dest = exportDestination.trim() || "/music/PodLoader_Export";
      const res = await bridge.exportDevicePlaylist(selectedPlaylist.id, dest);
      setExportSuccessMessage(
        `Exported playlist: ${selectedPlaylist.name} (${res.exportedCount} tracks)`,
      );
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    }
  };

  const renderPlayerBar = () => (
    <>
      <footer
        role="region"
        aria-label="Audio Player"
        className="mini-audio-player-bar"
      >
      <div className="player-track-info">
        {playbackStatus.track ? (
          <>
            <span className="player-track-title">
              {playbackStatus.track.title}
            </span>
            <span className="player-track-artist-album">
              {playbackStatus.track.artist} — {playbackStatus.track.album}
            </span>
            <span
              className={`player-source-badge ${playbackStatus.track.sourceKind}`}
            >
              {playbackStatus.track.sourceKind === "host" ? "Host" : "Device"}
            </span>
            {(isAudioBuffering || loadingTrackId !== null || loadingHostPath !== null) && (
              <span className="player-buffering-badge">
                <span className="spinner-dot" /> Buffering...
              </span>
            )}
          </>
        ) : (
          <span className="player-idle">No track playing</span>
        )}
      </div>
      <div className={`player-controls ${isAudioBuffering || loadingTrackId !== null || loadingHostPath !== null ? "is-buffering" : ""}`}>
        <button
          type="button"
          aria-label={playbackStatus.state === "playing" ? "Pause" : "Play"}
          disabled={!playbackStatus.track}
          onClick={handleTogglePlayPause}
        >
          {(isAudioBuffering || loadingTrackId !== null || loadingHostPath !== null) && (
            <span className="spinner-icon" aria-hidden="true" />
          )}
          {playbackStatus.state === "playing" ? "Pause" : "Play"}
        </button>
        <button
          type="button"
          aria-label="Stop"
          disabled={playbackStatus.state === "stopped"}
          onClick={handleStopPlayback}
        >
          Stop
        </button>
        <span className="player-time">
          {formatDuration(playbackStatus.elapsedMs)}
        </span>
        <input
          type="range"
          role="slider"
          aria-label="Seek track"
          min={0}
          max={playbackStatus.durationMs || 100}
          value={playbackStatus.elapsedMs}
          disabled={!playbackStatus.track}
          onChange={(e) => handleSeekPlayback(Number(e.target.value))}
        />
        <span className="player-time">
          {formatDuration(playbackStatus.durationMs)}
        </span>
        <label htmlFor="player-volume-slider">Volume</label>
        <input
          id="player-volume-slider"
          type="range"
          role="slider"
          aria-label="Volume"
          min={0}
          max={1}
          step={0.01}
          value={playbackStatus.volume}
          onChange={(e) => handleVolumeChange(Number(e.target.value))}
        />
        <audio
          ref={audioRef}
          style={{ display: "none" }}
          preload="auto"
          onLoadStart={() => {
            logger.audio(`audio loadstart: ${audioRef.current?.src}`);
            setIsAudioBuffering(true);
          }}
          onCanPlay={() => {
            logger.audio(
              `audio canplay: readyState=${audioRef.current?.readyState}, duration=${audioRef.current?.duration}s`,
            );
          }}
          onPlaying={() => {
            logger.audio("audio playing: hardware audio stream active");
            setIsAudioBuffering(false);
            setLoadingTrackId(null);
            setLoadingHostPath(null);
          }}
          onPause={() => {
            logger.audio("audio paused");
            setIsAudioBuffering(false);
          }}
          onWaiting={() => {
            logger.audioWarn("audio waiting / buffering...");
            setIsAudioBuffering(true);
          }}
          onStalled={() => {
            logger.audioWarn("audio stalled");
            setIsAudioBuffering(true);
          }}
          onError={(e) => {
            const err = e.currentTarget.error;
            logger.audioError(
              `audio error code=${err?.code}: ${err?.message || "playback failed"}`,
            );
            setIsAudioBuffering(false);
            setLoadingTrackId(null);
            setLoadingHostPath(null);
          }}
          onTimeUpdate={(e) => {
            const audio = e.currentTarget;
            if (audio && !isNaN(audio.currentTime) && audio.currentTime > 0) {
              const curMs = Math.round(audio.currentTime * 1000);
              setPlaybackStatus((prev) =>
                prev.state === "playing" ? { ...prev, elapsedMs: curMs } : prev,
              );
            }
          }}
          onEnded={() => {
            logger.audio("audio ended naturally");
            setIsAudioBuffering(false);
            setLoadingTrackId(null);
            setLoadingHostPath(null);
            setPlaybackStatus((prev) => ({
              ...prev,
              state: "stopped",
              elapsedMs: prev.durationMs,
            }));
          }}
        />
      </div>
      <div className="player-dock-status">
        <button
          type="button"
          className={`status-pill debug-toggle-pill ${isDebugOpen ? "active" : ""}`}
          onClick={() => setIsDebugOpen((prev) => !prev)}
          title="Toggle Debug Activity Logger"
        >
          🪵 Debug Log {logs.length > 0 && `(${logs.length})`}
        </button>
        <span
          className={`status-pill ${hasUncommittedChanges ? "pending" : "synced"}`}
          title={
            hasUncommittedChanges
              ? "Sync Batch has uncommitted changes"
              : "All changes committed"
          }
        >
          {hasUncommittedChanges ? "● Staged Changes" : "● Synced"}
        </span>
        {activeSession && (
          <span className="status-pill tracks-count">
            {orderedTracks.length} tracks
          </span>
        )}
        <span
          className="status-pill ipc-status"
          title="Native Rust Backend connected on port 41731"
        >
          ● IPC Active
        </span>
      </div>
    </footer>
    {isDebugOpen && (
      <section
        aria-label="Debug Activity Logger"
        className="debug-log-drawer"
      >
        <div className="debug-log-header">
          <div className="debug-log-title-cluster">
            <span className="debug-log-badge">DEBUG ACTIVITY LOGGER</span>
            <span className="debug-log-count">{filteredLogs.length} events</span>
            <label className="debug-log-toggle-label">
              <input
                type="checkbox"
                checked={logger.enabled}
                onChange={(e) => logger.setEnabled(e.target.checked)}
              />
              Logging Active
            </label>
            <label className="debug-log-toggle-label">
              <input
                type="checkbox"
                checked={isAutoScroll}
                onChange={(e) => setIsAutoScroll(e.target.checked)}
              />
              Auto-Scroll
            </label>
          </div>
          <div className="debug-log-controls">
            <input
              type="search"
              placeholder="Filter logs by text..."
              value={logSearch}
              onChange={(e) => setLogSearch(e.target.value)}
              className="debug-log-search"
              aria-label="Filter logs"
            />
            <select
              value={logFilter}
              onChange={(e) => setLogFilter(e.target.value)}
              className="debug-log-filter"
              aria-label="Filter category"
            >
              <option value="ALL">All Categories</option>
              <option value="AUDIO">Audio Only</option>
              <option value="IPC">IPC / Bridge</option>
              <option value="UI">UI Actions</option>
              <option value="ERROR">Errors Only</option>
            </select>
            <button
              type="button"
              onClick={() => {
                navigator.clipboard.writeText(logger.exportText());
                setCopiedLogs(true);
                setTimeout(() => setCopiedLogs(false), 2000);
              }}
              className="debug-log-btn"
              title="Copy all logs to clipboard"
            >
              {copiedLogs ? "✓ Copied!" : "📋 Copy"}
            </button>
            <button
              type="button"
              onClick={() => logger.clear()}
              className="debug-log-btn"
              title="Clear log history"
            >
              🗑 Clear
            </button>
            <button
              type="button"
              onClick={() => setIsDebugOpen(false)}
              className="debug-log-close"
              aria-label="Close Debug Logger"
            >
              ✕
            </button>
          </div>
        </div>
        <div className="debug-log-body" ref={logsContainerRef}>
          {filteredLogs.length === 0 ? (
            <div className="debug-log-empty">No log events recorded.</div>
          ) : (
            filteredLogs.map((item) => (
              <div
                key={item.id}
                className={`debug-log-row level-${item.level}`}
              >
                <span className="debug-log-time">{item.timestamp}</span>
                <span
                  className={`debug-log-cat cat-${item.category.toLowerCase()}`}
                >
                  {item.category}
                </span>
                <span className="debug-log-msg">{item.message}</span>
                {item.details !== undefined && (
                  <span className="debug-log-details">
                    {typeof item.details === "object"
                      ? JSON.stringify(item.details)
                      : String(item.details)}
                  </span>
                )}
              </div>
            ))
          )}
        </div>
      </section>
    )}
    </>
  );

  if (connection.status === "disconnected") {
    return (
      <main className="podloader-shell disconnected-view">
        <div className="empty-state-card">
          <div className="macos-traffic-lights" aria-hidden="true">
            <span className="traffic-dot close"></span>
            <span className="traffic-dot minimize"></span>
            <span className="traffic-dot zoom"></span>
          </div>
          <div className="ipod-illustration-badge"> iPod Device</div>
          <h1>PodLoader</h1>
          <p>
            No iPod Device Connected. Connect a USB mass-storage Device to view
            its Device Library.
          </p>
          <div className="empty-state-hint">
            <span>
              Supported: iPod Classic (1st–7th Gen), iPod Mini, iPod Nano, iPod
              Video
            </span>
          </div>
        </div>
        {renderPlayerBar()}
      </main>
    );
  }

  if (connection.status === "uninitializedVolume") {
    const vol = connection.data;
    const handleInitialize = async () => {
      if (isInitializing) return;
      setIsInitializing(true);
      try {
        const nextState = await bridge.initializeDevice(
          vol.mountPath,
          initDeviceName.trim() || vol.volumeName || "My iPod",
        );
        applyConnectionState(nextState);
      } finally {
        setIsInitializing(false);
      }
    };

    return (
      <main className="podloader-shell uninitialized-view">
        <div className="init-wizard-card">
          <div className="macos-traffic-lights" aria-hidden="true">
            <span className="traffic-dot close"></span>
            <span className="traffic-dot minimize"></span>
            <span className="traffic-dot zoom"></span>
          </div>
          <div className="wizard-badge">New Device</div>
          <h1>PodLoader</h1>
          <p>Uninitialized volume detected: {vol.volumeName}</p>
          <p>
            Mount Path: {vol.mountPath} ({formatGigabytes(vol.freeBytes)} free)
          </p>
          <div className="init-form-group">
            <label htmlFor="device-name-input">Device Name</label>
            <input
              id="device-name-input"
              type="text"
              value={initDeviceName}
              onChange={(e) => setInitDeviceName(e.target.value)}
            />
            <button
              type="button"
              disabled={isInitializing}
              className={isInitializing ? "btn-loading" : ""}
              onClick={handleInitialize}
            >
              {isInitializing ? (
                <>
                  <span className="spinner-icon" aria-hidden="true" />
                  Initializing...
                </>
              ) : (
                "Initialize Device"
              )}
            </button>
          </div>
        </div>
        {renderPlayerBar()}
      </main>
    );
  }

  const { device, library } = connection.data;
  const masterPlaylists = library.playlists.filter((p) => p.kind === "master");
  const userPlaylists = library.playlists.filter((p) => p.kind === "user");
  const smartPlaylists = library.playlists.filter((p) => p.kind === "smart");

  const handleBrowseHost = async (targetPath: string) => {
    if (!bridge.browseHostDirectory || isBrowsingHost) return;
    setIsBrowsingHost(true);
    logger.ui(`Browse host directory: ${targetPath}`);
    try {
      const listing = await bridge.browseHostDirectory(targetPath);
      setHostListing(listing);
      setHostPathInput(listing.currentPath);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    } finally {
      setIsBrowsingHost(false);
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
    if (!bridge.commitSyncBatch || isCommitting) return;
    setIsCommitting(true);
    logger.ui("Commit sync batch started");
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
    } finally {
      setIsCommitting(false);
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
        <div className="header-left-cluster">
          <div className="macos-traffic-lights" aria-hidden="true">
            <span className="traffic-dot close"></span>
            <span className="traffic-dot minimize"></span>
            <span className="traffic-dot zoom"></span>
          </div>
          <div className="device-title-row">
            <h1>{device.name}</h1>
            <button
              type="button"
              aria-label="Eject Device"
              disabled={isEjecting}
              className={isEjecting ? "btn-loading" : ""}
              onClick={handleEjectClick}
            >
              {isEjecting ? (
                <>
                  <span className="spinner-icon" aria-hidden="true" />
                  Ejecting...
                </>
              ) : (
                "Eject Device"
              )}
            </button>
            <button
              type="button"
              className={`debug-header-toggle-btn ${isDebugOpen ? "active" : ""}`}
              onClick={() => setIsDebugOpen((prev) => !prev)}
              title="Toggle Debug Activity Logger"
            >
              🪵 Debug Log {logs.length > 0 && `(${logs.length})`}
            </button>
          </div>
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

        <div className="itunes-lcd-screen">
          <div className="lcd-apple-logo"></div>
          <div className="lcd-display-content">
            {isAudioBuffering || loadingTrackId !== null || loadingHostPath !== null ? (
              <>
                <div className="lcd-track-title">
                  {playbackStatus.track ? playbackStatus.track.title : "Loading Audio..."}
                </div>
                <div className="lcd-track-artist lcd-buffering-pulse">
                  <span className="lcd-spinner-dot" /> Buffering stream from {playbackStatus.track?.sourceKind === "device" || loadingTrackId !== null ? "iPod" : "Host"}...
                </div>
                <div className="lcd-progress-track lcd-indeterminate">
                  <div className="lcd-progress-shimmer" />
                </div>
              </>
            ) : playbackStatus.track ? (
              <>
                <div className="lcd-track-title">
                  {playbackStatus.track.title}
                </div>
                <div className="lcd-track-artist">
                  {playbackStatus.track.artist} — {playbackStatus.track.album}
                </div>
                <div className="lcd-progress-track">
                  <div
                    className="lcd-progress-fill"
                    style={{
                      width: `${
                        playbackStatus.durationMs > 0
                          ? Math.min(
                              100,
                              Math.round(
                                (playbackStatus.elapsedMs /
                                  playbackStatus.durationMs) *
                                  100,
                              ),
                            )
                          : 0
                      }%`,
                    }}
                  />
                </div>
              </>
            ) : (
              <>
                <div className="lcd-idle-title">iTunes iPod Manager</div>
                <div className="lcd-idle-subtitle">
                  {device.name} • {orderedTracks.length} Songs
                </div>
              </>
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

      {exportSuccessMessage && (
        <div role="status" className="success-banner">
          {exportSuccessMessage}
        </div>
      )}

      <div className="workspace">
        <aside
          aria-label="Host File Browser"
          className={`host-browser-pane ${isHostBrowserCollapsed ? "collapsed" : ""}`}
        >
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
              disabled={isBrowsingHost}
              className={isBrowsingHost ? "btn-loading" : ""}
              onClick={() => handleBrowseHost(hostPathInput)}
            >
              {isBrowsingHost ? (
                <>
                  <span className="spinner-icon" aria-hidden="true" />
                  Loading...
                </>
              ) : (
                "Browse"
              )}
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
                  onDoubleClick={() => handlePlayHostFile(hf)}
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
                    {(() => {
                      const isThisLoading =
                        loadingHostPath === hf.path ||
                        (playbackStatus.track?.sourcePath === hf.path &&
                          isAudioBuffering);
                      const isThisPlaying =
                        playbackStatus.track?.sourcePath === hf.path &&
                        playbackStatus.state === "playing" &&
                        !isAudioBuffering;
                      return (
                        <button
                          type="button"
                          className={`track-play-btn ${isThisLoading ? "is-loading" : ""} ${isThisPlaying ? "is-playing" : ""}`}
                          aria-label={`Play ${hf.title}`}
                          onClick={(e) => {
                            e.stopPropagation();
                            handlePlayHostFile(hf);
                          }}
                        >
                          {isThisLoading ? (
                            <span className="track-spinner" aria-hidden="true" />
                          ) : isThisPlaying ? (
                            "⏸"
                          ) : (
                            "▶"
                          )}
                        </button>
                      );
                    })()}
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
          {device && (
            <div className="sidebar-capacity-footer">
              <div className="capacity-label-row">
                <span className="capacity-title">Device Storage</span>
                <span className="capacity-pct">
                  {device.totalBytes > 0
                    ? Math.min(
                        100,
                        Math.round(
                          (device.usedBytes / device.totalBytes) * 100,
                        ),
                      )
                    : 0}
                  % capacity
                </span>
              </div>
              <div
                className="capacity-gauge-track"
                role="progressbar"
                aria-valuenow={
                  device.totalBytes > 0
                    ? Math.min(
                        100,
                        Math.round(
                          (device.usedBytes / device.totalBytes) * 100,
                        ),
                      )
                    : 0
                }
                aria-valuemin={0}
                aria-valuemax={100}
              >
                <div
                  className="capacity-gauge-fill"
                  style={{
                    width: `${
                      device.totalBytes > 0
                        ? Math.min(
                            100,
                            Math.round(
                              (device.usedBytes / device.totalBytes) * 100,
                            ),
                          )
                        : 0
                    }%`,
                  }}
                />
              </div>
              <div className="storage-summary-mini">
                <span>
                  {formatGigabytes(device.usedBytes)} /{" "}
                  {formatGigabytes(device.totalBytes)}
                </span>
                <span>{formatGigabytes(device.freeBytes)} avail</span>
              </div>
            </div>
          )}
        </nav>

        {(() => {
          const hasStagedItems =
            syncBatch.additions.length > 0 ||
            (syncBatch.edits && syncBatch.edits.length > 0) ||
            (syncBatch.removals && syncBatch.removals.length > 0) ||
            (syncBatch.orphanRecoveries && syncBatch.orphanRecoveries.length > 0) ||
            (syncBatch.orphanDeletions && syncBatch.orphanDeletions.length > 0);
          const totalStagedCount =
            syncBatch.additions.length +
            (syncBatch.edits?.length ?? 0) +
            (syncBatch.removals?.length ?? 0) +
            (syncBatch.orphanRecoveries?.length ?? 0) +
            (syncBatch.orphanDeletions?.length ?? 0);

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
              <div className="track-pane-search-bar">
                <div className="search-input-wrapper">
                  <span className="search-icon">🔍</span>
                  <input
                    type="search"
                    aria-label="Search Tracks"
                    placeholder="Filter tracks by title, artist, album, genre..."
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                  />
                  {searchQuery && (
                    <button
                      type="button"
                      className="clear-search-btn"
                      onClick={() => setSearchQuery("")}
                      aria-label="Clear search"
                    >
                      ✕
                    </button>
                  )}
                </div>
                <div className="search-meta">
                  <span className="track-counter-pill">
                    {searchQuery
                      ? `${filteredTracks.length} of ${orderedTracks.length} tracks`
                      : `${orderedTracks.length} tracks`}
                  </span>
                  <button
                    type="button"
                    className="toggle-host-btn"
                    onClick={() => setIsHostBrowserCollapsed((prev) => !prev)}
                    title={
                      isHostBrowserCollapsed
                        ? "Show Host File Browser"
                        : "Hide Host File Browser"
                    }
                  >
                    {isHostBrowserCollapsed
                      ? "📁 Show Host Files"
                      : "✕ Hide Host Files"}
                  </button>
                </div>
              </div>
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
                {syncBatch.orphanRecoveries?.map((rec) => (
                  <li key={`rec-${rec.stagedId}`}>
                    <span>
                      Recover: {rec.orphanFile.title} ({rec.orphanFile.obfuscatedPath})
                    </span>
                  </li>
                ))}
                {syncBatch.orphanDeletions?.map((del) => (
                  <li key={`del-${del.stagedId}`}>
                    <span>
                      Delete Orphan: {del.orphanFile.title} ({del.orphanFile.obfuscatedPath})
                    </span>
                  </li>
                ))}
              </ul>

              <div className="sync-batch-actions">
                <button
                  type="button"
                  disabled={isCommitting}
                  className={isCommitting ? "btn-loading" : ""}
                  onClick={() => handleCommitBatch()}
                >
                  {isCommitting ? (
                    <>
                      <span className="spinner-icon" aria-hidden="true" />
                      Committing Sync Batch...
                    </>
                  ) : (
                    "Commit Sync Batch"
                  )}
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

          {activeSession?.orphanFiles && activeSession.orphanFiles.length > 0 && (
            <section aria-label="Orphan Files" className="orphan-files-section">
              <h2>Orphan Files ({activeSession.orphanFiles.length})</h2>
              <table aria-label="Orphan Files List">
                <thead>
                  <tr>
                    <th>Title</th>
                    <th>Artist</th>
                    <th>Album</th>
                    <th>Format</th>
                    <th>Size</th>
                    <th>Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {activeSession.orphanFiles.map((orphan) => (
                    <tr key={orphan.obfuscatedPath}>
                      <td>{orphan.title}</td>
                      <td>{orphan.artist}</td>
                      <td>{orphan.album}</td>
                      <td>{orphan.format}</td>
                      <td>{formatGigabytes(orphan.fileSize)}</td>
                      <td>
                        <button
                          type="button"
                          aria-label={`Recover ${orphan.title}`}
                          onClick={() =>
                            handleStageOrphanRecovery(orphan.obfuscatedPath)
                          }
                        >
                          Recover into Device Library
                        </button>
                        <button
                          type="button"
                          aria-label={`Delete ${orphan.title}`}
                          onClick={() =>
                            handleStageOrphanDeletion(orphan.obfuscatedPath)
                          }
                        >
                          Delete Orphan
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
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
            <div className="export-destination-group">
              <label htmlFor="export-destination-input">Export Folder:</label>
              <input
                id="export-destination-input"
                aria-label="Export Destination"
                type="text"
                value={exportDestination}
                onChange={(e) => setExportDestination(e.target.value)}
              />
            </div>
            <button
              type="button"
              disabled={selectedTrackIds.length === 0 || isExporting}
              className={isExporting ? "btn-loading" : ""}
              onClick={handleExportSelectedTracks}
            >
              {isExporting ? (
                <>
                  <span className="spinner-icon" aria-hidden="true" />
                  Exporting...
                </>
              ) : (
                "Export Selected Tracks"
              )}
            </button>
            <button
              type="button"
              disabled={isExporting}
              className={isExporting ? "btn-loading" : ""}
              onClick={handleExportPlaylist}
            >
              {isExporting ? (
                <>
                  <span className="spinner-icon" aria-hidden="true" />
                  Exporting...
                </>
              ) : (
                "Export Playlist"
              )}
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
                  aria-label="Destination Playlist"
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
                      filteredTracks.length > 0 &&
                      selectedTrackIds.length === filteredTracks.length
                    }
                    onChange={(e) => {
                      if (e.target.checked) {
                        setSelectedTrackIds(filteredTracks.map((t) => t.id));
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
              {filteredTracks.map((track, idx) => (
                <tr
                  key={track.id}
                  className={
                    selectedTrackIds.includes(track.id) ? "selected" : ""
                  }
                  onClick={(e) => {
                    const target = e.target as HTMLElement;
                    if (target.tagName !== "BUTTON" && target.tagName !== "INPUT") {
                      handleRowClick(track.id, e);
                    }
                  }}
                  onDoubleClick={() => handlePlayDeviceTrack(track.id)}
                >
                  <td>
                    <input
                      type="checkbox"
                      aria-label={`Select ${track.title}`}
                      checked={selectedTrackIds.includes(track.id)}
                      onChange={(e) => {
                        e.stopPropagation();
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
                  <td>
                    {(() => {
                      const isThisLoading =
                        loadingTrackId === track.id ||
                        (playbackStatus.track?.trackId === track.id &&
                          isAudioBuffering);
                      const isThisPlaying =
                        playbackStatus.track?.trackId === track.id &&
                        playbackStatus.state === "playing" &&
                        !isAudioBuffering;
                      return (
                        <button
                          type="button"
                          className={`track-play-btn ${isThisLoading ? "is-loading" : ""} ${isThisPlaying ? "is-playing" : ""}`}
                          aria-label={`Play ${track.title}`}
                          onClick={(e) => {
                            e.stopPropagation();
                            handlePlayDeviceTrack(track.id);
                          }}
                        >
                          {isThisLoading ? (
                            <span className="track-spinner" aria-hidden="true" />
                          ) : isThisPlaying ? (
                            "⏸"
                          ) : (
                            "▶"
                          )}
                        </button>
                      );
                    })()}
                    <span>{track.title}</span>
                  </td>
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

      {showEjectWarning && (
        <div
          role="dialog"
          aria-label="Uncommitted Changes"
          className="modal-overlay"
        >
          <div className="modal-content">
            <h3>Uncommitted Changes</h3>
            <p>Sync Batch has uncommitted changes. Eject anyway?</p>
            <div className="modal-actions">
              <button type="button" onClick={() => executeEject(true)}>
                Eject Anyway
              </button>
              <button
                type="button"
                onClick={() => setShowEjectWarning(false)}
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}

      {renderPlayerBar()}
    </div>
  );
}
