use std::fs;
use std::path::Path;

pub fn crc8_flac(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            if (crc & 0x80) != 0 {
                crc = (crc << 1) ^ 0x07;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

pub fn crc16_flac(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            if (crc & 0x8000) != 0 {
                crc = (crc << 1) ^ 0x8005;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

pub fn crc32_ogg(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in data {
        crc ^= (byte as u32) << 24;
        for _ in 0..8 {
            if crc & 0x8000_0000 != 0 {
                crc = (crc << 1) ^ 0x04C1_1DB7;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

pub fn build_vorbis_comment_payload(comments: &[(&str, &str)]) -> Vec<u8> {
    let vendor = "PodLoader Test Suite";
    let mut payload = Vec::new();
    payload.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    payload.extend_from_slice(vendor.as_bytes());
    payload.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for (k, v) in comments {
        let entry = format!("{}={}", k, v);
        payload.extend_from_slice(&(entry.len() as u32).to_le_bytes());
        payload.extend_from_slice(entry.as_bytes());
    }
    payload
}

pub fn build_flac_picture_payload(mime: &str, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&3u32.to_be_bytes()); // Cover (front)
    payload.extend_from_slice(&(mime.len() as u32).to_be_bytes());
    payload.extend_from_slice(mime.as_bytes());
    payload.extend_from_slice(&0u32.to_be_bytes()); // empty description
    payload.extend_from_slice(&100u32.to_be_bytes()); // width
    payload.extend_from_slice(&100u32.to_be_bytes()); // height
    payload.extend_from_slice(&24u32.to_be_bytes()); // color depth
    payload.extend_from_slice(&0u32.to_be_bytes()); // indexed color count
    payload.extend_from_slice(&(data.len() as u32).to_be_bytes());
    payload.extend_from_slice(data);
    payload
}

pub fn build_flac_streaminfo_bytes(
    sample_rate: u32,
    channels: u8,
    bits_per_sample: u8,
    total_samples: u64,
) -> [u8; 34] {
    let mut si = [0u8; 34];
    let block_size = 4096u16;
    si[0..2].copy_from_slice(&block_size.to_be_bytes());
    si[2..4].copy_from_slice(&block_size.to_be_bytes());
    let packed: u64 = ((sample_rate as u64 & 0xFFFFF) << 44)
        | ((((channels - 1) as u64) & 0x7) << 41)
        | ((((bits_per_sample - 1) as u64) & 0x1F) << 36)
        | (total_samples & 0xF_FFFF_FFFF);
    si[10..18].copy_from_slice(&packed.to_be_bytes());
    si
}

pub fn build_flac_audio_frame(
    frame_number: u8,
    sample_rate: u32,
    sample_val_left: i16,
    sample_val_right: i16,
) -> Vec<u8> {
    let sr_code: u8 = if sample_rate == 48_000 { 0xCA } else { 0xC9 };
    let mut frame = vec![0xFF, 0xF8, sr_code, 0x18, frame_number];
    let crc8 = crc8_flac(&frame);
    frame.push(crc8);

    // Subframe 0 (Left): Constant subframe (0x00) + 16-bit big-endian sample
    frame.push(0x00);
    frame.extend_from_slice(&sample_val_left.to_be_bytes());
    // Subframe 1 (Right): Constant subframe (0x00) + 16-bit big-endian sample
    frame.push(0x00);
    frame.extend_from_slice(&sample_val_right.to_be_bytes());

    let crc16 = crc16_flac(&frame);
    frame.extend_from_slice(&crc16.to_be_bytes());
    frame
}

pub fn build_valid_flac_fixture(
    title: &str,
    artist: &str,
    album: &str,
    track_number: u32,
    disc_number: u32,
    genre: &str,
    year: u32,
    sample_rate: u32,
    num_frames: u32,
    artwork: &[u8],
) -> Vec<u8> {
    let total_samples = (num_frames as u64) * 4096;
    let si = build_flac_streaminfo_bytes(sample_rate, 2, 16, total_samples);
    let vc = build_vorbis_comment_payload(&[
        ("TITLE", title),
        ("ARTIST", artist),
        ("ALBUM", album),
        ("TRACKNUMBER", &track_number.to_string()),
        ("DISCNUMBER", &disc_number.to_string()),
        ("GENRE", genre),
        ("DATE", &year.to_string()),
    ]);
    let pic = build_flac_picture_payload("image/png", artwork);

    let mut out = Vec::new();
    out.extend_from_slice(b"fLaC");

    // Block 0: STREAMINFO (not last, type 0, len 34)
    out.push(0x00);
    out.extend_from_slice(&[0x00, 0x00, 34]);
    out.extend_from_slice(&si);

    // Block 1: VORBIS_COMMENT (not last, type 4)
    out.push(0x04);
    let vc_len = vc.len() as u32;
    out.extend_from_slice(&vc_len.to_be_bytes()[1..4]);
    out.extend_from_slice(&vc);

    // Block 2: PICTURE (last block = 0x80 | 6 = 0x86)
    out.push(0x86);
    let pic_len = pic.len() as u32;
    out.extend_from_slice(&pic_len.to_be_bytes()[1..4]);
    out.extend_from_slice(&pic);

    for f in 0..num_frames {
        let sample = ((f as i16) * 200).wrapping_add(1024);
        out.extend_from_slice(&build_flac_audio_frame(
            f as u8,
            sample_rate,
            sample,
            -sample,
        ));
    }
    out
}

pub fn wrap_ogg_page(
    header_type: u8,
    granule_pos: u64,
    serial: u32,
    seq_no: u32,
    packet: &[u8],
) -> Vec<u8> {
    let mut segments = Vec::new();
    let mut rem = packet.len();
    while rem >= 255 {
        segments.push(255u8);
        rem -= 255;
    }
    segments.push(rem as u8);

    let mut page = Vec::new();
    page.extend_from_slice(b"OggS");
    page.push(0); // version
    page.push(header_type);
    page.extend_from_slice(&granule_pos.to_le_bytes());
    page.extend_from_slice(&serial.to_le_bytes());
    page.extend_from_slice(&seq_no.to_le_bytes());
    page.extend_from_slice(&0u32.to_le_bytes()); // CRC placeholder at 22..26
    page.push(segments.len() as u8);
    page.extend_from_slice(&segments);
    page.extend_from_slice(packet);

    let crc = crc32_ogg(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

pub fn build_valid_ogg_fixture(
    title: &str,
    artist: &str,
    album: &str,
    track_number: u32,
    disc_number: u32,
    genre: &str,
    year: u32,
    num_frames: u32,
    artwork: &[u8],
) -> Vec<u8> {
    let serial = 0x1234_5678u32;
    let total_samples = (num_frames as u64) * 4096;
    let si = build_flac_streaminfo_bytes(44_100, 2, 16, total_samples);

    let mut id_pkt = Vec::with_capacity(51);
    id_pkt.push(0x7F);
    id_pkt.extend_from_slice(b"FLAC");
    id_pkt.push(1);
    id_pkt.push(0);
    id_pkt.extend_from_slice(&2u16.to_be_bytes());
    id_pkt.extend_from_slice(b"fLaC");
    id_pkt.push(0x00);
    id_pkt.extend_from_slice(&[0x00, 0x00, 34]);
    id_pkt.extend_from_slice(&si);

    let vc = build_vorbis_comment_payload(&[
        ("TITLE", title),
        ("ARTIST", artist),
        ("ALBUM", album),
        ("TRACKNUMBER", &track_number.to_string()),
        ("DISCNUMBER", &disc_number.to_string()),
        ("GENRE", genre),
        ("DATE", &year.to_string()),
    ]);
    let mut vc_pkt = Vec::new();
    vc_pkt.push(0x04);
    vc_pkt.extend_from_slice(&(vc.len() as u32).to_be_bytes()[1..4]);
    vc_pkt.extend_from_slice(&vc);

    let pic = build_flac_picture_payload("image/png", artwork);
    let mut pic_pkt = Vec::new();
    pic_pkt.push(0x86);
    pic_pkt.extend_from_slice(&(pic.len() as u32).to_be_bytes()[1..4]);
    pic_pkt.extend_from_slice(&pic);

    let mut ogg = Vec::new();
    ogg.extend_from_slice(&wrap_ogg_page(0x02, 0, serial, 0, &id_pkt));
    ogg.extend_from_slice(&wrap_ogg_page(0x00, 0, serial, 1, &vc_pkt));
    ogg.extend_from_slice(&wrap_ogg_page(0x00, 0, serial, 2, &pic_pkt));

    for f in 0..num_frames {
        let is_last = f + 1 == num_frames;
        let hdr_type = if is_last { 0x04 } else { 0x00 };
        let granule = ((f + 1) as u64) * 4096;
        let frame = build_flac_audio_frame(f as u8, 44_100, 512, -512);
        ogg.extend_from_slice(&wrap_ogg_page(hdr_type, granule, serial, 3 + f, &frame));
    }
    ogg
}

pub fn build_id3v23_mp3_fixture(
    title: &str,
    artist: &str,
    album: &str,
    track_number: u32,
    genre: &str,
    year: u32,
    duration_seconds: u32,
) -> Vec<u8> {
    fn encode_text_frame(frame_id: &[u8; 4], text: &str) -> Vec<u8> {
        let payload_len = 1 + text.len() as u32;
        let mut frame = Vec::with_capacity(10 + payload_len as usize);
        frame.extend_from_slice(frame_id);
        frame.extend_from_slice(&payload_len.to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.push(3);
        frame.extend_from_slice(text.as_bytes());
        frame
    }

    let mut frames = Vec::new();
    frames.extend_from_slice(&encode_text_frame(b"TIT2", title));
    frames.extend_from_slice(&encode_text_frame(b"TPE1", artist));
    frames.extend_from_slice(&encode_text_frame(b"TALB", album));
    frames.extend_from_slice(&encode_text_frame(b"TRCK", &track_number.to_string()));
    frames.extend_from_slice(&encode_text_frame(b"TCON", genre));
    frames.extend_from_slice(&encode_text_frame(b"TYER", &year.to_string()));

    let size = frames.len() as u32;
    let synchsafe = [
        ((size >> 21) & 0x7F) as u8,
        ((size >> 14) & 0x7F) as u8,
        ((size >> 7) & 0x7F) as u8,
        (size & 0x7F) as u8,
    ];

    let mut mp3 = Vec::new();
    mp3.extend_from_slice(b"ID3");
    mp3.extend_from_slice(&[3, 0, 0]);
    mp3.extend_from_slice(&synchsafe);
    mp3.extend_from_slice(&frames);

    let audio_bytes = (duration_seconds as usize) * 16_000;
    let mut audio = vec![0u8; audio_bytes.max(4)];
    audio[0] = 0xFF;
    audio[1] = 0xFB;
    audio[2] = 0x90;
    audio[3] = 0x00;
    mp3.extend_from_slice(&audio);
    mp3
}

pub fn build_wav_fixture(sample_rate: u32, duration_seconds: u32) -> Vec<u8> {
    let channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * (channels as u32) * (bits_per_sample as u32 / 8);
    let block_align = channels * (bits_per_sample / 8);
    let data_len = byte_rate * duration_seconds;
    let riff_len = 36 + data_len;

    let mut wav = Vec::with_capacity((44 + data_len) as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_len.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize((44 + data_len) as usize, 0);
    wav
}

pub fn build_m4a_alac_fixture(
    title: &str,
    artist: &str,
    album: &str,
    track_number: u16,
    duration_seconds: u32,
) -> Vec<u8> {
    fn encode_ilst_text_atom(atom_id: &[u8; 4], text: &str) -> Vec<u8> {
        let data_len = 16 + text.len() as u32;
        let atom_len = 8 + data_len;
        let mut out = Vec::with_capacity(atom_len as usize);
        out.extend_from_slice(&atom_len.to_be_bytes());
        out.extend_from_slice(atom_id);
        out.extend_from_slice(&data_len.to_be_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(text.as_bytes());
        out
    }

    fn encode_trkn_atom(track_number: u16) -> Vec<u8> {
        let payload = [
            0u8,
            0,
            (track_number >> 8) as u8,
            (track_number & 0xFF) as u8,
            0,
            10,
            0,
            0,
        ];
        let data_len = 16 + payload.len() as u32;
        let atom_len = 8 + data_len;
        let mut out = Vec::with_capacity(atom_len as usize);
        out.extend_from_slice(&atom_len.to_be_bytes());
        out.extend_from_slice(b"trkn");
        out.extend_from_slice(&data_len.to_be_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&payload);
        out
    }

    let mut ilst_body = Vec::new();
    ilst_body.extend_from_slice(&encode_ilst_text_atom(b"\xa9nam", title));
    ilst_body.extend_from_slice(&encode_ilst_text_atom(b"\xa9ART", artist));
    ilst_body.extend_from_slice(&encode_ilst_text_atom(b"\xa9alb", album));
    ilst_body.extend_from_slice(&encode_trkn_atom(track_number));

    let ilst_len = (8 + ilst_body.len()) as u32;
    let mut ilst_atom = Vec::with_capacity(ilst_len as usize);
    ilst_atom.extend_from_slice(&ilst_len.to_be_bytes());
    ilst_atom.extend_from_slice(b"ilst");
    ilst_atom.extend_from_slice(&ilst_body);

    let meta_len = (12 + ilst_atom.len()) as u32;
    let mut meta_atom = Vec::with_capacity(meta_len as usize);
    meta_atom.extend_from_slice(&meta_len.to_be_bytes());
    meta_atom.extend_from_slice(b"meta");
    meta_atom.extend_from_slice(&[0, 0, 0, 0]);
    meta_atom.extend_from_slice(&ilst_atom);

    let udta_len = (8 + meta_atom.len()) as u32;
    let mut udta_atom = Vec::with_capacity(udta_len as usize);
    udta_atom.extend_from_slice(&udta_len.to_be_bytes());
    udta_atom.extend_from_slice(b"udta");
    udta_atom.extend_from_slice(&meta_atom);

    let alac_magic = [
        0x00, 0x00, 0x00, 0x24, b'a', b'l', b'a', b'c', 0, 0, 0, 0, 0, 0, 0x10, 0x00, 0, 16, 40,
        10, 14, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00, 0xAC, 0x44,
    ];

    let mut stsd_body = Vec::new();
    stsd_body.extend_from_slice(&[0, 0, 0, 0]);
    stsd_body.extend_from_slice(&1u32.to_be_bytes());
    let stsd_entry_len = 8 + 28 + alac_magic.len();
    stsd_body.extend_from_slice(&(stsd_entry_len as u32).to_be_bytes());
    stsd_body.extend_from_slice(b"alac");
    stsd_body.extend_from_slice(&[0u8; 6]);
    stsd_body.extend_from_slice(&1u16.to_be_bytes());
    stsd_body.extend_from_slice(&[0u8; 6]);
    stsd_body.extend_from_slice(&2u16.to_be_bytes());
    stsd_body.extend_from_slice(&16u16.to_be_bytes());
    stsd_body.extend_from_slice(&[0u8; 4]);
    stsd_body.extend_from_slice(&44100u16.to_be_bytes());
    stsd_body.extend_from_slice(&0u16.to_be_bytes());
    stsd_body.extend_from_slice(&alac_magic);

    let stsd_len = (8 + stsd_body.len()) as u32;
    let mut stsd_atom = Vec::with_capacity(stsd_len as usize);
    stsd_atom.extend_from_slice(&stsd_len.to_be_bytes());
    stsd_atom.extend_from_slice(b"stsd");
    stsd_atom.extend_from_slice(&stsd_body);

    let stbl_len = (8 + stsd_atom.len()) as u32;
    let mut stbl_atom = Vec::with_capacity(stbl_len as usize);
    stbl_atom.extend_from_slice(&stbl_len.to_be_bytes());
    stbl_atom.extend_from_slice(b"stbl");
    stbl_atom.extend_from_slice(&stsd_atom);

    let minf_len = (8 + stbl_atom.len()) as u32;
    let mut minf_atom = Vec::with_capacity(minf_len as usize);
    minf_atom.extend_from_slice(&minf_len.to_be_bytes());
    minf_atom.extend_from_slice(b"minf");
    minf_atom.extend_from_slice(&stbl_atom);

    let mut mdhd_payload = vec![0u8; 24];
    mdhd_payload[12..16].copy_from_slice(&44100u32.to_be_bytes());
    let total_samples = duration_seconds * 44100;
    mdhd_payload[16..20].copy_from_slice(&total_samples.to_be_bytes());
    let mdhd_len = (8 + mdhd_payload.len()) as u32;
    let mut mdhd_atom = Vec::with_capacity(mdhd_len as usize);
    mdhd_atom.extend_from_slice(&mdhd_len.to_be_bytes());
    mdhd_atom.extend_from_slice(b"mdhd");
    mdhd_atom.extend_from_slice(&mdhd_payload);

    let mdia_len = (8 + mdhd_atom.len() + minf_atom.len()) as u32;
    let mut mdia_atom = Vec::with_capacity(mdia_len as usize);
    mdia_atom.extend_from_slice(&mdia_len.to_be_bytes());
    mdia_atom.extend_from_slice(b"mdia");
    mdia_atom.extend_from_slice(&mdhd_atom);
    mdia_atom.extend_from_slice(&minf_atom);

    let trak_len = (8 + mdia_atom.len()) as u32;
    let mut trak_atom = Vec::with_capacity(trak_len as usize);
    trak_atom.extend_from_slice(&trak_len.to_be_bytes());
    trak_atom.extend_from_slice(b"trak");
    trak_atom.extend_from_slice(&mdia_atom);

    let moov_len = (8 + trak_atom.len() + udta_atom.len()) as u32;
    let mut moov_atom = Vec::with_capacity(moov_len as usize);
    moov_atom.extend_from_slice(&moov_len.to_be_bytes());
    moov_atom.extend_from_slice(b"moov");
    moov_atom.extend_from_slice(&trak_atom);
    moov_atom.extend_from_slice(&udta_atom);

    let ftyp_payload = b"M4A \x00\x00\x02\x00M4A mp42isom\x00\x00\x00\x00";
    let ftyp_len = (8 + ftyp_payload.len()) as u32;
    let mut ftyp_atom = Vec::with_capacity(ftyp_len as usize);
    ftyp_atom.extend_from_slice(&ftyp_len.to_be_bytes());
    ftyp_atom.extend_from_slice(b"ftyp");
    ftyp_atom.extend_from_slice(ftyp_payload);

    let mdat_payload = vec![0u8; 1024];
    let mdat_len = (8 + mdat_payload.len()) as u32;
    let mut mdat_atom = Vec::with_capacity(mdat_len as usize);
    mdat_atom.extend_from_slice(&mdat_len.to_be_bytes());
    mdat_atom.extend_from_slice(b"mdat");
    mdat_atom.extend_from_slice(&mdat_payload);

    let mut out = Vec::new();
    out.extend_from_slice(&ftyp_atom);
    out.extend_from_slice(&moov_atom);
    out.extend_from_slice(&mdat_atom);
    out
}

pub fn generate_regtest_sample_audio(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;

    let mp3_bytes = build_id3v23_mp3_fixture(
        "Open Sailor MP3",
        "Sea Shanty Folk",
        "Maritime Anthems",
        1,
        "Shanty",
        2024,
        3,
    );
    fs::write(dir.join("01 - Open Sailor.mp3"), mp3_bytes)?;

    let wav_bytes = build_wav_fixture(44_100, 3);
    fs::write(dir.join("02 - Harbor Shanty.wav"), wav_bytes)?;

    let artwork = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xAA, 0xBB, 0xCC, 0xDD];
    let flac_bytes = build_valid_flac_fixture(
        "Open Waters FLAC",
        "Sea Shanty Folk",
        "Maritime Anthems",
        3,
        1,
        "Acoustic",
        2024,
        44_100,
        10,
        &artwork,
    );
    fs::write(dir.join("03 - Open Waters.flac"), flac_bytes)?;

    let alac_bytes = build_m4a_alac_fixture(
        "Island Breeze ALAC",
        "Sea Shanty Folk",
        "Maritime Anthems",
        4,
        3,
    );
    fs::write(dir.join("04 - Island Breeze.m4a"), alac_bytes)?;

    let ogg_bytes = build_valid_ogg_fixture(
        "Trade Winds OGG",
        "Sea Shanty Folk",
        "Maritime Anthems",
        5,
        1,
        "Folk",
        2024,
        10,
        &artwork,
    );
    fs::write(dir.join("05 - Trade Winds.ogg"), ogg_bytes)?;

    Ok(())
}
