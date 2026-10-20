import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";
import { App } from "./App";
import type { DeviceConnectionState, PodLoaderBridge } from "./bridge";

class InMemoryPodLoaderBridge implements PodLoaderBridge {
  private state: DeviceConnectionState;

  constructor(initialState: DeviceConnectionState) {
    this.state = initialState;
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
      },
    };
    return this.state;
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
});
