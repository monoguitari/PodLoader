#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::TcpListener;
use podloader_core::{
    DeviceConnectionState, DuplicateResolution, InterruptedResolution, TranscodePreset,
};
use podloader_shell::AppState;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Optional positional path override: `cargo run -p podloader-shell -- /mnt/d`
    for arg in args.iter().skip(1) {
        if !arg.starts_with("--") {
            std::env::set_var("PODLOADER_DEVICE_MOUNT", arg);
        }
    }

    let state = std::sync::Arc::new(AppState::default());

    if let Some(pos) = args.iter().position(|a| a == "--setup-regtest") {
        if let Some(base_dir_str) = args.get(pos + 1) {
            let base_dir = std::path::PathBuf::from(base_dir_str);
            let device_dir = base_dir.join("device");
            let host_dir = base_dir.join("host_music");
            std::fs::create_dir_all(&device_dir).expect("create device dir");
            std::fs::create_dir_all(&host_dir).expect("create host dir");

            std::env::set_var("PODLOADER_DEVICE_MOUNT", &device_dir);

            state
                .initialize_device(&device_dir.to_string_lossy(), "RegTest Classic")
                .expect("init regtest device");

            podloader_core::fixtures::generate_regtest_sample_audio(&host_dir)
                .expect("generate regtest sample audio");

            let f14_dir = device_dir.join("iPod_Control").join("Music").join("F14");
            std::fs::create_dir_all(&f14_dir).expect("create F14 dir");
            let orphan_bytes = podloader_core::fixtures::build_id3v23_mp3_fixture(
                "Lost Mariner",
                "Ghost Crew",
                "Sunken Echoes",
                9,
                "Sea Folk",
                2021,
                3,
            );
            std::fs::write(f14_dir.join("LOST.mp3"), orphan_bytes).expect("write orphan file");

            println!("Regtest environment ready at {:?}", base_dir);
            return;
        }
    }

    if args.iter().any(|a| a == "--serve") {
        run_dev_bridge_server(std::sync::Arc::clone(&state));
        return;
    }

    match state.scan_device() {
        Ok(DeviceConnectionState::Disconnected) => {
            println!("No connected iPod Device detected.");
            println!(
                "Tip: Pass your iPod mount path directly or set PODLOADER_DEVICE_MOUNT, e.g.:"
            );
            println!("  cargo run -p podloader-shell -- /mnt/d");
        }
        Ok(DeviceConnectionState::UninitializedVolume(vol)) => {
            println!(
                "Uninitialized volume detected at {:?} (name: {}, {:.1} GB free)",
                vol.mount_path,
                vol.volume_name,
                vol.free_bytes as f64 / 1_000_000_000.0
            );
        }
        Ok(DeviceConnectionState::Connected(session)) => {
            let d = &session.device;
            println!("=== Connected Device: {} ===", d.name);
            println!("Mount Path:    {}", d.mount_path.display());
            println!(
                "Model Number:  {}",
                d.model_number.as_deref().unwrap_or("Unknown")
            );
            println!(
                "FireWire GUID: {}",
                d.firewire_guid.as_deref().unwrap_or("Not found")
            );
            println!(
                "Storage:       {:.1} GB used / {:.1} GB free ({:.1} GB total)",
                d.used_bytes as f64 / 1_000_000_000.0,
                d.free_bytes as f64 / 1_000_000_000.0,
                d.total_bytes as f64 / 1_000_000_000.0
            );
            println!(
                "Play Counts:   {}",
                if session.has_pending_play_counts {
                    "Merged pending Play Counts file"
                } else {
                    "None pending"
                }
            );
            println!("\nPlaylists ({}):", session.library.playlists.len());
            for pl in &session.library.playlists {
                println!(
                    "  - [{:?}] {} ({} tracks)",
                    pl.kind,
                    pl.name,
                    pl.track_ids.len()
                );
            }
            println!("\nDevice Tracks ({} total):", session.library.tracks.len());
            for track in session.library.tracks.iter().take(25) {
                let mins = (track.duration_ms / 1000) / 60;
                let secs = (track.duration_ms / 1000) % 60;
                println!(
                    "  [{:>4}] {} - {} ({}) [{}:{:02}, {} kbps, plays: {}, ★{}]",
                    track.id,
                    track.artist,
                    track.title,
                    track.album,
                    mins,
                    secs,
                    track.bitrate_kbps,
                    track.play_count,
                    track.rating
                );
            }
            if session.library.tracks.len() > 25 {
                println!(
                    "  ... and {} more tracks.",
                    session.library.tracks.len() - 25
                );
            }
        }
        Err(err) => {
            eprintln!("Error reading Device: {}", err);
            std::process::exit(1);
        }
    }
}

