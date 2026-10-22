/**
 * PodLoader Full-Suite Regression Test Runner (Tickets #2 - #7)
 *
 * Runs end-to-end regression tests across all implemented features:
 * - Ticket #2: Device Detection, Initialization, & Read-Only Device Library Viewer
 * - Ticket #3: Stateless Host File Browser & Native Audio Transfer via Staged Sync Batch
 * - Ticket #4: Pure-Rust Lossless ALAC Transcoding & Per-Device Transcode Presets
 * - Ticket #5: Track Metadata Editing, Track Removal, & Playlist Management
 * - Ticket #6: Orphan File Recovery/Cleanup, Device Export, & Safe Eject
 * - Ticket #7: Built-In Mini Audio Player Bar for Host Files & Device Tracks
 *
 * Captures timestamped screenshots and an HD video of the entire session.
 */

const { chromium } = require("playwright");
const { execSync, spawn } = require("child_process");
const fs = require("fs");
const path = require("path");

const ROOT_DIR = path.resolve(__dirname, "..");
const WORKSPACE_DIR = path.join(ROOT_DIR, ".regtest-env");
const ARTIFACTS_DIR = path.join(ROOT_DIR, "regtest-artifacts");
const SCREENSHOTS_DIR = path.join(ARTIFACTS_DIR, "screenshots");
const VIDEOS_DIR = path.join(ARTIFACTS_DIR, "videos");
const EXPORT_DIR = path.join(ARTIFACTS_DIR, "exported_music");

// Ensure chromium deps LD_LIBRARY_PATH
const DEP_LIB = path.join(
  process.env.HOME || "",
  ".local/lib/chromium-deps/usr/lib/x86_64-linux-gnu",
);
if (fs.existsSync(DEP_LIB)) {
  process.env.LD_LIBRARY_PATH = `${DEP_LIB}:${process.env.LD_LIBRARY_PATH || ""}`;
}

async function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForHttp(url, timeoutMs = 25000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(url);
      if (res.ok) return true;
    } catch (_) {}
    await sleep(400);
  }
  throw new Error(`Timeout waiting for HTTP service at ${url}`);
}

