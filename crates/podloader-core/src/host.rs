use std::fs;
use std::path::{Path, PathBuf};

use crate::{HostDirectoryEntry, HostDirectoryListing, HostFile, PodLoaderError};

const SUPPORTED_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "alac", "wav"];

pub fn is_supported_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|s| s.eq_ignore_ascii_case(ext))
        })
        .unwrap_or(false)
}

pub fn is_native_ipod_format(format: &str) -> bool {
    matches!(
        format.to_ascii_uppercase().as_str(),
        "MP3" | "AAC" | "ALAC" | "M4A" | "WAV"
    )
}

pub fn browse_directory(dir_path: &Path) -> Result<HostDirectoryListing, PodLoaderError> {
    if !dir_path.is_dir() {
        return Err(PodLoaderError::DirectoryNotFound(dir_path.to_path_buf()));
    }

    let mut subdirectories = Vec::new();
    let mut files = Vec::new();

    for entry in fs::read_dir(dir_path)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            subdirectories.push(HostDirectoryEntry {
                name: file_name,
                path,
            });
        } else if path.is_file() && is_supported_audio_path(&path) {
            if let Ok(host_file) = inspect_host_file(&path) {
                files.push(host_file);
            }
        }
    }

    subdirectories.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| {
        (a.track_number, a.file_name.to_lowercase())
            .cmp(&(b.track_number, b.file_name.to_lowercase()))
    });

    Ok(HostDirectoryListing {
        current_path: dir_path.to_path_buf(),
        parent_path: dir_path.parent().map(|p| p.to_path_buf()),
        subdirectories,
        files,
    })
}

pub fn collect_host_files_from_paths(paths: &[PathBuf]) -> Result<Vec<HostFile>, PodLoaderError> {
    let mut out = Vec::new();
    for path in paths {
        collect_recursive(path, &mut out)?;
    }
    Ok(out)
}

fn collect_recursive(path: &Path, out: &mut Vec<HostFile>) -> Result<(), PodLoaderError> {
    if path.is_file() {
        if is_supported_audio_path(path) {
            out.push(inspect_host_file(path)?);
        }
    } else if path.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(path)?
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for child in entries {
            collect_recursive(&child, out)?;
        }
    }
    Ok(())
}

pub fn inspect_host_file(path: &Path) -> Result<HostFile, PodLoaderError> {
    let bytes = fs::read(path)?;
    let file_size = bytes.len() as u64;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown")
        .to_string();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();

    let (fallback_track, fallback_title) = parse_stem_track_and_title(path);

    let mut host_file = HostFile {
        path: path.to_path_buf(),
        file_name,
        format: if ext == "M4A" {
            detect_mp4_codec(&bytes).unwrap_or_else(|| "AAC".to_string())
        } else {
            ext.clone()
        },
        file_size,
        title: fallback_title,
        artist: "Unknown Artist".to_string(),
        album: "Unknown Album".to_string(),
        track_number: fallback_track.unwrap_or(1),
        disc_number: 1,
        genre: String::new(),
        year: 0,
        duration_ms: 0,
        bitrate_kbps: 0,
    };

    match ext.as_str() {
        "MP3" => parse_mp3_metadata(&bytes, &mut host_file),
        "WAV" => parse_wav_metadata(&bytes, &mut host_file),
        "M4A" | "AAC" | "ALAC" => parse_mp4_metadata(&bytes, &mut host_file),
        _ => {}
    }

    Ok(host_file)
}

fn parse_stem_track_and_title(path: &Path) -> (Option<u32>, String) {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .trim();

    if let Some((left, right)) = stem.split_once(" - ") {
        if let Ok(num) = left.trim().parse::<u32>() {
            let title = right.trim();
            if !title.is_empty() {
                return (Some(num), title.to_string());
            }
        }
    }
    (None, stem.to_string())
}

