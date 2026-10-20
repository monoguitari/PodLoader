use crate::playcounts::PlayCountEntry;
use crate::{DeviceLibrary, DevicePlaylist, DeviceTrack, PlaylistKind, PodLoaderError};

#[derive(Debug, Clone)]
struct MhodChunk {
    mhod_type: u32,
    raw_bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct MhitChunk {
    header_bytes: Vec<u8>,
    mhods: Vec<MhodChunk>,
    track: DeviceTrack,
}

impl MhitChunk {
    fn to_bytes(&self) -> Vec<u8> {
        let mut header = self.header_bytes.clone();
        let header_len = header.len() as u32;
        let body_len: usize = self.mhods.iter().map(|m| m.raw_bytes.len()).sum();
        let total_len = header_len + body_len as u32;
        header[4..8].copy_from_slice(&header_len.to_le_bytes());
        header[8..12].copy_from_slice(&total_len.to_le_bytes());
        header[12..16].copy_from_slice(&(self.mhods.len() as u32).to_le_bytes());
        for mhod in &self.mhods {
            header.extend_from_slice(&mhod.raw_bytes);
        }
        header
    }
}

#[derive(Debug, Clone)]
struct MhipChunk {
    header_bytes: Vec<u8>,
    mhods: Vec<MhodChunk>,
    track_id: u32,
    order_position: Option<u32>,
    is_podcast_group: bool,
}

impl MhipChunk {
    fn to_bytes(&self) -> Vec<u8> {
        let mut header = self.header_bytes.clone();
        let header_len = header.len() as u32;
        let mhods_len: usize = self.mhods.iter().map(|m| m.raw_bytes.len()).sum();
        let total_len = header_len + mhods_len as u32;
        header[4..8].copy_from_slice(&header_len.to_le_bytes());
        header[8..12].copy_from_slice(&total_len.to_le_bytes());
        header[12..16].copy_from_slice(&(self.mhods.len() as u32).to_le_bytes());
        header[24..28].copy_from_slice(&self.track_id.to_le_bytes());
        for mhod in &self.mhods {
            header.extend_from_slice(&mhod.raw_bytes);
        }
        header
    }
}

#[derive(Debug, Clone)]
struct MhypChunk {
    header_bytes: Vec<u8>,
    mhods: Vec<MhodChunk>,
    mhips: Vec<MhipChunk>,
    playlist: DevicePlaylist,
}

impl MhypChunk {
    fn to_bytes(&self) -> Vec<u8> {
        let mut header = self.header_bytes.clone();
        let header_len = header.len() as u32;
        let mhod_bytes: Vec<Vec<u8>> = self.mhods.iter().map(|m| m.raw_bytes.clone()).collect();
        let mhip_bytes: Vec<Vec<u8>> = self.mhips.iter().map(|m| m.to_bytes()).collect();
        let body_len: usize = mhod_bytes.iter().map(|b| b.len()).sum::<usize>()
            + mhip_bytes.iter().map(|b| b.len()).sum::<usize>();
        let total_len = header_len + body_len as u32;

        header[4..8].copy_from_slice(&header_len.to_le_bytes());
        header[8..12].copy_from_slice(&total_len.to_le_bytes());
        header[12..16].copy_from_slice(&(self.mhods.len() as u32).to_le_bytes());
        header[16..20].copy_from_slice(&(self.mhips.len() as u32).to_le_bytes());

        for b in mhod_bytes {
            header.extend_from_slice(&b);
        }
        for b in mhip_bytes {
            header.extend_from_slice(&b);
        }
        header
    }
}

#[derive(Debug, Clone)]
enum MhsdBody {
    TrackList {
        mhlt_header_bytes: Vec<u8>,
        tracks: Vec<MhitChunk>,
    },
    PlaylistList {
        mhlp_header_bytes: Vec<u8>,
        playlists: Vec<MhypChunk>,
    },
    Raw(Vec<u8>),
}

#[derive(Debug, Clone)]
struct MhsdChunk {
    header_bytes: Vec<u8>,
    dataset_type: u32,
    body: MhsdBody,
}

impl MhsdChunk {
    fn to_bytes(&self) -> Vec<u8> {
        let mut header = self.header_bytes.clone();
        let header_len = header.len() as u32;
        let body_bytes = match &self.body {
            MhsdBody::TrackList {
                mhlt_header_bytes,
                tracks,
            } => {
                let mut mhlt = mhlt_header_bytes.clone();
                let mhlt_header_len = mhlt.len() as u32;
                mhlt[4..8].copy_from_slice(&mhlt_header_len.to_le_bytes());
                mhlt[8..12].copy_from_slice(&(tracks.len() as u32).to_le_bytes());
                for t in tracks {
                    mhlt.extend_from_slice(&t.to_bytes());
                }
                mhlt
            }
            MhsdBody::PlaylistList {
                mhlp_header_bytes,
                playlists,
            } => {
                let mut mhlp = mhlp_header_bytes.clone();
                let mhlp_header_len = mhlp.len() as u32;
                mhlp[4..8].copy_from_slice(&mhlp_header_len.to_le_bytes());
                mhlp[8..12].copy_from_slice(&(playlists.len() as u32).to_le_bytes());
                for pl in playlists {
                    mhlp.extend_from_slice(&pl.to_bytes());
                }
                mhlp
            }
            MhsdBody::Raw(raw) => raw.clone(),
        };

        let total_len = header_len + body_bytes.len() as u32;
        header[4..8].copy_from_slice(&header_len.to_le_bytes());
        header[8..12].copy_from_slice(&total_len.to_le_bytes());
        header[12..16].copy_from_slice(&self.dataset_type.to_le_bytes());
        header.extend_from_slice(&body_bytes);
        header
    }
}

#[derive(Debug, Clone)]
pub struct ItunesDb {
    mhbd_header_bytes: Vec<u8>,
    datasets: Vec<MhsdChunk>,
}

impl ItunesDb {
    pub fn parse(bytes: &[u8]) -> Result<Self, PodLoaderError> {
        if bytes.get(0..4) != Some(b"mhbd") {
            return Err(PodLoaderError::InvalidItunesDb(
                "Missing mhbd magic".into(),
            ));
        }
        let header_len = read_u32_le(bytes, 4)? as usize;
        let num_datasets = read_u32_le(bytes, 20)? as usize;
        let mhbd_header_bytes = bytes
            .get(0..header_len)
            .ok_or_else(|| PodLoaderError::InvalidItunesDb("Truncated mhbd header".into()))?
            .to_vec();

        let mut offset = header_len;
        let mut datasets = Vec::with_capacity(num_datasets);

        for _ in 0..num_datasets {
            if bytes.get(offset..offset + 4) != Some(b"mhsd") {
                return Err(PodLoaderError::InvalidItunesDb(
                    "Expected mhsd header".into(),
                ));
            }
            let mhsd_header_len = read_u32_le(bytes, offset + 4)? as usize;
            let mhsd_total_len = read_u32_le(bytes, offset + 8)? as usize;
            let dataset_type = read_u32_le(bytes, offset + 12)?;

            let header_bytes = bytes
                .get(offset..offset + mhsd_header_len)
                .ok_or_else(|| PodLoaderError::InvalidItunesDb("Truncated mhsd header".into()))?
                .to_vec();
            let body_slice = bytes
                .get(offset + mhsd_header_len..offset + mhsd_total_len)
                .ok_or_else(|| PodLoaderError::InvalidItunesDb("Truncated mhsd body".into()))?;

            let body = match dataset_type {
                1 => parse_track_list_body(body_slice)?,
                2 => parse_playlist_list_body(body_slice)?,
                _ => MhsdBody::Raw(body_slice.to_vec()),
            };

            datasets.push(MhsdChunk {
                header_bytes,
                dataset_type,
                body,
            });

            offset += mhsd_total_len;
        }

        Ok(Self {
            mhbd_header_bytes,
            datasets,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut mhbd = self.mhbd_header_bytes.clone();
        let header_len = mhbd.len() as u32;
        let dataset_bytes: Vec<Vec<u8>> = self.datasets.iter().map(|d| d.to_bytes()).collect();
        let body_len: usize = dataset_bytes.iter().map(|b| b.len()).sum();
        let total_len = header_len + body_len as u32;

        mhbd[4..8].copy_from_slice(&header_len.to_le_bytes());
        mhbd[8..12].copy_from_slice(&total_len.to_le_bytes());
        mhbd[20..24].copy_from_slice(&(self.datasets.len() as u32).to_le_bytes());

        for b in dataset_bytes {
            mhbd.extend_from_slice(&b);
        }
        mhbd
    }

    pub fn to_device_library(&self) -> DeviceLibrary {
        let mut tracks = Vec::new();
        let mut playlists = Vec::new();

        for ds in &self.datasets {
            match &ds.body {
                MhsdBody::TrackList { tracks: t, .. } => {
                    for chunk in t {
                        tracks.push(chunk.track.clone());
                    }
                }
                MhsdBody::PlaylistList { playlists: p, .. } => {
                    for chunk in p {
                        playlists.push(chunk.playlist.clone());
                    }
                }
                MhsdBody::Raw(_) => {}
            }
        }

        DeviceLibrary { tracks, playlists }
    }

    pub fn merge_play_counts(&mut self, entries: &[PlayCountEntry]) {
        for ds in &mut self.datasets {
            if let MhsdBody::TrackList { tracks, .. } = &mut ds.body {
                for (chunk, entry) in tracks.iter_mut().zip(entries.iter()) {
                    if entry.play_count_delta > 0 {
                        chunk.track.play_count = chunk
                            .track
                            .play_count
                            .saturating_add(entry.play_count_delta);
                        if chunk.header_bytes.len() >= 84 {
                            chunk.header_bytes[80..84]
                                .copy_from_slice(&chunk.track.play_count.to_le_bytes());
                        }
                    }
                    if entry.last_played_timestamp > 0 {
                        chunk.track.last_played_timestamp = Some(entry.last_played_timestamp);
                        if chunk.header_bytes.len() >= 92 {
                            chunk.header_bytes[88..92]
                                .copy_from_slice(&entry.last_played_timestamp.to_le_bytes());
                        }
                    }
                    if entry.bookmark_ms > 0 {
                        chunk.track.bookmark_ms = entry.bookmark_ms;
                        if chunk.header_bytes.len() >= 152 {
                            chunk.header_bytes[148..152]
                                .copy_from_slice(&entry.bookmark_ms.to_le_bytes());
                        }
                    }
                    if let Some(new_rating) = entry.rating {
                        chunk.track.rating = new_rating;
                        if chunk.header_bytes.len() > 30 {
                            chunk.header_bytes[30] = new_rating.saturating_mul(20);
                        }
                    }
                }
            }
        }
    }
}

pub fn encode_string_mhod(mhod_type: u32, text: &str) -> Vec<u8> {
    let utf16_bytes: Vec<u8> = text
        .encode_utf16()
        .flat_map(|ch| ch.to_le_bytes())
        .collect();
    let header_len: u32 = 24;
    let total_len: u32 = 40 + utf16_bytes.len() as u32;
    let mut buf = vec![0u8; total_len as usize];
    buf[0..4].copy_from_slice(b"mhod");
    buf[4..8].copy_from_slice(&header_len.to_le_bytes());
    buf[8..12].copy_from_slice(&total_len.to_le_bytes());
    buf[12..16].copy_from_slice(&mhod_type.to_le_bytes());
    buf[24..28].copy_from_slice(&1u32.to_le_bytes());
    buf[28..32].copy_from_slice(&(utf16_bytes.len() as u32).to_le_bytes());
    buf[32..36].copy_from_slice(&1u32.to_le_bytes());
    buf[40..].copy_from_slice(&utf16_bytes);
    buf
}

pub fn build_empty_itunesdb(master_playlist_name: &str) -> Vec<u8> {
    let mut mhlt = vec![0u8; 92];
    mhlt[0..4].copy_from_slice(b"mhlt");
    mhlt[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlt[8..12].copy_from_slice(&0u32.to_le_bytes());

    let mut mhsd_tracks = vec![0u8; 96];
    mhsd_tracks[0..4].copy_from_slice(b"mhsd");
    mhsd_tracks[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_tracks[8..12].copy_from_slice(&((96 + mhlt.len()) as u32).to_le_bytes());
    mhsd_tracks[12..16].copy_from_slice(&1u32.to_le_bytes());
    mhsd_tracks.extend_from_slice(&mhlt);

    let title_mhod = encode_string_mhod(1, master_playlist_name);
    let mhyp_header_len = 108u32;
    let mhyp_total_len = mhyp_header_len + title_mhod.len() as u32;
    let mut mhyp = vec![0u8; mhyp_header_len as usize];
    mhyp[0..4].copy_from_slice(b"mhyp");
    mhyp[4..8].copy_from_slice(&mhyp_header_len.to_le_bytes());
    mhyp[8..12].copy_from_slice(&mhyp_total_len.to_le_bytes());
    mhyp[12..16].copy_from_slice(&1u32.to_le_bytes());
    mhyp[16..20].copy_from_slice(&0u32.to_le_bytes());
    mhyp[20] = 1;
    mhyp[28..36].copy_from_slice(&1u64.to_le_bytes());
    mhyp.extend_from_slice(&title_mhod);

    let mut mhlp = vec![0u8; 92];
    mhlp[0..4].copy_from_slice(b"mhlp");
    mhlp[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlp[8..12].copy_from_slice(&1u32.to_le_bytes());
    mhlp.extend_from_slice(&mhyp);

    let mut mhsd_playlists = vec![0u8; 96];
    mhsd_playlists[0..4].copy_from_slice(b"mhsd");
    mhsd_playlists[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_playlists[8..12].copy_from_slice(&((96 + mhlp.len()) as u32).to_le_bytes());
    mhsd_playlists[12..16].copy_from_slice(&2u32.to_le_bytes());
    mhsd_playlists.extend_from_slice(&mhlp);

    let mhbd_header_len = 104u32;
    let total_len = mhbd_header_len + (mhsd_tracks.len() + mhsd_playlists.len()) as u32;
    let mut mhbd = vec![0u8; mhbd_header_len as usize];
    mhbd[0..4].copy_from_slice(b"mhbd");
    mhbd[4..8].copy_from_slice(&mhbd_header_len.to_le_bytes());
    mhbd[8..12].copy_from_slice(&total_len.to_le_bytes());
    mhbd[12..16].copy_from_slice(&1u32.to_le_bytes());
    mhbd[16..20].copy_from_slice(&0x19u32.to_le_bytes());
    mhbd[20..24].copy_from_slice(&2u32.to_le_bytes());
    mhbd.extend_from_slice(&mhsd_tracks);
    mhbd.extend_from_slice(&mhsd_playlists);
    mhbd
}

fn parse_mhod_chunk(bytes: &[u8], offset: usize) -> Result<(MhodChunk, usize), PodLoaderError> {
    if bytes.get(offset..offset + 4) != Some(b"mhod") {
        return Err(PodLoaderError::InvalidItunesDb(
            "Expected mhod header".into(),
        ));
    }
    let total_len = read_u32_le(bytes, offset + 8)? as usize;
    let mhod_type = read_u32_le(bytes, offset + 12)?;
    let raw_bytes = bytes
        .get(offset..offset + total_len)
        .ok_or_else(|| PodLoaderError::InvalidItunesDb("Truncated mhod chunk".into()))?
        .to_vec();
    Ok((
        MhodChunk {
            mhod_type,
            raw_bytes,
        },
        total_len,
    ))
}

fn parse_track_list_body(body: &[u8]) -> Result<MhsdBody, PodLoaderError> {
    if body.get(0..4) != Some(b"mhlt") {
        return Err(PodLoaderError::InvalidItunesDb(
            "Expected mhlt header".into(),
        ));
    }
    let mhlt_header_len = read_u32_le(body, 4)? as usize;
    let num_tracks = read_u32_le(body, 8)? as usize;
    let mhlt_header_bytes = body
        .get(0..mhlt_header_len)
        .ok_or_else(|| PodLoaderError::InvalidItunesDb("Truncated mhlt header".into()))?
        .to_vec();

    let mut offset = mhlt_header_len;
    let mut tracks = Vec::with_capacity(num_tracks);

    for _ in 0..num_tracks {
        if body.get(offset..offset + 4) != Some(b"mhit") {
            return Err(PodLoaderError::InvalidItunesDb(
                "Expected mhit header".into(),
            ));
        }
        let mhit_header_len = read_u32_le(body, offset + 4)? as usize;
        let mhit_total_len = read_u32_le(body, offset + 8)? as usize;
        let num_mhods = read_u32_le(body, offset + 12)? as usize;

        let header_bytes = body
            .get(offset..offset + mhit_header_len)
            .ok_or_else(|| PodLoaderError::InvalidItunesDb("Truncated mhit header".into()))?
            .to_vec();

        let id = read_u32_le(&header_bytes, 16)?;
        let raw_rating = header_bytes.get(30).copied().unwrap_or(0);
        let rating = (raw_rating / 20).min(5);
        let duration_ms = read_u32_le(&header_bytes, 40).unwrap_or(0);
        let track_number = read_u32_le(&header_bytes, 44).unwrap_or(0);
        let year = read_u32_le(&header_bytes, 52).unwrap_or(0);
        let bitrate_kbps = read_u32_le(&header_bytes, 56).unwrap_or(0);
        let play_count = read_u32_le(&header_bytes, 80).unwrap_or(0);
        let raw_last_played = read_u32_le(&header_bytes, 88).unwrap_or(0);
        let last_played_timestamp = if raw_last_played == 0 {
            None
        } else {
            Some(raw_last_played)
        };
        let disc_number = read_u32_le(&header_bytes, 92).unwrap_or(0);
        let bookmark_ms = read_u32_le(&header_bytes, 148).unwrap_or(0);

        let mut child_offset = offset + mhit_header_len;
        let mut mhods = Vec::with_capacity(num_mhods);
        let mut title = String::new();
        let mut artist = String::new();
        let mut album = String::new();
        let mut genre = String::new();
        let mut obfuscated_path = String::new();

        for _ in 0..num_mhods {
            let (mhod, len) = parse_mhod_chunk(body, child_offset)?;
            match mhod.mhod_type {
                1 => {
                    if let Some(s) = decode_mhod_string(&mhod.raw_bytes) {
                        title = s;
                    }
                }
                2 => {
                    if let Some(s) = decode_mhod_string(&mhod.raw_bytes) {
                        obfuscated_path = s;
                    }
                }
                3 => {
                    if let Some(s) = decode_mhod_string(&mhod.raw_bytes) {
                        album = s;
                    }
                }
                4 => {
                    if let Some(s) = decode_mhod_string(&mhod.raw_bytes) {
                        artist = s;
                    }
                }
                5 => {
                    if let Some(s) = decode_mhod_string(&mhod.raw_bytes) {
                        genre = s;
                    }
                }
                _ => {}
            }
            mhods.push(mhod);
            child_offset += len;
        }

        tracks.push(MhitChunk {
            header_bytes,
            mhods,
            track: DeviceTrack {
                id,
                title,
                artist,
                album,
                track_number,
                disc_number,
                genre,
                year,
                duration_ms,
                bitrate_kbps,
                play_count,
                rating,
                last_played_timestamp,
                bookmark_ms,
                obfuscated_path,
            },
        });

        offset += mhit_total_len;
    }

    Ok(MhsdBody::TrackList {
        mhlt_header_bytes,
        tracks,
    })
}

fn parse_playlist_list_body(body: &[u8]) -> Result<MhsdBody, PodLoaderError> {
    if body.get(0..4) != Some(b"mhlp") {
        return Err(PodLoaderError::InvalidItunesDb(
            "Expected mhlp header".into(),
        ));
    }
    let mhlp_header_len = read_u32_le(body, 4)? as usize;
    let num_playlists = read_u32_le(body, 8)? as usize;
    let mhlp_header_bytes = body
        .get(0..mhlp_header_len)
        .ok_or_else(|| PodLoaderError::InvalidItunesDb("Truncated mhlp header".into()))?
        .to_vec();

    let mut pl_offset = mhlp_header_len;
    let mut playlists = Vec::with_capacity(num_playlists);

    for _ in 0..num_playlists {
        if body.get(pl_offset..pl_offset + 4) != Some(b"mhyp") {
            return Err(PodLoaderError::InvalidItunesDb(
                "Expected mhyp header".into(),
            ));
        }
        let mhyp_header_len = read_u32_le(body, pl_offset + 4)? as usize;
        let mhyp_total_len = read_u32_le(body, pl_offset + 8)? as usize;
        let num_mhods = read_u32_le(body, pl_offset + 12)? as usize;
        let num_mhips = read_u32_le(body, pl_offset + 16)? as usize;
        let is_master = body.get(pl_offset + 20).copied().unwrap_or(0) == 1;
        let playlist_id = read_u64_le(body, pl_offset + 28)?;

        let header_bytes = body
            .get(pl_offset..pl_offset + mhyp_header_len)
            .ok_or_else(|| PodLoaderError::InvalidItunesDb("Truncated mhyp header".into()))?
            .to_vec();

        let mut child_offset = pl_offset + mhyp_header_len;
        let mut mhods = Vec::with_capacity(num_mhods);
        let mut name = String::new();
        let mut is_smart = false;

        for _ in 0..num_mhods {
            let (mhod, len) = parse_mhod_chunk(body, child_offset)?;
            if mhod.mhod_type == 1 {
                if let Some(s) = decode_mhod_string(&mhod.raw_bytes) {
                    name = s;
                }
            } else if mhod.mhod_type == 50 || mhod.mhod_type == 51 {
                is_smart = true;
            }
            mhods.push(mhod);
            child_offset += len;
        }

        let mut mhips = Vec::with_capacity(num_mhips);

        for _ in 0..num_mhips {
            if body.get(child_offset..child_offset + 4) != Some(b"mhip") {
                return Err(PodLoaderError::InvalidItunesDb(
                    "Expected mhip header".into(),
                ));
            }
            let mhip_header_len = read_u32_le(body, child_offset + 4)? as usize;
            let mhip_num_mhods = read_u32_le(body, child_offset + 12)? as usize;
            let podcast_group_flag = read_u32_le(body, child_offset + 16).unwrap_or(0);
            let track_id = read_u32_le(body, child_offset + 24)?;

            let mhip_header_bytes = body
                .get(child_offset..child_offset + mhip_header_len)
                .ok_or_else(|| PodLoaderError::InvalidItunesDb("Truncated mhip header".into()))?
                .to_vec();

            let mut mhip_child_offset = child_offset + mhip_header_len;
            let mut mhip_mhods = Vec::with_capacity(mhip_num_mhods);
            let mut order_position = None;

            for _ in 0..mhip_num_mhods {
                let (mhod, len) = parse_mhod_chunk(body, mhip_child_offset)?;
                if mhod.mhod_type == 100 && mhod.raw_bytes.len() >= 28 {
                    order_position = read_u32_le(&mhod.raw_bytes, 24).ok();
                }
                mhip_mhods.push(mhod);
                mhip_child_offset += len;
            }

            mhips.push(MhipChunk {
                header_bytes: mhip_header_bytes,
                mhods: mhip_mhods,
                track_id,
                order_position,
                is_podcast_group: podcast_group_flag != 0,
            });
            child_offset = mhip_child_offset;
        }

        let mut ordered_items: Vec<(usize, &MhipChunk)> = mhips
            .iter()
            .enumerate()
            .filter(|(_, item)| !item.is_podcast_group)
            .collect();
        ordered_items.sort_by_key(|(idx, item)| (item.order_position.unwrap_or(u32::MAX), *idx));
        let track_ids: Vec<u32> = ordered_items
            .into_iter()
            .map(|(_, item)| item.track_id)
            .collect();

        let kind = if is_master {
            PlaylistKind::Master
        } else if is_smart {
            PlaylistKind::Smart
        } else {
            PlaylistKind::User
        };

        playlists.push(MhypChunk {
            header_bytes,
            mhods,
            mhips,
            playlist: DevicePlaylist {
                id: playlist_id,
                name,
                kind,
                track_ids,
            },
        });

        pl_offset += mhyp_total_len;
    }

    Ok(MhsdBody::PlaylistList {
        mhlp_header_bytes,
        playlists,
    })
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, PodLoaderError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| PodLoaderError::InvalidItunesDb("Unexpected EOF reading u32".into()))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64_le(bytes: &[u8], offset: usize) -> Result<u64, PodLoaderError> {
    let slice = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| PodLoaderError::InvalidItunesDb("Unexpected EOF reading u64".into()))?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn decode_mhod_string(mhod_bytes: &[u8]) -> Option<String> {
    if mhod_bytes.len() < 40 {
        return None;
    }
    let str_len = u32::from_le_bytes([
        mhod_bytes[28],
        mhod_bytes[29],
        mhod_bytes[30],
        mhod_bytes[31],
    ]) as usize;
    let encoding = u32::from_le_bytes([
        mhod_bytes[32],
        mhod_bytes[33],
        mhod_bytes[34],
        mhod_bytes[35],
    ]);
    let payload = mhod_bytes.get(40..40 + str_len)?;
    if encoding == 2 {
        String::from_utf8(payload.to_vec()).ok()
    } else {
        let u16s: Vec<u16> = payload
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&u16s).ok()
    }
}
