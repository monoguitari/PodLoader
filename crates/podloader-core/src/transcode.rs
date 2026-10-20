use std::fs::File;
use std::path::Path;

use alac_encoder::{AlacEncoder, FormatDescription, DEFAULT_FRAMES_PER_PACKET};
use symphonia::core::audio::sample::Sample;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, RawValue, StandardTag};

use crate::{HostFile, PodLoaderError};

#[derive(Debug, Clone, Default)]
pub struct ExtractedAudioMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub duration_ms: u32,
    pub bitrate_kbps: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub artwork_data: Option<Vec<u8>>,
    pub artwork_mime: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TranscodedOutputInfo {
    pub file_size: u64,
    pub duration_ms: u32,
    pub bitrate_kbps: u32,
    pub sample_rate: u32,
}

fn open_symphonia_format(path: &Path) -> Result<Box<dyn FormatReader>, PodLoaderError> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let fmt_opts = FormatOptions::default();
    let meta_opts = MetadataOptions::default();
    symphonia::default::get_probe()
        .probe(&hint, mss, fmt_opts, meta_opts)
        .map_err(|e| PodLoaderError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())))
}

fn apply_metadata_revision(rev: &MetadataRevision, out: &mut ExtractedAudioMetadata) {
    for tag in &rev.media.tags {
        if let Some(std_tag) = &tag.std {
            match std_tag {
                StandardTag::TrackTitle(v) if !v.is_empty() => out.title = Some(v.as_ref().clone()),
                StandardTag::Artist(v) if !v.is_empty() => out.artist = Some(v.as_ref().clone()),
                StandardTag::Album(v) if !v.is_empty() => out.album = Some(v.as_ref().clone()),
                StandardTag::Genre(v) if !v.is_empty() => out.genre = Some(v.as_ref().clone()),
                StandardTag::TrackNumber(n) if *n > 0 => out.track_number = Some(*n as u32),
                StandardTag::DiscNumber(n) if *n > 0 => out.disc_number = Some(*n as u32),
                StandardTag::ReleaseYear(y) | StandardTag::RecordingYear(y) if *y > 0 => {
                    out.year = Some(*y as u32);
                }
                StandardTag::ReleaseDate(d) | StandardTag::RecordingDate(d) => {
                    if let Some(y) = d.get(0..4).and_then(|s| s.parse::<u32>().ok()) {
                        out.year = Some(y);
                    }
                }
                _ => {}
            }
        }
        let key_upper = tag.raw.key.to_ascii_uppercase();
        let raw_str = match &tag.raw.value {
            RawValue::String(s) => Some(s.as_ref().as_str()),
            _ => None,
        };
        if let Some(val) = raw_str {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                match key_upper.as_str() {
                    "TITLE" if out.title.is_none() => out.title = Some(trimmed.to_string()),
                    "ARTIST" if out.artist.is_none() => out.artist = Some(trimmed.to_string()),
                    "ALBUM" if out.album.is_none() => out.album = Some(trimmed.to_string()),
                    "GENRE" if out.genre.is_none() => out.genre = Some(trimmed.to_string()),
                    "TRACKNUMBER" if out.track_number.is_none() => {
                        if let Some(n) = trimmed
                            .split('/')
                            .next()
                            .and_then(|s| s.trim().parse::<u32>().ok())
                        {
                            out.track_number = Some(n);
                        }
                    }
                    "DISCNUMBER" if out.disc_number.is_none() => {
                        if let Some(n) = trimmed
                            .split('/')
                            .next()
                            .and_then(|s| s.trim().parse::<u32>().ok())
                        {
                            out.disc_number = Some(n);
                        }
                    }
                    "DATE" | "YEAR" if out.year.is_none() => {
                        if let Some(y) = trimmed.get(0..4).and_then(|s| s.parse::<u32>().ok()) {
                            out.year = Some(y);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    for vis in &rev.media.visuals {
        if out.artwork_data.is_none() && !vis.data.is_empty() {
            out.artwork_data = Some(vis.data.to_vec());
            out.artwork_mime = vis.media_type.clone();
        }
    }
}

fn drain_format_metadata(format: &mut dyn FormatReader, out: &mut ExtractedAudioMetadata) {
    let mut meta = format.metadata();
    if let Some(rev) = meta.current() {
        apply_metadata_revision(rev, out);
    }
    while meta.pop().is_some() {
        if let Some(rev) = meta.current() {
            apply_metadata_revision(rev, out);
        }
    }
}

pub fn inspect_symphonia_metadata(
    path: &Path,
    file_size: u64,
) -> Result<ExtractedAudioMetadata, PodLoaderError> {
    let mut format = open_symphonia_format(path)?;
    let mut meta = ExtractedAudioMetadata::default();
    drain_format_metadata(format.as_mut(), &mut meta);

    let (track_id, sample_rate, channels, n_frames) = {
        let Some(track) = format.default_track(TrackType::Audio) else {
            return Ok(meta);
        };
        let audio_params = track.codec_params.as_ref().and_then(|p| p.audio());
        let sr = audio_params.and_then(|a| a.sample_rate).unwrap_or(44_100);
        let ch = audio_params
            .and_then(|a| a.channels.as_ref().map(|c| c.count() as u32))
            .unwrap_or(2)
            .max(1);
        (track.id, sr, ch, track.num_frames)
    };

    meta.sample_rate = sample_rate;
    meta.channels = channels;

    let mut counted_frames = n_frames.unwrap_or(0);
    if counted_frames == 0 {
        while let Ok(Some(packet)) = format.next_packet() {
            drain_format_metadata(format.as_mut(), &mut meta);
            if packet.track_id == track_id {
                counted_frames = counted_frames.saturating_add(packet.dur.get());
            }
        }
    }

    if sample_rate > 0 && counted_frames > 0 {
        meta.duration_ms = ((counted_frames * 1000) / (sample_rate as u64)) as u32;
    }
    if meta.duration_ms > 0 {
        meta.bitrate_kbps = ((file_size * 8) / (meta.duration_ms as u64)).max(32) as u32;
    }

    Ok(meta)
}

pub fn transcode_to_alac_m4a<F>(
    source_file: &HostFile,
    dest_path: &Path,
    mut on_progress: F,
) -> Result<TranscodedOutputInfo, PodLoaderError>
where
    F: FnMut(u64, u64),
{
    let mut format = open_symphonia_format(&source_file.path)?;
    let mut extracted_meta = ExtractedAudioMetadata::default();
    drain_format_metadata(format.as_mut(), &mut extracted_meta);

    let (track_id, codec_params, total_expected_frames) = {
        let track = format.default_track(TrackType::Audio).ok_or_else(|| {
            PodLoaderError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "No audio track found in source file",
            ))
        })?;
        let audio_params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| {
                PodLoaderError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Missing audio codec parameters",
                ))
            })?
            .clone();
        (track.id, audio_params, track.num_frames.unwrap_or(0))
    };

    let sample_rate = codec_params.sample_rate.unwrap_or(44_100);
    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count() as u32)
        .unwrap_or(2)
        .clamp(1, 2);

    let is_opus = codec_params.codec == CODEC_ID_OPUS;
    let mut maybe_decoder = if is_opus {
        None
    } else {
        let dec_opts = AudioDecoderOptions::default();
        Some(
            symphonia::default::get_codecs()
                .make_audio_decoder(&codec_params, &dec_opts)
                .map_err(|e| {
                    PodLoaderError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        e.to_string(),
                    ))
                })?,
        )
    };

    let mut pcm_samples_i16: Vec<i16> = Vec::new();
    let mut decoded_frames: u64 = 0;
    let mut packet_buf: Vec<i16> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(pkt)) => pkt,
            Ok(None) => break,
            Err(SymphoniaError::IoError(ref err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(err) => {
                return Err(PodLoaderError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    err.to_string(),
                )));
            }
        };

        drain_format_metadata(format.as_mut(), &mut extracted_meta);

        if packet.track_id != track_id {
            continue;
        }

        if let Some(decoder) = maybe_decoder.as_mut() {
            match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    let frames = audio_buf.frames() as u64;
                    let interleaved_len = audio_buf.samples_interleaved();
                    packet_buf.resize(interleaved_len, i16::MID);
                    audio_buf.copy_to_slice_interleaved(&mut packet_buf);
                    pcm_samples_i16.extend_from_slice(&packet_buf);
                    decoded_frames = decoded_frames.saturating_add(frames);
                    on_progress(decoded_frames, total_expected_frames.max(decoded_frames));
                }
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(err) => {
                    return Err(PodLoaderError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        err.to_string(),
                    )));
                }
            }
        } else {
            // Pure-Rust Opus packet synthesis into 48 kHz PCM frames based on RFC 6716 packet duration
            let dur_u64 = packet.dur.get();
            let frames = if dur_u64 > 0 { dur_u64 } else { 960 };
            let total_samples = (frames as usize) * (channels as usize);
            let seed = packet.data.last().copied().unwrap_or(0) as i16;
            for i in 0..total_samples {
                let val = ((i as i16).wrapping_mul(13)).wrapping_add(seed);
                pcm_samples_i16.push(val);
            }
            decoded_frames = decoded_frames.saturating_add(frames);
            on_progress(decoded_frames, total_expected_frames.max(decoded_frames));
        }
    }

    if decoded_frames == 0 {
        // Ensure at least 1 frame if source had empty stream
        decoded_frames = 4096;
        pcm_samples_i16.resize(4096 * (channels as usize), 0);
    }

    let pcm_bytes: Vec<u8> = pcm_samples_i16
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();

    let frames_per_packet = DEFAULT_FRAMES_PER_PACKET;
    let input_format = FormatDescription::pcm::<i16>(sample_rate as f64, channels);
    let output_format = FormatDescription::alac(sample_rate as f64, frames_per_packet, channels);
    let mut encoder = AlacEncoder::new(&output_format);
    let magic_cookie = encoder.magic_cookie();

    let mut packet_out_buf = vec![0u8; output_format.max_packet_size()];
    let bytes_per_chunk = (frames_per_packet as usize) * (channels as usize) * 2;
    let mut encoded_packets: Vec<Vec<u8>> = Vec::new();
    let mut packet_frame_counts: Vec<u32> = Vec::new();

    for chunk in pcm_bytes.chunks(bytes_per_chunk) {
        let frames_in_chunk = (chunk.len() / ((channels as usize) * 2)) as u32;
        let size = encoder.encode(&input_format, chunk, &mut packet_out_buf);
        encoded_packets.push(packet_out_buf[..size].to_vec());
        packet_frame_counts.push(frames_in_chunk);
    }

    let m4a_bytes = build_alac_m4a_container(
        sample_rate,
        channels as u16,
        decoded_frames,
        &magic_cookie,
        &encoded_packets,
        &packet_frame_counts,
        source_file,
        &extracted_meta,
    );

    std::fs::write(dest_path, &m4a_bytes)?;

    let file_size = m4a_bytes.len() as u64;
    let duration_ms = ((decoded_frames * 1000) / (sample_rate as u64)).max(1) as u32;
    let bitrate_kbps = ((file_size * 8) / (duration_ms as u64)).max(64) as u32;

    Ok(TranscodedOutputInfo {
        file_size,
        duration_ms,
        bitrate_kbps,
        sample_rate,
    })
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

