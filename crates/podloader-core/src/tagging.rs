use std::fs;
use std::path::Path;

use crate::{DeviceTrack, PodLoaderError, TrackMetadataEdits};

pub fn update_audio_file_tags(
    file_path: &Path,
    edits: &TrackMetadataEdits,
    current: &DeviceTrack,
) -> Result<(), PodLoaderError> {
    let bytes = fs::read(file_path)?;
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();

    let updated_bytes = match ext.as_str() {
        "MP3" => update_mp3_id3v2_tags(&bytes, edits, current),
        "M4A" | "AAC" | "ALAC" => update_mp4_tags(&bytes, edits, current),
        _ => return Ok(()),
    };

    fs::write(file_path, &updated_bytes)?;
    Ok(())
}

fn encode_id3v2_text_frame(frame_id: &[u8; 4], text: &str) -> Vec<u8> {
    let text_bytes = text.as_bytes();
    let payload_len = 1 + text_bytes.len() as u32; // encoding byte 3 (UTF-8) + text
    let mut frame = Vec::with_capacity(10 + payload_len as usize);
    frame.extend_from_slice(frame_id);
    frame.extend_from_slice(&payload_len.to_be_bytes());
    frame.extend_from_slice(&[0, 0]); // flags
    frame.push(3); // UTF-8 text encoding
    frame.extend_from_slice(text_bytes);
    frame
}

pub fn update_mp3_id3v2_tags(
    bytes: &[u8],
    edits: &TrackMetadataEdits,
    current: &DeviceTrack,
) -> Vec<u8> {
    let (audio_offset, existing_apic) = if bytes.starts_with(b"ID3") && bytes.len() >= 10 {
        let tag_size = (((bytes[6] as usize) & 0x7F) << 21)
            | (((bytes[7] as usize) & 0x7F) << 14)
            | (((bytes[8] as usize) & 0x7F) << 7)
            | ((bytes[9] as usize) & 0x7F);
        let tag_end = (10 + tag_size).min(bytes.len());
        let mut apic_data = None;
        let mut cur = 10;
        while cur + 10 <= tag_end {
            let frame_id = &bytes[cur..cur + 4];
            if frame_id[0] == 0 {
                break;
            }
            // ID3v2.3 / ID3v2.4 frame size
            let frame_size = u32::from_be_bytes([
                bytes[cur + 4],
                bytes[cur + 5],
                bytes[cur + 6],
                bytes[cur + 7],
            ]) as usize;
            let frame_total = 10 + frame_size;
            if cur + frame_total <= tag_end {
                if frame_id == b"APIC" {
                    apic_data = Some(bytes[cur..cur + frame_total].to_vec());
                    break;
                }
                cur += frame_total;
            } else {
                break;
            }
        }
        (tag_end, apic_data)
    } else {
        (0, None)
    };

    let title = edits.title.as_deref().unwrap_or(&current.title);
    let artist = edits.artist.as_deref().unwrap_or(&current.artist);
    let album = edits.album.as_deref().unwrap_or(&current.album);
    let album_artist = edits
        .album_artist
        .as_deref()
        .or(current.album_artist.as_deref())
        .unwrap_or(artist);
    let track_num = edits.track_number.unwrap_or(current.track_number);
    let disc_num = edits.disc_number.unwrap_or(current.disc_number);
    let genre = edits.genre.as_deref().unwrap_or(&current.genre);
    let year = edits.year.unwrap_or(current.year);
    let is_comp = edits.is_compilation.unwrap_or(current.is_compilation);
    let rating = edits.rating.unwrap_or(current.rating);

    let mut frames = Vec::new();
    if !title.is_empty() {
        frames.extend_from_slice(&encode_id3v2_text_frame(b"TIT2", title));
    }
    if !artist.is_empty() {
        frames.extend_from_slice(&encode_id3v2_text_frame(b"TPE1", artist));
    }
    if !album.is_empty() {
        frames.extend_from_slice(&encode_id3v2_text_frame(b"TALB", album));
    }
    if !album_artist.is_empty() {
        frames.extend_from_slice(&encode_id3v2_text_frame(b"TPE2", album_artist));
    }
    if track_num > 0 {
        frames.extend_from_slice(&encode_id3v2_text_frame(b"TRCK", &track_num.to_string()));
    }
    if disc_num > 0 {
        frames.extend_from_slice(&encode_id3v2_text_frame(b"TPOS", &disc_num.to_string()));
    }
    if !genre.is_empty() {
        frames.extend_from_slice(&encode_id3v2_text_frame(b"TCON", genre));
    }
    if year > 0 {
        frames.extend_from_slice(&encode_id3v2_text_frame(b"TYER", &year.to_string()));
    }
    if is_comp {
        frames.extend_from_slice(&encode_id3v2_text_frame(b"TCMP", "1"));
    }
    if rating > 0 {
        frames.extend_from_slice(&encode_id3v2_popm_frame(rating));
    }
    if let Some(ref apic) = existing_apic {
        frames.extend_from_slice(apic);
    }

    let size = frames.len() as u32;
    let synchsafe = [
        ((size >> 21) & 0x7F) as u8,
        ((size >> 14) & 0x7F) as u8,
        ((size >> 7) & 0x7F) as u8,
        (size & 0x7F) as u8,
    ];

    let mut out = Vec::with_capacity(10 + frames.len() + (bytes.len() - audio_offset));
    out.extend_from_slice(b"ID3");
    out.push(3); // v2.3
    out.push(0);
    out.push(0); // flags
    out.extend_from_slice(&synchsafe);
    out.extend_from_slice(&frames);
    out.extend_from_slice(&bytes[audio_offset..]);
    out
}

