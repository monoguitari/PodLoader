use crate::{DeviceTrack, PodLoaderError};

#[derive(Debug, Clone)]
pub struct PlayCountEntry {
    pub play_count_delta: u32,
    pub last_played_timestamp: u32,
    pub bookmark_ms: u32,
    pub rating: Option<u8>,
}

pub fn parse_play_counts(bytes: &[u8]) -> Result<Vec<PlayCountEntry>, PodLoaderError> {
    if bytes.get(0..4) != Some(b"mhdp") {
        return Err(PodLoaderError::InvalidPlayCounts(
            "Missing mhdp magic".into(),
        ));
    }
    let header_len = read_u32_le(bytes, 4)? as usize;
    let entry_len = read_u32_le(bytes, 8)? as usize;
    let num_entries = read_u32_le(bytes, 12)? as usize;

    if num_entries == 0 {
        return Ok(Vec::new());
    }

    if entry_len < 12 {
        return Err(PodLoaderError::InvalidPlayCounts(format!(
            "Entry length too small: {}",
            entry_len
        )));
    }

    let mut entries = Vec::with_capacity(num_entries);
    for idx in 0..num_entries {
        let offset = header_len + idx * entry_len;
        let entry_slice = bytes.get(offset..offset + entry_len).ok_or_else(|| {
            PodLoaderError::InvalidPlayCounts("Truncated Play Counts entry".into())
        })?;
        let play_count_delta = read_u32_le(entry_slice, 0)?;
        let last_played_timestamp = read_u32_le(entry_slice, 4)?;
        let bookmark_ms = read_u32_le(entry_slice, 8)?;
        let rating = if entry_len >= 16 {
            let raw_rating = read_u32_le(entry_slice, 12)?;
            if raw_rating > 0 {
                Some(((raw_rating / 20) as u8).min(5))
            } else {
                None
            }
        } else {
            None
        };

        entries.push(PlayCountEntry {
            play_count_delta,
            last_played_timestamp,
            bookmark_ms,
            rating,
        });
    }

    Ok(entries)
}

pub fn merge_play_counts_into_tracks(tracks: &mut [DeviceTrack], entries: &[PlayCountEntry]) {
    for (track, entry) in tracks.iter_mut().zip(entries.iter()) {
        if entry.play_count_delta > 0 {
            track.play_count = track.play_count.saturating_add(entry.play_count_delta);
        }
        if entry.last_played_timestamp > 0 {
            track.last_played_timestamp = Some(entry.last_played_timestamp);
        }
        if entry.bookmark_ms > 0 {
            track.bookmark_ms = entry.bookmark_ms;
        }
        if let Some(new_rating) = entry.rating {
            track.rating = new_rating;
        }
    }
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, PodLoaderError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| PodLoaderError::InvalidPlayCounts("Unexpected EOF reading u32".into()))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}
