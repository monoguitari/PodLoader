#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::TcpListener;
use podloader_core::DeviceConnectionState;
use podloader_shell::AppState;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Optional positional path override: `cargo run -p podloader-shell -- /mnt/d`
    for arg in args.iter().skip(1) {
        if !arg.starts_with("--") {
            std::env::set_var("PODLOADER_DEVICE_MOUNT", arg);
        }
    }

    let state = AppState::default();

    if args.iter().any(|a| a == "--serve") {
        run_dev_bridge_server(&state);
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

fn run_dev_bridge_server(state: &AppState) {
    let addr = "127.0.0.1:41731";
    let listener = TcpListener::bind(addr).expect("bind dev bridge server on 127.0.0.1:41731");
    println!("PodLoader Rust backend listening on http://{}", addr);

    for stream in listener.incoming().flatten() {
        let _ = handle_http_client(stream, state);
    }
}

fn handle_http_client(mut stream: std::net::TcpStream, state: &AppState) -> std::io::Result<()> {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf)?;
    if n == 0 {
        return Ok(());
    }
    let req = String::from_utf8_lossy(&buf[..n]);
    let first_line = req.lines().next().unwrap_or("");

    let response_json = if first_line.starts_with("GET /api/scan_device") {
        match state.scan_device() {
            Ok(res) => serde_json::to_string(&res).unwrap_or_default(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        }
    } else if first_line.starts_with("POST /api/initialize_device") {
        let body = req.split("\r\n\r\n").nth(1).unwrap_or("{}");
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let mount_path = parsed["mountPath"].as_str().unwrap_or("");
        let device_name = parsed["deviceName"].as_str().unwrap_or("My iPod");
        match state.initialize_device(mount_path, device_name) {
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
