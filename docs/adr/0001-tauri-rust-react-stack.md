# Use Tauri v2 (Rust Core + TypeScript/React UI)

PodLoader is built on Tauri v2, pairing a Rust backend with a TypeScript/React frontend. Rust handles binary `iTunesDB` parsing/serialization, `hash58`/`hash72` device signing, USB mount detection, `iPod_Control` filesystem operations, and concurrent FFmpeg transcoding pipelines, while React renders the dual-pane virtualized tables and drag-and-drop workflow without the memory overhead of bundling Chromium via Electron.