fn mp4_box(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let total_len = (8 + payload.len()) as u32;
    let mut out = Vec::with_capacity(total_len as usize);
    out.extend_from_slice(&total_len.to_be_bytes());
    out.extend_from_slice(fourcc);
    out.extend_from_slice(payload);
    out
}

fn mp4_full_box(fourcc: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let total_len = (12 + payload.len()) as u32;
    let mut out = Vec::with_capacity(total_len as usize);
    out.extend_from_slice(&total_len.to_be_bytes());
    out.extend_from_slice(fourcc);
    let vf = ((version as u32) << 24) | (flags & 0x00FF_FFFF);
    out.extend_from_slice(&vf.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn encode_ilst_utf8(atom_id: &[u8; 4], text: &str) -> Vec<u8> {
    let data_box = mp4_full_box(b"data", 0, 1, &{
        let mut p = vec![0u8; 4];
        p.extend_from_slice(text.as_bytes());
        p
    });
    mp4_box(atom_id, &data_box)
}

fn encode_ilst_u16_pair(atom_id: &[u8; 4], num: u16, total: u16, pad: usize) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + 6 + pad);
    p.extend_from_slice(&0u32.to_be_bytes());
    p.extend_from_slice(&0u16.to_be_bytes());
    p.extend_from_slice(&num.to_be_bytes());
    p.extend_from_slice(&total.to_be_bytes());
    p.extend(std::iter::repeat(0u8).take(pad));
    let data_box = mp4_full_box(b"data", 0, 0, &p);
    mp4_box(atom_id, &data_box)
}

fn encode_ilst_u8_flag(atom_id: &[u8; 4], val: u8) -> Vec<u8> {
    let mut p = Vec::with_capacity(5);
    p.extend_from_slice(&0u32.to_be_bytes());
    p.push(val);
    let data_box = mp4_full_box(b"data", 0, 21, &p);
    mp4_box(atom_id, &data_box)
}