fn run_dev_bridge_server(state: std::sync::Arc<AppState>) {
    let addr = "127.0.0.1:41731";
    let listener = TcpListener::bind(addr).expect("bind dev bridge server on 127.0.0.1:41731");
    println!("PodLoader Rust backend listening on http://{}", addr);

    for stream in listener.incoming().flatten() {
        let state = std::sync::Arc::clone(&state);
        std::thread::spawn(move || {
            let _ = handle_http_client(stream, &state);
        });
    }
}

fn url_decode(s: &str) -> String {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn stream_audio_file(
    mut stream: std::net::TcpStream,
    path: &std::path::Path,
    range_header: Option<&str>,
) -> std::io::Result<()> {
    use std::io::Seek;
    if !path.exists() {
        let resp = "HTTP/1.1 404 Not Found\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes())?;
        return Ok(());
    }

    if podloader_core::transcode::is_alac_file(path) {
        if let Err(err) = podloader_core::transcode::stream_audio_pcm_wav(path, range_header, &mut stream) {
            eprintln!("Error streaming ALAC audio as WAV: {}", err);
        }
        return Ok(());
    }

    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => {
            let resp = "HTTP/1.1 404 Not Found\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(resp.as_bytes())?;
            return Ok(());
        }
    };

    let total_len = file.metadata()?.len();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let content_type = match ext.as_str() {
        "mp3" => "audio/mpeg",
        "m4a" | "mp4" | "aac" | "alac" => "audio/mp4",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" | "opus" => "audio/ogg",
        _ => "audio/mp4",
    };

    if let Some(range_val) = range_header {
        if let Some(bytes_range) = range_val.strip_prefix("bytes=") {
            let parts: Vec<&str> = bytes_range.split('-').collect();
            let start = parts.first().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            let end = parts
                .get(1)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(total_len.saturating_sub(1))
                .min(total_len.saturating_sub(1));

            if start <= end && start < total_len {
                let chunk_len = end - start + 1;
                file.seek(std::io::SeekFrom::Start(start))?;
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: {}\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                    content_type, start, end, total_len, chunk_len
                );
                stream.write_all(header.as_bytes())?;

                let mut to_read = chunk_len;
                let mut buf = [0u8; 64 * 1024];
                while to_read > 0 {
                    let max = (to_read as usize).min(buf.len());
                    let n = file.read(&mut buf[..max])?;
                    if n == 0 {
                        break;
                    }
                    stream.write_all(&buf[..n])?;
                    to_read -= n as u64;
                }
                return Ok(());
            }
        }
    }

    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        content_type, total_len
    );
    stream.write_all(header.as_bytes())?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n])?;
    }
    Ok(())
}

fn read_http_request(
    stream: &mut std::net::TcpStream,
) -> std::io::Result<(String, String, std::collections::HashMap<String, String>)> {
    let mut data = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let mut header_end: Option<usize> = None;
    let mut content_length: usize = 0;

    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);

        if header_end.is_none() {
            if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let end = pos + 4;
                header_end = Some(end);
                let headers_str = String::from_utf8_lossy(&data[..pos]);
                for line in headers_str.lines() {
                    if let Some((k, v)) = line.split_once(':') {
                        if k.trim().eq_ignore_ascii_case("content-length") {
                            content_length = v.trim().parse::<usize>().unwrap_or(0);
                        }
                    }
                }
            }
        }

        if let Some(end) = header_end {
            if data.len() >= end + content_length {
                break;
            }
        }
    }

    let Some(end) = header_end else {
        let text = String::from_utf8_lossy(&data);
        let first = text.lines().next().unwrap_or("").to_string();
        return Ok((first, "{}".to_string(), std::collections::HashMap::new()));
    };

    let headers_str = String::from_utf8_lossy(&data[..end - 4]);
    let first_line = headers_str.lines().next().unwrap_or("").to_string();
    let mut headers = std::collections::HashMap::new();
    for line in headers_str.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    let body_slice = data.get(end..end + content_length).unwrap_or(&data[end..]);
    let body = if body_slice.is_empty() {
        "{}".to_string()
    } else {
        String::from_utf8_lossy(body_slice).to_string()
    };
    Ok((first_line, body, headers))
}