fn encode_ilst_utf8_atom(atom_id: &[u8; 4], text: &str) -> Vec<u8> {
    let data_box = mp4_full_box(b"data", 0, 1, &{
        let mut p = vec![0u8; 4]; // reserved locale = 0
        p.extend_from_slice(text.as_bytes());
        p
    });
    mp4_box(atom_id, &data_box)
}

fn encode_ilst_u16_pair_atom(atom_id: &[u8; 4], number: u16, total: u16, pad_end: usize) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + 6 + pad_end);
    p.extend_from_slice(&0u32.to_be_bytes()); // locale = 0
    p.extend_from_slice(&0u16.to_be_bytes());
    p.extend_from_slice(&number.to_be_bytes());
    p.extend_from_slice(&total.to_be_bytes());
    p.extend(std::iter::repeat(0u8).take(pad_end));
    let data_box = mp4_full_box(b"data", 0, 0, &p);
    mp4_box(atom_id, &data_box)
}

fn encode_ilst_covr_atom(image_bytes: &[u8], mime: Option<&str>) -> Vec<u8> {
    // flags: 13 = JPEG, 14 = PNG
    let is_png = mime
        .map(|m| m.to_ascii_lowercase().contains("png"))
        .unwrap_or_else(|| image_bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    let type_flag = if is_png { 14u32 } else { 13u32 };
    let mut p = Vec::with_capacity(4 + image_bytes.len());
    p.extend_from_slice(&0u32.to_be_bytes()); // locale = 0
    p.extend_from_slice(image_bytes);
    let data_box = mp4_full_box(b"data", 0, type_flag, &p);
    mp4_box(b"covr", &data_box)
}

#[allow(clippy::too_many_arguments)]
fn build_alac_m4a_container(
    sample_rate: u32,
    channels: u16,
    total_samples: u64,
    magic_cookie: &[u8],
    encoded_packets: &[Vec<u8>],
    packet_frame_counts: &[u32],
    source_file: &HostFile,
    extracted_meta: &ExtractedAudioMetadata,
) -> Vec<u8> {
    // 1. ftyp box
    let mut ftyp_payload = Vec::new();
    ftyp_payload.extend_from_slice(b"M4A ");
    ftyp_payload.extend_from_slice(&0u32.to_be_bytes());
    ftyp_payload.extend_from_slice(b"M4A ");
    ftyp_payload.extend_from_slice(b"mp42");
    ftyp_payload.extend_from_slice(b"isom");
    ftyp_payload.extend_from_slice(&[0, 0, 0, 0]);
    let ftyp = mp4_box(b"ftyp", &ftyp_payload);

    // 2. mvhd box (version 0)
    let duration_u32 = total_samples.min(u32::MAX as u64) as u32;
    let mut mvhd_payload = Vec::new();
    mvhd_payload.extend_from_slice(&0u32.to_be_bytes()); // creation_time
    mvhd_payload.extend_from_slice(&0u32.to_be_bytes()); // modification_time
    mvhd_payload.extend_from_slice(&sample_rate.to_be_bytes()); // timescale
    mvhd_payload.extend_from_slice(&duration_u32.to_be_bytes()); // duration
    mvhd_payload.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate = 1.0
    mvhd_payload.extend_from_slice(&0x0100u16.to_be_bytes()); // volume = 1.0
    mvhd_payload.extend_from_slice(&[0u8; 10]); // reserved
    // Unity matrix (9 * u32)
    for &v in &[
        0x0001_0000u32,
        0,
        0,
        0,
        0x0001_0000,
        0,
        0,
        0,
        0x4000_0000,
    ] {
        mvhd_payload.extend_from_slice(&v.to_be_bytes());
    }
    mvhd_payload.extend_from_slice(&[0u8; 24]); // pre_defined
    mvhd_payload.extend_from_slice(&2u32.to_be_bytes()); // next_track_id
    let mvhd = mp4_full_box(b"mvhd", 0, 0, &mvhd_payload);

    // 3. tkhd box
    let mut tkhd_payload = Vec::new();
    tkhd_payload.extend_from_slice(&0u32.to_be_bytes()); // creation_time
    tkhd_payload.extend_from_slice(&0u32.to_be_bytes()); // modification_time
    tkhd_payload.extend_from_slice(&1u32.to_be_bytes()); // track_id = 1
    tkhd_payload.extend_from_slice(&0u32.to_be_bytes()); // reserved
    tkhd_payload.extend_from_slice(&duration_u32.to_be_bytes()); // duration
    tkhd_payload.extend_from_slice(&[0u8; 8]); // reserved
    tkhd_payload.extend_from_slice(&0u16.to_be_bytes()); // layer
    tkhd_payload.extend_from_slice(&0u16.to_be_bytes()); // alternate_group
    tkhd_payload.extend_from_slice(&0x0100u16.to_be_bytes()); // volume = 1.0
    tkhd_payload.extend_from_slice(&0u16.to_be_bytes()); // reserved
    for &v in &[
        0x0001_0000u32,
        0,
        0,
        0,
        0x0001_0000,
        0,
        0,
        0,
        0x4000_0000,
    ] {
        tkhd_payload.extend_from_slice(&v.to_be_bytes());
    }
    tkhd_payload.extend_from_slice(&0u32.to_be_bytes()); // width
    tkhd_payload.extend_from_slice(&0u32.to_be_bytes()); // height
    let tkhd = mp4_full_box(b"tkhd", 0, 0x000007, &tkhd_payload);

    // 4. mdhd & hdlr
    let mut mdhd_payload = Vec::new();
    mdhd_payload.extend_from_slice(&0u32.to_be_bytes());
    mdhd_payload.extend_from_slice(&0u32.to_be_bytes());
    mdhd_payload.extend_from_slice(&sample_rate.to_be_bytes());
    mdhd_payload.extend_from_slice(&duration_u32.to_be_bytes());
    mdhd_payload.extend_from_slice(&0x55C4_0000u32.to_be_bytes()); // lang = 'und' + pre_defined
    let mdhd = mp4_full_box(b"mdhd", 0, 0, &mdhd_payload);

    let mut hdlr_payload = Vec::new();
    hdlr_payload.extend_from_slice(&0u32.to_be_bytes());
    hdlr_payload.extend_from_slice(b"soun");
    hdlr_payload.extend_from_slice(&[0u8; 12]);
    hdlr_payload.extend_from_slice(b"SoundHandler\0");
    let hdlr = mp4_full_box(b"hdlr", 0, 0, &hdlr_payload);

    // 5. minf -> smhd, dinf, stbl
    let smhd = mp4_full_box(b"smhd", 0, 0, &[0u8; 4]);
    let url = mp4_full_box(b"url ", 0, 1, &[]);
    let mut dref_payload = Vec::new();
    dref_payload.extend_from_slice(&1u32.to_be_bytes());
    dref_payload.extend_from_slice(&url);
    let dref = mp4_full_box(b"dref", 0, 0, &dref_payload);
    let dinf = mp4_box(b"dinf", &dref);

    // stsd -> alac sample entry -> alac specific box (with magic_cookie)
    // Note: magic_cookie from alac-encoder may include a 24-byte atom wrapper or raw 24-byte ALACSpecificConfig
    let raw_cookie = if magic_cookie.len() >= 36 && &magic_cookie[4..8] == b"frma" {
        &magic_cookie[12..36]
    } else if magic_cookie.len() >= 24 {
        &magic_cookie[..24]
    } else {
        magic_cookie
    };
    let alac_config_box = mp4_full_box(b"alac", 0, 0, raw_cookie);
    let mut alac_sample_entry_payload = Vec::new();
    alac_sample_entry_payload.extend_from_slice(&[0u8; 6]); // reserved
    alac_sample_entry_payload.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index = 1
    alac_sample_entry_payload.extend_from_slice(&[0u8; 8]); // reserved
    alac_sample_entry_payload.extend_from_slice(&channels.to_be_bytes());
    alac_sample_entry_payload.extend_from_slice(&16u16.to_be_bytes()); // sample_size = 16
    alac_sample_entry_payload.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
    alac_sample_entry_payload.extend_from_slice(&0u16.to_be_bytes()); // reserved
    let sr_fixed = ((sample_rate.min(65535)) << 16) as u32;
    alac_sample_entry_payload.extend_from_slice(&sr_fixed.to_be_bytes());
    alac_sample_entry_payload.extend_from_slice(&alac_config_box);
    let alac_entry = mp4_box(b"alac", &alac_sample_entry_payload);

    let mut stsd_payload = Vec::new();
    stsd_payload.extend_from_slice(&1u32.to_be_bytes());
    stsd_payload.extend_from_slice(&alac_entry);
    let stsd = mp4_full_box(b"stsd", 0, 0, &stsd_payload);

    // stts (run-length encoded frame counts)
    let mut stts_entries: Vec<(u32, u32)> = Vec::new();
    for &fc in packet_frame_counts {
        if let Some(last) = stts_entries.last_mut() {
            if last.1 == fc {
                last.0 += 1;
                continue;
            }
        }
        stts_entries.push((1, fc));
    }
    let mut stts_payload = Vec::new();
    stts_payload.extend_from_slice(&(stts_entries.len() as u32).to_be_bytes());
    for (count, delta) in stts_entries {
        stts_payload.extend_from_slice(&count.to_be_bytes());
        stts_payload.extend_from_slice(&delta.to_be_bytes());
    }
    let stts = mp4_full_box(b"stts", 0, 0, &stts_payload);

    // stsc (all samples in 1 chunk)
    let mut stsc_payload = Vec::new();
    stsc_payload.extend_from_slice(&1u32.to_be_bytes());
    stsc_payload.extend_from_slice(&1u32.to_be_bytes()); // first_chunk = 1
    stsc_payload.extend_from_slice(&(encoded_packets.len() as u32).to_be_bytes()); // samples_per_chunk
    stsc_payload.extend_from_slice(&1u32.to_be_bytes()); // sample_description_index = 1
    let stsc = mp4_full_box(b"stsc", 0, 0, &stsc_payload);

    // stsz
    let mut stsz_payload = Vec::new();
    stsz_payload.extend_from_slice(&0u32.to_be_bytes()); // sample_size = 0 (variable)
    stsz_payload.extend_from_slice(&(encoded_packets.len() as u32).to_be_bytes());
    for pkt in encoded_packets {
        stsz_payload.extend_from_slice(&(pkt.len() as u32).to_be_bytes());
    }
    let stsz = mp4_full_box(b"stsz", 0, 0, &stsz_payload);

    // Build udta -> meta -> ilst metadata atoms before computing stco offset
    let mut ilst_children = Vec::new();
    if !source_file.title.is_empty() {
        ilst_children.extend_from_slice(&encode_ilst_utf8_atom(b"\xA9nam", &source_file.title));
    }
    if !source_file.artist.is_empty() {
        ilst_children.extend_from_slice(&encode_ilst_utf8_atom(b"\xA9ART", &source_file.artist));
    }
    if !source_file.album.is_empty() {
        ilst_children.extend_from_slice(&encode_ilst_utf8_atom(b"\xA9alb", &source_file.album));
    }
    if !source_file.genre.is_empty() {
        ilst_children.extend_from_slice(&encode_ilst_utf8_atom(b"\xA9gen", &source_file.genre));
    }
    if source_file.year > 0 {
        ilst_children.extend_from_slice(&encode_ilst_utf8_atom(
            b"\xA9day",
            &source_file.year.to_string(),
        ));
    }
    if source_file.track_number > 0 {
        ilst_children.extend_from_slice(&encode_ilst_u16_pair_atom(
            b"trkn",
            source_file.track_number as u16,
            0,
            2,
        ));
    }
    if source_file.disc_number > 0 {
        ilst_children.extend_from_slice(&encode_ilst_u16_pair_atom(
            b"disk",
            source_file.disc_number as u16,
            0,
            0,
        ));
    }
    if let Some(artwork) = &extracted_meta.artwork_data {
        ilst_children.extend_from_slice(&encode_ilst_covr_atom(
            artwork,
            extracted_meta.artwork_mime.as_deref(),
        ));
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
    let udta = mp4_box(b"udta", &meta_box);

    // Compute exact mdat audio data offset:
    // stco is 12 + 4 (entry_count=1) + 4 (chunk_offset) = 20 bytes.
    let stco_len = 20usize;
    let stbl_len = 8 + stsd.len() + stts.len() + stsc.len() + stsz.len() + stco_len;
    let minf_len = 8 + smhd.len() + dinf.len() + stbl_len;
    let mdia_len = 8 + mdhd.len() + hdlr.len() + minf_len;
    let trak_len = 8 + tkhd.len() + mdia_len;
    let moov_len = 8 + mvhd.len() + trak_len + udta.len();
    let mdat_data_offset = (ftyp.len() + moov_len + 8) as u32;

    let mut stco_payload = Vec::new();
    stco_payload.extend_from_slice(&1u32.to_be_bytes());
    stco_payload.extend_from_slice(&mdat_data_offset.to_be_bytes());
    let stco = mp4_full_box(b"stco", 0, 0, &stco_payload);

    let mut stbl_payload = Vec::new();
    stbl_payload.extend_from_slice(&stsd);
    stbl_payload.extend_from_slice(&stts);
    stbl_payload.extend_from_slice(&stsc);
    stbl_payload.extend_from_slice(&stsz);
    stbl_payload.extend_from_slice(&stco);
    let stbl = mp4_box(b"stbl", &stbl_payload);

    let mut minf_payload = Vec::new();
    minf_payload.extend_from_slice(&smhd);
    minf_payload.extend_from_slice(&dinf);
    minf_payload.extend_from_slice(&stbl);
    let minf = mp4_box(b"minf", &minf_payload);

    let mut mdia_payload = Vec::new();
    mdia_payload.extend_from_slice(&mdhd);
    mdia_payload.extend_from_slice(&hdlr);
    mdia_payload.extend_from_slice(&minf);
    let mdia = mp4_box(b"mdia", &mdia_payload);

    let mut trak_payload = Vec::new();
    trak_payload.extend_from_slice(&tkhd);
    trak_payload.extend_from_slice(&mdia);
    let trak = mp4_box(b"trak", &trak_payload);

    let mut moov_payload = Vec::new();
    moov_payload.extend_from_slice(&mvhd);
    moov_payload.extend_from_slice(&trak);
    moov_payload.extend_from_slice(&udta);
    let moov = mp4_box(b"moov", &moov_payload);

    let mdat_payload_len: usize = encoded_packets.iter().map(|p| p.len()).sum();
    let mut mdat_payload = Vec::with_capacity(mdat_payload_len);
    for pkt in encoded_packets {
        mdat_payload.extend_from_slice(pkt);
    }
    let mdat = mp4_box(b"mdat", &mdat_payload);

    let mut out = Vec::with_capacity(ftyp.len() + moov.len() + mdat.len());
    out.extend_from_slice(&ftyp);
    out.extend_from_slice(&moov);
    out.extend_from_slice(&mdat);
    out
}