pub fn update_mp4_tags(
    bytes: &[u8],
    edits: &TrackMetadataEdits,
    current: &DeviceTrack,
) -> Vec<u8> {
    let title = edits.title.as_deref().unwrap_or(&current.title);
    let artist = edits.artist.as_deref().unwrap_or(&current.artist);
    let album = edits.album.as_deref().unwrap_or(&current.album);
    let album_artist = edits
        .album_artist
        .as_deref()
        .or(current.album_artist.as_deref())
        .unwrap_or(artist);
    let track_num = edits.track_number.unwrap_or(current.track_number);
    let disc_num = edits.disc_number.unwrap_or(current.disc_number);
    let genre = edits.genre.as_deref().unwrap_or(&current.genre);
    let year = edits.year.unwrap_or(current.year);
    let is_comp = edits.is_compilation.unwrap_or(current.is_compilation);

    // Build new ilst children
    let mut ilst_children = Vec::new();
    if !title.is_empty() {
        ilst_children.extend_from_slice(&encode_ilst_utf8(b"\xA9nam", title));
    }
    if !artist.is_empty() {
        ilst_children.extend_from_slice(&encode_ilst_utf8(b"\xA9ART", artist));
    }
    if !album.is_empty() {
        ilst_children.extend_from_slice(&encode_ilst_utf8(b"\xA9alb", album));
    }
    if !album_artist.is_empty() {
        ilst_children.extend_from_slice(&encode_ilst_utf8(b"aART", album_artist));
    }
    if !genre.is_empty() {
        ilst_children.extend_from_slice(&encode_ilst_utf8(b"\xA9gen", genre));
    }
    if year > 0 {
        ilst_children.extend_from_slice(&encode_ilst_utf8(b"\xA9day", &year.to_string()));
    }
    if track_num > 0 {
        ilst_children.extend_from_slice(&encode_ilst_u16_pair(b"trkn", track_num as u16, 0, 2));
    }
    if disc_num > 0 {
        ilst_children.extend_from_slice(&encode_ilst_u16_pair(b"disk", disc_num as u16, 0, 0));
    }
    if is_comp {
        ilst_children.extend_from_slice(&encode_ilst_u8_flag(b"cpil", 1));
    }
    let rating = edits.rating.unwrap_or(current.rating);
    if rating > 0 {
        ilst_children.extend_from_slice(&encode_ilst_u8_flag(b"rate", (rating.min(5) * 20) as u8));
    }

    // Preserve existing covr atom if present
    if let Some(covr_pos) = bytes.windows(4).position(|w| w == b"covr") {
        if covr_pos >= 4 {
            let start = covr_pos - 4;
            let len = u32::from_be_bytes([
                bytes[start],
                bytes[start + 1],
                bytes[start + 2],
                bytes[start + 3],
            ]) as usize;
            if start + len <= bytes.len() {
                ilst_children.extend_from_slice(&bytes[start..start + len]);
            }
        }
    }

    let ilst = mp4_box(b"ilst", &ilst_children);
    let mut meta_hdlr_payload = Vec::new();
    meta_hdlr_payload.extend_from_slice(&0u32.to_be_bytes());
    meta_hdlr_payload.extend_from_slice(b"mdir");
    meta_hdlr_payload.extend_from_slice(b"appl");
    meta_hdlr_payload.extend_from_slice(&[0u8; 9]);
    let meta_hdlr = mp4_full_box(b"hdlr", 0, 0, &meta_hdlr_payload);

    let mut meta_payload = Vec::new();
    meta_payload.extend_from_slice(&meta_hdlr);
    meta_payload.extend_from_slice(&ilst);
    let meta_box = mp4_full_box(b"meta", 0, 0, &meta_payload);
    let new_udta = mp4_box(b"udta", &meta_box);

    // Locate top-level moov atom in bytes
    let Some(moov_idx) = bytes.windows(4).position(|w| w == b"moov") else {
        return bytes.to_vec();
    };
    if moov_idx < 4 {
        return bytes.to_vec();
    }
    let moov_start = moov_idx - 4;
    let old_moov_len = u32::from_be_bytes([
        bytes[moov_start],
        bytes[moov_start + 1],
        bytes[moov_start + 2],
        bytes[moov_start + 3],
    ]) as usize;
    let old_moov_end = moov_start + old_moov_len;
    if old_moov_end > bytes.len() {
        return bytes.to_vec();
    }

    let old_moov_bytes = &bytes[moov_start..old_moov_end];
    // Remove existing udta box from moov if present
    let mut moov_inner = Vec::new();
    let mut offset = 8usize;
    while offset + 8 <= old_moov_bytes.len() {
        let child_len = u32::from_be_bytes([
            old_moov_bytes[offset],
            old_moov_bytes[offset + 1],
            old_moov_bytes[offset + 2],
            old_moov_bytes[offset + 3],
        ]) as usize;
        let fourcc = &old_moov_bytes[offset + 4..offset + 8];
        if child_len == 0 || offset + child_len > old_moov_bytes.len() {
            break;
        }
        if fourcc != b"udta" {
            moov_inner.extend_from_slice(&old_moov_bytes[offset..offset + child_len]);
        }
        offset += child_len;
    }
    moov_inner.extend_from_slice(&new_udta);
    let new_moov = mp4_box(b"moov", &moov_inner);

    let delta = (new_moov.len() as i64) - (old_moov_len as i64);

    let mut out_moov = new_moov;
    // If moov is before mdat and changed size, adjust stco chunk offsets
    if delta != 0 {
        if let Some(mdat_pos) = bytes.windows(4).position(|w| w == b"mdat") {
            if moov_start < mdat_pos {
                adjust_stco_offsets(&mut out_moov, delta);
            }
        }
    }

    let mut result = Vec::with_capacity((bytes.len() as i64 + delta).max(0) as usize);
    result.extend_from_slice(&bytes[..moov_start]);
    result.extend_from_slice(&out_moov);
    result.extend_from_slice(&bytes[old_moov_end..]);
    result
}

fn adjust_stco_offsets(moov: &mut [u8], delta: i64) {
    let mut pos = 0usize;
    while let Some(stco_rel) = moov[pos..].windows(4).position(|w| w == b"stco") {
        let stco_idx = pos + stco_rel;
        if stco_idx >= 4 && stco_idx + 12 <= moov.len() {
            let entry_count = u32::from_be_bytes([
                moov[stco_idx + 8],
                moov[stco_idx + 9],
                moov[stco_idx + 10],
                moov[stco_idx + 11],
            ]) as usize;
            let mut off = stco_idx + 12;
            for _ in 0..entry_count {
                if off + 4 > moov.len() {
                    break;
                }
                let old_val = u32::from_be_bytes([
                    moov[off],
                    moov[off + 1],
                    moov[off + 2],
                    moov[off + 3],
                ]);
                let new_val = ((old_val as i64) + delta).max(0) as u32;
                moov[off..off + 4].copy_from_slice(&new_val.to_be_bytes());
                off += 4;
            }
        }
        pos = stco_idx + 4;
    }
}

fn encode_id3v2_popm_frame(rating_stars: u8) -> Vec<u8> {
    let raw_rating: u8 = match rating_stars {
        1 => 1,
        2 => 64,
        3 => 128,
        4 => 196,
        5 => 255,
        _ => 0,
    };
    let mut payload = Vec::new();
    payload.extend_from_slice(b"iTunes\0");
    payload.push(raw_rating);
    payload.extend_from_slice(&0u32.to_be_bytes()); // counter

    let mut frame = Vec::new();
    frame.extend_from_slice(b"POPM");
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&[0, 0]); // flags
    frame.extend_from_slice(&payload);
    frame
}