fn parse_mp3_metadata(bytes: &[u8], file: &mut HostFile) {
    let mut audio_offset = 0usize;
    if bytes.get(0..3) == Some(b"ID3") && bytes.len() >= 10 {
        let version = bytes[3];
        let tag_size = (((bytes[6] as usize) & 0x7F) << 21)
            | (((bytes[7] as usize) & 0x7F) << 14)
            | (((bytes[8] as usize) & 0x7F) << 7)
            | ((bytes[9] as usize) & 0x7F);
        let tag_end = (10 + tag_size).min(bytes.len());
        audio_offset = tag_end;

        let mut offset = 10usize;
        while offset + 10 <= tag_end {
            let frame_id = &bytes[offset..offset + 4];
            if frame_id == [0, 0, 0, 0] {
                break;
            }
            let frame_size = if version == 4 {
                (((bytes[offset + 4] as usize) & 0x7F) << 21)
                    | (((bytes[offset + 5] as usize) & 0x7F) << 14)
                    | (((bytes[offset + 6] as usize) & 0x7F) << 7)
                    | ((bytes[offset + 7] as usize) & 0x7F)
            } else {
                u32::from_be_bytes([
                    bytes[offset + 4],
                    bytes[offset + 5],
                    bytes[offset + 6],
                    bytes[offset + 7],
                ]) as usize
            };
            if frame_size == 0 || offset + 10 + frame_size > tag_end {
                break;
            }
            let payload = &bytes[offset + 10..offset + 10 + frame_size];
            if let Some(text) = decode_id3_text(payload) {
                match frame_id {
                    b"TIT2" if !text.is_empty() => file.title = text,
                    b"TPE1" if !text.is_empty() => file.artist = text,
                    b"TALB" if !text.is_empty() => file.album = text,
                    b"TCON" if !text.is_empty() => file.genre = text,
                    b"TRCK" => {
                        if let Some(n) = text
                            .split('/')
                            .next()
                            .and_then(|s| s.trim().parse::<u32>().ok())
                        {
                            file.track_number = n;
                        }
                    }
                    b"TPOS" => {
                        if let Some(n) = text
                            .split('/')
                            .next()
                            .and_then(|s| s.trim().parse::<u32>().ok())
                        {
                            file.disc_number = n;
                        }
                    }
                    b"TYER" | b"TDRC" => {
                        if let Some(y) = text.get(0..4).and_then(|s| s.parse::<u32>().ok()) {
                            file.year = y;
                        }
                    }
                    _ => {}
                }
            }
            offset += 10 + frame_size;
        }
    }

    let audio_slice = bytes.get(audio_offset..).unwrap_or(&[]);
    let bitrate_kbps = detect_mpeg_bitrate_kbps(audio_slice).unwrap_or(128);
    file.bitrate_kbps = bitrate_kbps;
    if bitrate_kbps > 0 {
        let audio_bytes = audio_slice.len() as u64;
        file.duration_ms = ((audio_bytes * 8) / (bitrate_kbps as u64)) as u32;
    }
}

fn decode_id3_text(payload: &[u8]) -> Option<String> {
    let (&encoding, rest) = payload.split_first()?;
    let raw = match encoding {
        0 | 3 => String::from_utf8_lossy(rest).to_string(),
        1 => {
            let body = if rest.starts_with(&[0xFF, 0xFE]) || rest.starts_with(&[0xFE, 0xFF]) {
                &rest[2..]
            } else {
                rest
            };
            let is_be = rest.starts_with(&[0xFE, 0xFF]);
            let u16s: Vec<u16> = body
                .chunks_exact(2)
                .map(|c| {
                    if is_be {
                        u16::from_be_bytes([c[0], c[1]])
                    } else {
                        u16::from_le_bytes([c[0], c[1]])
                    }
                })
                .collect();
            String::from_utf16_lossy(&u16s)
        }
        2 => {
            let u16s: Vec<u16> = rest
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&u16s)
        }
        _ => String::from_utf8_lossy(rest).to_string(),
    };
    Some(raw.trim_matches('\0').trim().to_string())
}

fn detect_mpeg_bitrate_kbps(audio: &[u8]) -> Option<u32> {
    const BITRATES_MPEG1_L3: [u32; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    for window in audio.windows(4).take(4096) {
        if window[0] == 0xFF && (window[1] & 0xE0) == 0xE0 {
            let bitrate_idx = ((window[2] >> 4) & 0x0F) as usize;
            let rate = BITRATES_MPEG1_L3[bitrate_idx];
            if rate > 0 {
                return Some(rate);
            }
        }
    }
    None
}

fn parse_wav_metadata(bytes: &[u8], file: &mut HostFile) {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return;
    }
    let mut offset = 12usize;
    let mut byte_rate = 0u32;
    let mut data_len = 0u32;

    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let payload_start = offset + 8;
        let payload_end = (payload_start + chunk_size).min(bytes.len());

        if chunk_id == b"fmt " && payload_end - payload_start >= 16 {
            byte_rate = u32::from_le_bytes([
                bytes[payload_start + 8],
                bytes[payload_start + 9],
                bytes[payload_start + 10],
                bytes[payload_start + 11],
            ]);
        } else if chunk_id == b"data" {
            data_len = chunk_size as u32;
        }
        offset = payload_start + chunk_size + (chunk_size % 2);
    }

    if byte_rate > 0 {
        file.bitrate_kbps = (byte_rate * 8) / 1000;
        file.duration_ms = ((data_len as u64 * 1000) / byte_rate as u64) as u32;
    }
}