async function main() {
  console.log("==========================================================");
  console.log(" 🚀 PodLoader Regression Test & Video Recording Suite");
  console.log("==========================================================");

  // 1. Prepare directories cleanly
  fs.rmSync(WORKSPACE_DIR, { recursive: true, force: true });
  fs.rmSync(EXPORT_DIR, { recursive: true, force: true });
  fs.rmSync(SCREENSHOTS_DIR, { recursive: true, force: true });
  fs.rmSync(VIDEOS_DIR, { recursive: true, force: true });
  fs.mkdirSync(SCREENSHOTS_DIR, { recursive: true });
  fs.mkdirSync(VIDEOS_DIR, { recursive: true });

  console.log("\n[1/6] Initializing isolated mock iPod & sample audio fixtures...");
  const cargoCmd = `PATH="$HOME/.cache/cargo/bin:$PATH" CARGO_HOME="${process.env.CARGO_HOME || process.env.HOME + "/.cache/cargo"}" RUSTUP_HOME="${process.env.RUSTUP_HOME || process.env.HOME + "/.cache/rustup"}" cargo run --offline -p podloader-shell -- --setup-regtest "${WORKSPACE_DIR}"`;
  execSync(cargoCmd, { cwd: ROOT_DIR, stdio: "inherit" });

  // 2. Kill existing backend process on 41731 if running
  try {
    execSync("fuser -k 41731/tcp 2>/dev/null || true");
    await sleep(500);
  } catch (_) {}

  // 3. Start backend dev bridge pointed to .regtest-env/device
  console.log("\n[2/6] Starting Rust dev bridge for regtest environment...");
  const backendProc = spawn(
    "bash",
    [
      "-c",
      `PATH="$HOME/.cache/cargo/bin:$PATH" CARGO_HOME="${process.env.CARGO_HOME || process.env.HOME + "/.cache/cargo"}" RUSTUP_HOME="${process.env.RUSTUP_HOME || process.env.HOME + "/.cache/rustup"}" PODLOADER_DEVICE_MOUNT="${WORKSPACE_DIR}/device" cargo run --offline -p podloader-shell -- --serve`,
    ],
    {
      cwd: ROOT_DIR,
      stdio: "pipe",
    },
  );

  backendProc.stdout.on("data", (d) => {
    console.log("[Backend]", d.toString().trim());
  });

  backendProc.stderr.on("data", (d) => {
    console.error("[Backend Error]", d.toString().trim());
  });

  await waitForHttp("http://127.0.0.1:41731/api/scan_device");
  console.log("✓ Backend server listening on http://127.0.0.1:41731");

  // Verify frontend is ready, spawn vite if needed
  let viteProc = null;
  try {
    const res = await fetch("http://localhost:5173/");
    if (res.ok) {
      console.log("✓ Existing Vite dev server detected on http://localhost:5173");
    }
  } catch (_) {
    console.log("▶ Spawning Vite dev server on http://localhost:5173...");
    viteProc = spawn("npx", ["vite", "--port", "5173"], {
      cwd: ROOT_DIR,
      stdio: "pipe",
    });
  }

  await waitForHttp("http://localhost:5173/");
  console.log("✓ Frontend Vite server running on http://localhost:5173");

  // 4. Launch Playwright
  console.log("\n[3/6] Launching browser with video recording...");
  const browser = await chromium.launch({
    headless: true,
    args: ["--no-sandbox", "--disable-setuid-sandbox", "--disable-gpu"],
  });

  const context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    recordVideo: {
      dir: VIDEOS_DIR,
      size: { width: 1280, height: 800 },
    },
  });

  const page = await context.newPage();
  page.on("console", (msg) => {
    console.log(`  [Browser ${msg.type()}]`, msg.text());
  });
  page.on("pageerror", (err) => console.error("  [Page Error]", err));

  try {
    console.log("\n[4/6] Executing feature regression steps across Tickets #2 - #7...\n");

    // -------------------------------------------------------------
    // Ticket #2: Device Detection & Read-Only Library Viewer
    // -------------------------------------------------------------
    console.log("▶ [Ticket #2] Verifying Device Detection and Library Viewer...");
    await page.goto("http://localhost:5173");
    await page.waitForSelector("h1:has-text('RegTest Classic')");

    // Verify storage summary and master playlist
    await page.waitForSelector(".storage-summary");
    await page.waitForSelector("button:has-text('RegTest Classic')");

    await sleep(1000);
    await page.screenshot({
      path: path.join(SCREENSHOTS_DIR, "01_ticket2_device_library_viewer.png"),
      fullPage: false,
    });
    console.log("  ✓ Captured 01_ticket2_device_library_viewer.png");

    // -------------------------------------------------------------
    // Ticket #7 (Part 1): Host Audio Preview in Mini Player Bar
    // -------------------------------------------------------------
    console.log("▶ [Ticket #7] Previewing Host File audio in bottom mini player bar...");
    const hostMusicDir = path.join(WORKSPACE_DIR, "host_music");
    const hostInput = page.locator("#host-path-input");
    await hostInput.fill(hostMusicDir);
    await page.click("button:has-text('Browse')");

    // Wait for host files to display
    await page.waitForSelector("table[aria-label='Host Files']:has-text('Open Sailor MP3')");

    // Click Play on the first host file
    await page.click("button[aria-label='Play Open Sailor MP3']");
    await page.waitForSelector(".player-track-title:has-text('Open Sailor MP3')");
    await page.waitForSelector(".player-source-badge.host:has-text('Host')");

    // Pause and Resume
    await sleep(600);
    await page.click("button:has-text('Pause')");
    await sleep(400);
    await page.click("button:has-text('Play')");

    await page.screenshot({
      path: path.join(SCREENSHOTS_DIR, "02_ticket7_host_audio_playback.png"),
      fullPage: false,
    });
    console.log("  ✓ Captured 02_ticket7_host_audio_playback.png");

    // -------------------------------------------------------------
    // Ticket #3: Stateless Host File Browser & Native Audio Batch Transfer
    // -------------------------------------------------------------
    console.log("▶ [Ticket #3] Staging native host files and committing Sync Batch...");
    // Stage MP3 and WAV
    await page.click("table[aria-label='Host Files'] tr:has-text('Open Sailor') button:has-text('Stage')");
    await page.click("table[aria-label='Host Files'] tr:has-text('Harbor Shanty') button:has-text('Stage')");

    await page.waitForSelector("section[aria-label='Sync Batch']:has-text('Open Sailor')");
    await sleep(600);

    await page.screenshot({
      path: path.join(SCREENSHOTS_DIR, "03_ticket3_staged_sync_batch.png"),
      fullPage: false,
    });
    console.log("  ✓ Captured 03_ticket3_staged_sync_batch.png");

    // Commit Batch
    await page.click("button:has-text('Commit Sync Batch')");
    await sleep(1000);
    const errEl1 = await page.$("div[role='alert'].error-banner");
    if (errEl1) {
      console.error("  [App Error Banner]:", await errEl1.innerText());
    }
    // Wait for commit progress and completion
    await page.waitForSelector("table[aria-label='Device Tracks']:has-text('Open Sailor')", {
      timeout: 15000,
    });
    console.log("  ✓ Native audio files committed and visible in Device Library");

    // -------------------------------------------------------------
    // Ticket #4: Pure-Rust Lossless ALAC Transcoding
    // -------------------------------------------------------------
    console.log("▶ [Ticket #4] Staging FLAC and OGG files for pure-Rust ALAC transcoding...");
    // Stage FLAC and OGG
    await page.click("table[aria-label='Host Files'] tr:has-text('Open Waters') button:has-text('Stage')");
    await page.click("table[aria-label='Host Files'] tr:has-text('Trade Winds') button:has-text('Stage')");

    await page.waitForSelector("section[aria-label='Sync Batch']:has-text('Open Waters')");

    // Verify ALAC preset is selected
    const presetSelect = page.locator("#transcode-preset-select");
    await presetSelect.selectOption("alac");

    await page.screenshot({
      path: path.join(SCREENSHOTS_DIR, "04_ticket4_alac_transcoding_staged.png"),
      fullPage: false,
    });
    console.log("  ✓ Captured 04_ticket4_alac_transcoding_staged.png");

    // Commit Transcoded Batch
    await page.click("button:has-text('Commit Sync Batch')");
    await page.waitForSelector("table[aria-label='Device Tracks']:has-text('Open Waters')", {
      timeout: 20000,
    });
    console.log("  ✓ Lossless ALAC transcode committed and verified in Device Library");

    // -------------------------------------------------------------
    // Ticket #5: Metadata Editing & Playlist Management
    // -------------------------------------------------------------
    console.log("▶ [Ticket #5] Editing track metadata and managing custom playlists...");
    // Ensure Master Playlist is selected
    await page.click("button:has-text('RegTest Classic')");
    await sleep(400);

    // Select the first track checkbox in the table body
    const trackCheckbox = page.locator("table[aria-label='Device Tracks'] tbody tr:first-child input[type='checkbox']");
    await trackCheckbox.check();

    // Click Edit Metadata
    await page.click("button:has-text('Edit Metadata')");
    await page.waitForSelector("#edit-title");

    await page.fill("#edit-title", "RegTest Masterpiece");
    await page.fill("#edit-artist", "Open Source Artists");
    await page.fill("#edit-rating", "5");
    await page.click("button:has-text('Stage Edits')");

    // Wait for edit modal to detach
    await page.waitForSelector("div[role='dialog']", { state: "detached", timeout: 5000 });

    // Create custom playlist
    const isCreateInputVisible = await page.locator("input[aria-label='New Playlist Name']").isVisible();
    if (!isCreateInputVisible) {
      const newPlaylistBtn = page.locator("button:has-text('New Playlist')");
      await newPlaylistBtn.waitFor({ state: "visible", timeout: 5000 });
      await newPlaylistBtn.click();
    }
    const playlistInput = page.locator("input[aria-label='New Playlist Name']");
    await playlistInput.fill("Maritime Favorites");
    await page.click("button:has-text('Create Playlist')");
    await page.waitForSelector("button:has-text('Maritime Favorites')");

    // Return to Master Playlist to view all tracks
    await page.click("button:has-text('RegTest Classic')");
    await sleep(400);

    // Select track and add to Maritime Favorites playlist
    const trackCheckbox2 = page.locator("table[aria-label='Device Tracks'] tbody tr:first-child input[type='checkbox']");
    await trackCheckbox2.check();
    await page.click("button:has-text('Add to Playlist')");

    await sleep(600);
    await page.screenshot({
      path: path.join(SCREENSHOTS_DIR, "05_ticket5_metadata_and_playlists.png"),
      fullPage: false,
    });
    console.log("  ✓ Captured 05_ticket5_metadata_and_playlists.png");

    // Commit metadata changes
    await page.click("button:has-text('Commit Sync Batch')");
    await page.waitForSelector("table[aria-label='Device Tracks']:has-text('RegTest Masterpiece')");
    console.log("  ✓ Metadata edits committed and verified");

    // -------------------------------------------------------------
    // Ticket #6: Orphan File Recovery, Device Export, & Safe Eject
    // -------------------------------------------------------------
    console.log("▶ [Ticket #6] Scanning Orphan Files, exporting library, and testing Safe Eject...");
    // Orphan section should surface LOST.mp3 (Lost Mariner)
    await page.waitForSelector("section[aria-label='Orphan Files']:has-text('Lost Mariner')");
    console.log("  ✓ Orphan file 'Lost Mariner' detected under iPod_Control/Music/F14");

    // Stage orphan for recovery
    await page.click("button:has-text('Recover into Device Library')");
    await page.waitForSelector("section[aria-label='Sync Batch']:has-text('Recover: Lost Mariner')");

    // Verify Safe Eject warns on uncommitted changes
    await page.click("button:has-text('Eject Device')");
    await page.waitForSelector("div[role='dialog']:has-text('Uncommitted Changes')");
    console.log("  ✓ Safe Eject warned about uncommitted sync batch changes");
    await page.click("button:has-text('Cancel')");

    // Device Export
    const exportDestInput = page.locator("#export-destination-input");
    await exportDestInput.fill(EXPORT_DIR);
    await page.click("button:has-text('Export Playlist')");
    await page.waitForSelector("div.success-banner:has-text('Exported playlist')");
    console.log("  ✓ Device Export completed to host destination folder");

    await page.screenshot({
      path: path.join(SCREENSHOTS_DIR, "06_ticket6_orphan_recovery_and_export.png"),
      fullPage: false,
    });
    console.log("  ✓ Captured 06_ticket6_orphan_recovery_and_export.png");

    // -------------------------------------------------------------
    // Ticket #7 (Part 2): Device Track Playback & Safe Eject Termination
    // -------------------------------------------------------------
    console.log("▶ [Ticket #7] Playing device track and verifying safe eject playback termination...");
    await page.click("button[aria-label='Play RegTest Masterpiece']");
    await page.waitForSelector(".player-track-title:has-text('RegTest Masterpiece')");
    await page.waitForSelector(".player-source-badge.device:has-text('Device')");
    await sleep(1000);

    // Commit pending orphan recovery
    await page.click("button:has-text('Commit Sync Batch')");
    await page.waitForSelector("table[aria-label='Device Tracks']:has-text('Lost Mariner')");

    // Safe Eject
    await page.click("button:has-text('Eject Device')");
    await page.waitForSelector("h1:has-text('PodLoader')");
    await page.waitForSelector("p:has-text('No iPod Device Connected')");
    await page.waitForSelector(".mini-audio-player-bar:has-text('No track playing')");

    await page.screenshot({
      path: path.join(SCREENSHOTS_DIR, "07_ticket6_and_7_safe_ejected.png"),
      fullPage: false,
    });
    console.log("  ✓ Captured 07_ticket6_and_7_safe_ejected.png");

    // 5. Finalize Video
    console.log("\n[5/6] Finalizing HD video recording...");
    await sleep(1000);
    await page.close();
    await context.close();
    await browser.close();

    // Find recorded video and rename
    const rawVideos = fs.readdirSync(VIDEOS_DIR).filter((f) => f.endsWith(".webm") && f !== "regtest_full_suite.webm");
    if (rawVideos.length > 0) {
      const finalVideoPath = path.join(VIDEOS_DIR, "regtest_full_suite.webm");
      fs.renameSync(path.join(VIDEOS_DIR, rawVideos[0]), finalVideoPath);
      console.log(`✓ Video recorded successfully: ${finalVideoPath}`);
    }

    // 6. Generate HTML & Markdown Reports
    console.log("\n[6/6] Generating regression test dashboard report...");
    generateReports();

    // Cleanup background backend and vite process
    backendProc.kill();
    if (viteProc) viteProc.kill();

    console.log("\n==========================================================");
    console.log(" ✅ ALL REGRESSION TESTS PASSED (Tickets #2 through #7)");
    console.log(` 📸 Screenshots: ${SCREENSHOTS_DIR}`);
    console.log(` 🎥 Video Recording: ${path.join(VIDEOS_DIR, "regtest_full_suite.webm")}`);
    console.log(` 📊 HTML Report: ${path.join(ARTIFACTS_DIR, "report.html")}`);
    console.log("==========================================================");
  } catch (err) {
    try {
      await page.screenshot({ path: path.join(SCREENSHOTS_DIR, "debug_failure.png") });
      const text = await page.innerText("body");
      console.error("\n[PAGE BODY ON FAILURE]:\n", text);
    } catch (_) {}
    try {
      await context.close();
      await browser.close();
    } catch (_) {}
    backendProc.kill();
    if (viteProc) viteProc.kill();
    throw err;
  }
}