fn handle_http_client(mut stream: std::net::TcpStream, state: &AppState) -> std::io::Result<()> {
    let (first_line, body, headers) = read_http_request(&mut stream)?;
    if first_line.is_empty() {
        return Ok(());
    }

    if first_line.starts_with("OPTIONS ") {
        let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization, Range\r\nAccess-Control-Expose-Headers: Content-Range, Accept-Ranges, Content-Length\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes())?;
        return Ok(());
    }

    if first_line.starts_with("GET /api/audio_stream") {
        let range = headers.get("range").map(|s| s.as_str());
        let uri = first_line.split_whitespace().nth(1).unwrap_or("");
        let query = uri.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut target_path: Option<std::path::PathBuf> = None;

        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "path" {
                    let decoded = url_decode(v);
                    target_path = Some(std::path::PathBuf::from(decoded));
                } else if k == "trackId" {
                    if let Ok(tid) = v.parse::<u32>() {
                        if let Ok(path) = state.get_device_track_path(tid) {
                            target_path = Some(path);
                        }
                    }
                }
            }
        }

        if target_path.is_none() {
            if let Ok(status) = state.get_playback_status() {
                if let Some(active) = status.track {
                    target_path = Some(active.source_path);
                }
            }
        }

        if let Some(path) = target_path {
            return stream_audio_file(stream, &path, range);
        } else {
            let resp = "HTTP/1.1 404 Not Found\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(resp.as_bytes())?;
            return Ok(());
        }
    }

    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();

    let response_json = if first_line.starts_with("GET /api/scan_device") {
        match state.scan_device() {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/initialize_device") {
        let mount_path = parsed["mountPath"].as_str().unwrap_or("");
        let device_name = parsed["deviceName"].as_str().unwrap_or("My iPod");
        match state.initialize_device(mount_path, device_name) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/browse_host_directory") {
        let dir_path = parsed["dirPath"].as_str().unwrap_or(".");
        match state.browse_host_directory(dir_path) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/stage_host_paths") {
        let paths: Vec<String> = parsed["paths"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let target_playlist_id = parsed["targetPlaylistId"].as_u64();
        match state.stage_host_paths(paths, target_playlist_id) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/set_duplicate_resolution") {
        let staged_id = parsed["stagedId"].as_u64().unwrap_or(0);
        let resolution: DuplicateResolution =
            serde_json::from_value(parsed["resolution"].clone())
                .unwrap_or(DuplicateResolution::Skip);
        match state.set_duplicate_resolution(staged_id, resolution) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/commit_sync_batch") {
        let limit = parsed["maxTracksBeforeInterrupt"]
            .as_u64()
            .map(|n| n as usize);
        let header = "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n";
        stream.write_all(header.as_bytes())?;
        let outcome = state.commit_sync_batch_with_progress(limit, |prog| {
            let line = serde_json::json!({ "type": "progress", "data": prog }).to_string() + "\n";
            let _ = stream.write_all(line.as_bytes());
            let _ = stream.flush();
        });
        let outcome_json = match outcome {
            Ok(res) => serde_json::json!({ "type": "outcome", "data": res }),
            Err(e) => serde_json::json!({ "type": "error", "error": e }),
        };
        let line = outcome_json.to_string() + "\n";
        let _ = stream.write_all(line.as_bytes());
        stream.flush()?;
        return Ok(());
    } else if first_line.starts_with("POST /api/resolve_interrupted_batch") {
        let resolution: InterruptedResolution =
            serde_json::from_value(parsed["resolution"].clone())
                .unwrap_or(InterruptedResolution::RollBack);
        match state.resolve_interrupted_batch(resolution) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/set_manual_firewire_guid") {
        let guid = parsed["guid"].as_str().unwrap_or("");
        match state.set_manual_firewire_guid(guid) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/set_transcode_preset") {
        let preset: TranscodePreset = serde_json::from_value(parsed["preset"].clone())
            .unwrap_or(TranscodePreset::Alac);
        match state.set_transcode_preset(preset) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/stage_track_metadata_edits") {
        let track_ids: Vec<u32> = if let Some(arr) = parsed["trackIds"].as_array() {
            arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect()
        } else if let Some(arr) = parsed["edits"].as_array() {
            arr.iter().filter_map(|item| item["trackId"].as_u64().map(|n| n as u32)).collect()
        } else {
            Vec::new()
        };
        let edits: podloader_core::TrackMetadataEdits = if let Some(arr) = parsed["edits"].as_array() {
            if let Some(first) = arr.first() {
                serde_json::from_value(first["edits"].clone()).unwrap_or_default()
            } else {
                Default::default()
            }
        } else {
            serde_json::from_value(parsed["edits"].clone()).unwrap_or_default()
        };
        match state.stage_track_metadata_edits(&track_ids, edits) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/stage_track_removals") {
        let track_ids: Vec<u32> = parsed["trackIds"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
            .unwrap_or_default();
        match state.stage_track_removals(&track_ids) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/create_device_playlist") {
        let name = parsed["name"].as_str().unwrap_or("New Playlist");
        match state.create_device_playlist(name) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/add_tracks_to_playlist") {
        let playlist_id = parsed["playlistId"].as_u64().unwrap_or(0);
        let track_ids: Vec<u32> = parsed["trackIds"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
            .unwrap_or_default();
        match state.add_tracks_to_playlist(playlist_id, &track_ids) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/reorder_playlist_tracks") {
        let playlist_id = parsed["playlistId"].as_u64().unwrap_or(0);
        let track_ids: Vec<u32> = parsed["trackIds"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
            .unwrap_or_default();
        match state.reorder_playlist_tracks(playlist_id, &track_ids) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/remove_tracks_from_playlist") {
        let playlist_id = parsed["playlistId"].as_u64().unwrap_or(0);
        let track_ids: Vec<u32> = parsed["trackIds"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
            .unwrap_or_default();
        match state.remove_tracks_from_playlist(playlist_id, &track_ids) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/rename_device_playlist") {
        let playlist_id = parsed["playlistId"].as_u64().unwrap_or(0);
        let new_name = parsed["newName"].as_str().unwrap_or("");
        match state.rename_device_playlist(playlist_id, new_name) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/delete_device_playlist") {
        let playlist_id = parsed["playlistId"].as_u64().unwrap_or(0);
        match state.delete_device_playlist(playlist_id) {
            Ok(()) => serde_json::json!({ "status": "ok" }).to_string(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/stage_orphan_recovery") {
        let obfuscated_path = parsed["obfuscatedPath"].as_str().unwrap_or("");
        match state.stage_orphan_recovery(obfuscated_path) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/stage_orphan_deletion") {
        let obfuscated_path = parsed["obfuscatedPath"].as_str().unwrap_or("");
        match state.stage_orphan_deletion(obfuscated_path) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/export_device_tracks") {
        let track_ids: Vec<u32> = parsed["trackIds"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
            .unwrap_or_default();
        let destination_dir = parsed["destinationDir"].as_str().unwrap_or(".");
        match state.export_device_tracks(track_ids, destination_dir) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/export_device_playlist") {
        let playlist_id = parsed["playlistId"].as_u64().unwrap_or(0);
        let destination_dir = parsed["destinationDir"].as_str().unwrap_or(".");
        match state.export_device_playlist(playlist_id, destination_dir) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/eject_device") {
        let force = parsed["force"].as_bool().unwrap_or(false);
        match state.eject_device(force) {
            Ok(()) => serde_json::json!({ "status": "ok" }).to_string(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/play_host_file") {
        let host_file: podloader_core::HostFile =
            serde_json::from_value(parsed["hostFile"].clone()).unwrap_or_default();
        match state.play_host_file(host_file) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/play_device_track") {
        let track_id = parsed["trackId"].as_u64().unwrap_or(0) as u32;
        match state.play_device_track(track_id) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/pause_playback") {
        match state.pause_playback() {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/resume_playback") {
        match state.resume_playback() {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/stop_playback") {
        match state.stop_playback() {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/seek_playback") {
        let position_ms = parsed["positionMs"].as_u64().unwrap_or(0) as u32;
        match state.seek_playback(position_ms) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/set_playback_volume") {
        let volume = parsed["volume"].as_f64().unwrap_or(1.0) as f32;
        match state.set_playback_volume(volume) {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("GET /api/get_playback_status") {
        match state.get_playback_status() {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else {
        serde_json::json!({ "status": "ok" }).to_string()
    };

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_json.len(),
        response_json
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}
