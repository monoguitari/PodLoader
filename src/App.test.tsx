import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";
import { App } from "./App";
import type {
  CommitOutcome,
  DeviceConnectionState,
  DeviceSession,
  DuplicateResolution,
  HostDirectoryListing,
  HostFile,
  InterruptedResolution,
  PodLoaderBridge,
  SyncBatch,
  TranscodePreset,
  TransferProgress,
} from "./bridge";

class InMemoryPodLoaderBridge implements PodLoaderBridge {
  public state: DeviceConnectionState;
  private hostListing: HostDirectoryListing;
  private nextStagedId = 1;
  public shouldInterruptNextCommit = false;

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
        obfuscatedPath: `:iPod_Control:Music:F00:T${item.stagedId}.mp3`,
      });
      for (const pl of session.library.playlists) {
        if (pl.kind === "master" || pl.id === item.targetPlaylistId) {
          if (!pl.trackIds.includes(newId)) {
            pl.trackIds.push(newId);
          }
        }
      }
    }

    session.syncBatch = { additions: [], requiredBytes: 0 };
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
  });
});