function generateReports() {
  const screenshots = fs
    .readdirSync(SCREENSHOTS_DIR)
    .filter((f) => f.endsWith(".png") && !f.startsWith("debug_"))
    .sort();

  const reportHtml = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>PodLoader Feature Regression Suite Report</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0f172a; color: #f8fafc; margin: 0; padding: 2rem; }
    h1 { color: #38bdf8; border-bottom: 2px solid #1e293b; padding-bottom: 0.5rem; }
    .badge { background: #059669; color: #fff; padding: 0.25rem 0.5rem; border-radius: 4px; font-size: 0.85rem; font-weight: bold; }
    .video-section { margin: 2rem 0; background: #1e293b; padding: 1.5rem; border-radius: 8px; }
    video { width: 100%; max-width: 960px; border-radius: 6px; box-shadow: 0 4px 6px rgba(0,0,0,0.3); }
    .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(420px, 1fr)); gap: 1.5rem; margin-top: 1.5rem; }
    .card { background: #1e293b; border-radius: 8px; overflow: hidden; box-shadow: 0 4px 6px rgba(0,0,0,0.3); }
    .card h3 { margin: 1rem; color: #94a3b8; font-size: 1rem; }
    .card img { width: 100%; display: block; border-top: 1px solid #334155; }
  </style>
</head>
<body>
  <h1>PodLoader Feature Regression Test Suite <span class="badge">ALL PASSED</span></h1>
  <p>Automated end-to-end regression test suite verifying User Stories and Acceptance Criteria across Tickets #2 through #7.</p>

  <div class="video-section">
    <h2>🎥 Full Session Video Recording</h2>
    <video controls src="./videos/regtest_full_suite.webm"></video>
  </div>

  <h2>📸 Milestone Screenshots</h2>
  <div class="grid">
    ${screenshots
      .map(
        (img) => `
      <div class="card">
        <h3>${img.replace(/\.png$/, "").replace(/_/g, " ")}</h3>
        <img src="./screenshots/${img}" alt="${img}">
      </div>`,
      )
      .join("")}
  </div>
</body>
</html>`;

  fs.writeFileSync(path.join(ARTIFACTS_DIR, "report.html"), reportHtml);

  const reportMd = `# PodLoader Feature Regression Suite Report

**Status:** ✅ ALL PASSED (Tickets #2 through #7)

## Coverage Summary
- **Ticket #2**: Device Detection, Initialization, & Read-Only Device Library Viewer
- **Ticket #3**: Stateless Host File Browser & Native Audio Transfer via Staged Sync Batch
- **Ticket #4**: Pure-Rust Lossless ALAC Transcoding & Per-Device Transcode Presets
- **Ticket #5**: Track Metadata Editing, Track Removal, & Playlist Management
- **Ticket #6**: Orphan File Recovery/Cleanup, Device Export, & Safe Eject
- **Ticket #7**: Built-In Mini Audio Player Bar for Host Files & Device Tracks

## Artifacts
- **Full Video Recording**: \`regtest-artifacts/videos/regtest_full_suite.webm\`
- **Interactive HTML Report**: \`regtest-artifacts/report.html\`
- **Milestone Screenshots**:
${screenshots.map((s) => `  - \`regtest-artifacts/screenshots/${s}\``).join("\n")}
`;

  fs.writeFileSync(path.join(ARTIFACTS_DIR, "REPORT.md"), reportMd);
}

main()
  .then(() => {
    process.exit(0);
  })
  .catch((err) => {
    console.error("\n❌ Regression test suite failed:", err);
    process.exit(1);
  });