fn detect_mp4_codec(bytes: &[u8]) -> Option<String> {
    if bytes.windows(4).any(|w| w == b"alac") {
        Some("ALAC".to_string())
    } else if bytes.windows(4).any(|w| w == b"mp4a") {
        Some("AAC".to_string())
    } else {
        None
    }
}

fn parse_mp4_metadata(bytes: &[u8], file: &mut HostFile) {
    if let Some(codec) = detect_mp4_codec(bytes) {
        file.format = codec;
    }

    // Parse mvhd atom for timescale and duration
    if let Some(mvhd_pos) = bytes.windows(4).position(|w| w == b"mvhd") {
        let body = &bytes[mvhd_pos + 4..];
        if body.len() >= 20 {
            let version = body[0];
            if version == 0 {
                let timescale = u32::from_be_bytes([body[12], body[13], body[14], body[15]]) as u64;
                let duration = u32::from_be_bytes([body[16], body[17], body[18], body[19]]) as u64;
                if timescale > 0 {
                    file.duration_ms = ((duration * 1000) / timescale) as u32;
                }
            } else if version == 1 && body.len() >= 28 {
                let timescale = u32::from_be_bytes([body[20], body[21], body[22], body[23]]) as u64;
                let duration = u64::from_be_bytes([
                    body[24], body[25], body[26], body[27], body[28], body[29], body[30], body[31],
                ]);
                if timescale > 0 {
                    file.duration_ms = ((duration * 1000) / timescale) as u32;
                }
            }
        }
    }

    // Parse iTunes ilst atoms (©nam, ©ART, ©alb, ©day, ©gen, trkn, disk)
    if let Some(title) = extract_mp4_data_text(bytes, b"\xA9nam") {
        if !title.is_empty() {
            file.title = title;
        }
    }
    if let Some(artist) = extract_mp4_data_text(bytes, b"\xA9ART") {
        if !artist.is_empty() {
            file.artist = artist;
        }
    }
    if let Some(album) = extract_mp4_data_text(bytes, b"\xA9alb") {
        if !album.is_empty() {
            file.album = album;
        }
    }
    if let Some(genre) = extract_mp4_data_text(bytes, b"\xA9gen") {
        if !genre.is_empty() {
            file.genre = genre;
        }
    }
    if let Some(year_str) = extract_mp4_data_text(bytes, b"\xA9day") {
        if let Some(y) = year_str.get(0..4).and_then(|s| s.parse::<u32>().ok()) {
            file.year = y;
        }
    }
    if let Some(trkn) = extract_mp4_data_bytes(bytes, b"trkn") {
        if trkn.len() >= 4 {
            let num = u16::from_be_bytes([trkn[2], trkn[3]]) as u32;
            if num > 0 {
                file.track_number = num;
            }
        }
    }
    if let Some(disk) = extract_mp4_data_bytes(bytes, b"disk") {
        if disk.len() >= 4 {
            let num = u16::from_be_bytes([disk[2], disk[3]]) as u32;
            if num > 0 {
                file.disc_number = num;
            }
        }
    }

    if file.duration_ms > 0 {
        file.bitrate_kbps = ((file.file_size * 8) / (file.duration_ms as u64)).max(64) as u32;
    } else {
        file.duration_ms = 180_000;
        file.bitrate_kbps = if file.format == "ALAC" { 900 } else { 256 };
    }
}

fn extract_mp4_data_bytes<'a>(bytes: &'a [u8], atom_id: &[u8; 4]) -> Option<&'a [u8]> {
    let pos = bytes.windows(4).position(|w| w == atom_id)?;
    let atom_start = pos.checked_sub(4)?;
    let atom_len = u32::from_be_bytes([
        bytes[atom_start],
        bytes[atom_start + 1],
        bytes[atom_start + 2],
        bytes[atom_start + 3],
    ]) as usize;
    let atom_end = (atom_start + atom_len).min(bytes.len());
    let inner = bytes.get(pos + 4..atom_end)?;
    let data_pos = inner.windows(4).position(|w| w == b"data")?;
    let data_box_start = data_pos.checked_sub(4)?;
    let data_len = u32::from_be_bytes([
        inner[data_box_start],
        inner[data_box_start + 1],
        inner[data_box_start + 2],
        inner[data_box_start + 3],
    ]) as usize;
    let payload_start = data_box_start + 16;
    let payload_end = (data_box_start + data_len).min(inner.len());
    inner.get(payload_start..payload_end)
}

fn extract_mp4_data_text(bytes: &[u8], atom_id: &[u8; 4]) -> Option<String> {
    let payload = extract_mp4_data_bytes(bytes, atom_id)?;
    Some(String::from_utf8_lossy(payload).trim_matches('\0').trim().to_string())
}
