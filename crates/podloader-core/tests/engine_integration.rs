use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use podloader_core::{
    verify_itunesdb_signatures, CommitOutcome, DeviceConnectionState, DuplicateResolution,
    HardwareDeviceAdapter, InterruptedResolution, MountedVolume, PlaylistKind, PodLoaderEngine,
    PodLoaderError, TranscodePreset, TransferProgress,
};

struct FakeHardwareAdapter {
    volumes: Vec<MountedVolume>,
}

impl HardwareDeviceAdapter for FakeHardwareAdapter {
    fn list_mounted_volumes(&self) -> Result<Vec<MountedVolume>, PodLoaderError> {
        Ok(self.volumes.clone())
    }

    fn lookup_usb_firewire_guid(&self, mount_path: &std::path::Path) -> Option<String> {
        self.volumes
            .iter()
            .find(|v| v.mount_path == mount_path)
            .and_then(|v| v.usb_firewire_guid.clone())
    }
}

#[test]
fn detects_uninitialized_volume_and_initializes_device_hierarchy_and_empty_itunesdb() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let mount_path: PathBuf = temp_dir.path().to_path_buf();

    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD_FAT32".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 29_950_000_000,
            usb_firewire_guid: Some("000A27001A2B3C4D".to_string()),
        }],
    };

    let engine = PodLoaderEngine::new(adapter);

    // 1. Freshly formatted volume is detected as UninitializedVolume
    let initial_state = engine.scan_device().expect("scan uninitialized volume");
    match &initial_state {
        DeviceConnectionState::UninitializedVolume(vol) => {
            assert_eq!(vol.mount_path, mount_path);
            assert_eq!(vol.volume_name, "IPOD_FAT32");
            assert_eq!(vol.total_bytes, 30_000_000_000);
            assert_eq!(vol.free_bytes, 29_950_000_000);
        }
        other => panic!("expected UninitializedVolume, got {:?}", other),
    }

    // 2. Trigger Device Initialization
    let initialized_state = engine
        .initialize_device(&mount_path, "My Classic")
        .expect("initialize device");

    // 3. Verify canonical iPod_Control directory hierarchy (Music/F00..F49, iTunes, Device/SysInfo)
    for idx in 0..50 {
        let folder = mount_path
            .join("iPod_Control")
            .join("Music")
            .join(format!("F{:02}", idx));
        assert!(
            folder.is_dir(),
            "expected music folder {:?} to exist",
            folder
        );
    }

    let sysinfo_path = mount_path
        .join("iPod_Control")
        .join("Device")
        .join("SysInfo");
    assert!(sysinfo_path.is_file(), "expected SysInfo file to exist");
    let sysinfo_text = fs::read_to_string(&sysinfo_path).expect("read SysInfo");
    assert!(
        sysinfo_text.contains("FireWireGuid: 0x000A27001A2B3C4D"),
        "expected SysInfo to cache FireWire GUID, got: {}",
        sysinfo_text
    );

    let itunesdb_path = mount_path
        .join("iPod_Control")
        .join("iTunes")
        .join("iTunesDB");
    assert!(itunesdb_path.is_file(), "expected iTunesDB file to exist");

    // 4. Verify returned Device Library state has empty tracks and a Master Playlist
    match initialized_state {
        DeviceConnectionState::Connected(session) => {
            assert_eq!(session.device.name, "My Classic");
            assert_eq!(
                session.device.firewire_guid.as_deref(),
                Some("000A27001A2B3C4D")
            );
            assert_eq!(session.device.total_bytes, 30_000_000_000);
            assert_eq!(session.device.free_bytes, 29_950_000_000);
            assert_eq!(session.device.used_bytes, 50_000_000);
            assert!(session.library.tracks.is_empty());
            assert_eq!(session.library.playlists.len(), 1);
            assert_eq!(session.library.playlists[0].name, "My Classic");
            assert_eq!(session.library.playlists[0].kind, PlaylistKind::Master);
            assert!(session.library.playlists[0].track_ids.is_empty());
        }
        other => panic!("expected Connected state after initialization, got {:?}", other),
    }

    // 5. Subsequent scan_device() loads the newly initialized Device Library automatically
    let rescanned = engine.scan_device().expect("rescan initialized device");
    match rescanned {
        DeviceConnectionState::Connected(session) => {
            assert_eq!(session.device.name, "My Classic");
            assert_eq!(session.library.playlists.len(), 1);
            assert_eq!(session.library.playlists[0].kind, PlaylistKind::Master);
        }
        other => panic!("expected Connected on rescan, got {:?}", other),
    }
}

fn build_string_mhod_fixture(mhod_type: u32, value: &str) -> Vec<u8> {
    let utf16: Vec<u8> = value
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let total_len = 40 + utf16.len() as u32;
    let mut buf = vec![0u8; total_len as usize];
    buf[0..4].copy_from_slice(b"mhod");
    buf[4..8].copy_from_slice(&24u32.to_le_bytes());
    buf[8..12].copy_from_slice(&total_len.to_le_bytes());
    buf[12..16].copy_from_slice(&mhod_type.to_le_bytes());
    buf[24..28].copy_from_slice(&1u32.to_le_bytes());
    buf[28..32].copy_from_slice(&(utf16.len() as u32).to_le_bytes());
    buf[32..36].copy_from_slice(&1u32.to_le_bytes());
    buf[40..].copy_from_slice(&utf16);
    buf
}

fn build_raw_mhod_fixture(mhod_type: u32, payload: &[u8]) -> Vec<u8> {
    let total_len = 24 + payload.len() as u32;
    let mut buf = vec![0u8; 24];
    buf[0..4].copy_from_slice(b"mhod");
    buf[4..8].copy_from_slice(&24u32.to_le_bytes());
    buf[8..12].copy_from_slice(&total_len.to_le_bytes());
    buf[12..16].copy_from_slice(&mhod_type.to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

#[allow(clippy::too_many_arguments)]
fn build_mhit_fixture(
    track_id: u32,
    title: &str,
    artist: &str,
    album: &str,
    genre: &str,
    obfuscated_path: &str,
    track_number: u32,
    disc_number: u32,
    year: u32,
    duration_ms: u32,
    bitrate_kbps: u32,
    play_count: u32,
    rating_stars: u8,
    extra_mhods: Vec<Vec<u8>>,
) -> Vec<u8> {
    let mut mhods = vec![
        build_string_mhod_fixture(1, title),
        build_string_mhod_fixture(4, artist),
        build_string_mhod_fixture(3, album),
        build_string_mhod_fixture(5, genre),
        build_string_mhod_fixture(2, obfuscated_path),
    ];
    mhods.extend(extra_mhods);

    let header_len = 244u32;
    let body_len: usize = mhods.iter().map(|m| m.len()).sum();
    let total_len = header_len + body_len as u32;

    let mut header = vec![0u8; header_len as usize];
    header[0..4].copy_from_slice(b"mhit");
    header[4..8].copy_from_slice(&header_len.to_le_bytes());
    header[8..12].copy_from_slice(&total_len.to_le_bytes());
    header[12..16].copy_from_slice(&(mhods.len() as u32).to_le_bytes());
    header[16..20].copy_from_slice(&track_id.to_le_bytes());
    header[20..24].copy_from_slice(&1u32.to_le_bytes());
    header[30] = rating_stars * 20;
    header[40..44].copy_from_slice(&duration_ms.to_le_bytes());
    header[44..48].copy_from_slice(&track_number.to_le_bytes());
    header[52..56].copy_from_slice(&year.to_le_bytes());
    header[56..60].copy_from_slice(&bitrate_kbps.to_le_bytes());
    header[80..84].copy_from_slice(&play_count.to_le_bytes());
    header[92..96].copy_from_slice(&disc_number.to_le_bytes());
    // Put a sentinel byte in the trailing header region to verify lossless header preservation
    header[240..244].copy_from_slice(&0xDEADBEEFu32.to_le_bytes());

    for mhod in mhods {
        header.extend_from_slice(&mhod);
    }
    header
}

fn build_mhip_fixture(track_id: u32, order_index: u32) -> Vec<u8> {
    let order_mhod = build_raw_mhod_fixture(100, &order_index.to_le_bytes());
    let header_len = 76u32;
    let total_len = header_len + order_mhod.len() as u32;
    let mut mhip = vec![0u8; header_len as usize];
    mhip[0..4].copy_from_slice(b"mhip");
    mhip[4..8].copy_from_slice(&header_len.to_le_bytes());
    mhip[8..12].copy_from_slice(&total_len.to_le_bytes());
    mhip[12..16].copy_from_slice(&1u32.to_le_bytes());
    mhip[24..28].copy_from_slice(&track_id.to_le_bytes());
    mhip.extend_from_slice(&order_mhod);
    mhip
}

fn build_mhyp_fixture(
    playlist_id: u64,
    name: &str,
    is_master: bool,
    smart_mhods: Vec<Vec<u8>>,
    track_ids: &[u32],
) -> Vec<u8> {
    let mut mhods = vec![build_string_mhod_fixture(1, name)];
    mhods.extend(smart_mhods);

    let mhips: Vec<Vec<u8>> = track_ids
        .iter()
        .enumerate()
        .map(|(idx, &tid)| build_mhip_fixture(tid, (idx + 1) as u32))
        .collect();

    let header_len = 108u32;
    let mhods_len: usize = mhods.iter().map(|m| m.len()).sum();
    let mhips_len: usize = mhips.iter().map(|m| m.len()).sum();
    let total_len = header_len + (mhods_len + mhips_len) as u32;

    let mut mhyp = vec![0u8; header_len as usize];
    mhyp[0..4].copy_from_slice(b"mhyp");
    mhyp[4..8].copy_from_slice(&header_len.to_le_bytes());
    mhyp[8..12].copy_from_slice(&total_len.to_le_bytes());
    mhyp[12..16].copy_from_slice(&(mhods.len() as u32).to_le_bytes());
    mhyp[16..20].copy_from_slice(&(mhips.len() as u32).to_le_bytes());
    mhyp[20] = if is_master { 1 } else { 0 };
    mhyp[28..36].copy_from_slice(&playlist_id.to_le_bytes());

    for mhod in mhods {
        mhyp.extend_from_slice(&mhod);
    }
    for mhip in mhips {
        mhyp.extend_from_slice(&mhip);
    }
    mhyp
}

#[test]
fn loads_device_library_with_tracks_user_playlists_smart_playlists_and_preserves_unknown_chunks() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let mount_path: PathBuf = temp_dir.path().to_path_buf();

    let itunes_dir = mount_path.join("iPod_Control").join("iTunes");
    let device_dir = mount_path.join("iPod_Control").join("Device");
    fs::create_dir_all(&itunes_dir).expect("create iTunes dir");
    fs::create_dir_all(&device_dir).expect("create Device dir");

    fs::write(
        device_dir.join("SysInfo"),
        "ModelNumStr: MA147\nFireWireGuid: 0x000A2700AABBCCDD\n",
    )
    .expect("write SysInfo");

    // Build binary iTunesDB with 2 tracks, 3 playlists (Master, User, Smart), unknown mhod, and extra mhsd
    let track1 = build_mhit_fixture(
        101,
        "Paranoid Android",
        "Radiohead",
        "OK Computer",
        "Alternative",
        ":iPod_Control:Music:F04:ABCD.m4a",
        2,
        1,
        1997,
        383_000,
        945,
        12,
        5,
        vec![build_raw_mhod_fixture(99, &[0x11, 0x22, 0x33, 0x44])],
    );
    let track2 = build_mhit_fixture(
        102,
        "Everything In Its Right Place",
        "Radiohead",
        "Kid A",
        "Electronic",
        ":iPod_Control:Music:F12:EFGH.mp3",
        1,
        1,
        2000,
        251_000,
        320,
        4,
        4,
        vec![],
    );

    let mut mhlt = vec![0u8; 92];
    mhlt[0..4].copy_from_slice(b"mhlt");
    mhlt[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlt[8..12].copy_from_slice(&2u32.to_le_bytes());
    mhlt.extend_from_slice(&track1);
    mhlt.extend_from_slice(&track2);

    let mut mhsd_tracks = vec![0u8; 96];
    mhsd_tracks[0..4].copy_from_slice(b"mhsd");
    mhsd_tracks[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_tracks[8..12].copy_from_slice(&((96 + mhlt.len()) as u32).to_le_bytes());
    mhsd_tracks[12..16].copy_from_slice(&1u32.to_le_bytes());
    mhsd_tracks.extend_from_slice(&mhlt);

    let master_pl = build_mhyp_fixture(1000, "Classic 80GB", true, vec![], &[101, 102]);
    let user_pl = build_mhyp_fixture(2000, "Road Trip", false, vec![], &[102, 101]);
    let smart_pl = build_mhyp_fixture(
        3000,
        "Top Rated",
        false,
        vec![
            build_raw_mhod_fixture(50, &[1, 2, 3, 4, 5, 6, 7, 8]),
            build_raw_mhod_fixture(51, &[9, 10, 11, 12, 13, 14, 15, 16]),
        ],
        &[101],
    );

    let mut mhlp = vec![0u8; 92];
    mhlp[0..4].copy_from_slice(b"mhlp");
    mhlp[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlp[8..12].copy_from_slice(&3u32.to_le_bytes());
    mhlp.extend_from_slice(&master_pl);
    mhlp.extend_from_slice(&user_pl);
    mhlp.extend_from_slice(&smart_pl);

    let mut mhsd_playlists = vec![0u8; 96];
    mhsd_playlists[0..4].copy_from_slice(b"mhsd");
    mhsd_playlists[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_playlists[8..12].copy_from_slice(&((96 + mhlp.len()) as u32).to_le_bytes());
    mhsd_playlists[12..16].copy_from_slice(&2u32.to_le_bytes());
    mhsd_playlists.extend_from_slice(&mhlp);

    // Non-music mhsd dataset (type 4) to verify lossless round-trip
    let mut mhsd_extra = vec![0u8; 96];
    mhsd_extra[0..4].copy_from_slice(b"mhsd");
    mhsd_extra[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_extra[8..12].copy_from_slice(&104u32.to_le_bytes());
    mhsd_extra[12..16].copy_from_slice(&4u32.to_le_bytes());
    mhsd_extra.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x11]);

    let mhbd_header_len = 104u32;
    let total_len =
        mhbd_header_len + (mhsd_tracks.len() + mhsd_playlists.len() + mhsd_extra.len()) as u32;
    let mut original_db_bytes = vec![0u8; mhbd_header_len as usize];
    original_db_bytes[0..4].copy_from_slice(b"mhbd");
    original_db_bytes[4..8].copy_from_slice(&mhbd_header_len.to_le_bytes());
    original_db_bytes[8..12].copy_from_slice(&total_len.to_le_bytes());
    original_db_bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
    original_db_bytes[16..20].copy_from_slice(&0x19u32.to_le_bytes());
    original_db_bytes[20..24].copy_from_slice(&3u32.to_le_bytes());
    original_db_bytes.extend_from_slice(&mhsd_tracks);
    original_db_bytes.extend_from_slice(&mhsd_playlists);
    original_db_bytes.extend_from_slice(&mhsd_extra);

    fs::write(itunes_dir.join("iTunesDB"), &original_db_bytes).expect("write iTunesDB fixture");

    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 80_000_000_000,
            free_bytes: 60_000_000_000,
            usb_firewire_guid: None,
        }],
    };

    let engine = PodLoaderEngine::new(adapter);
    let state = engine.scan_device().expect("scan device with tracks");

    match state {
        DeviceConnectionState::Connected(session) => {
            assert_eq!(session.device.name, "Classic 80GB");
            assert_eq!(session.device.model_number.as_deref(), Some("MA147"));
            assert_eq!(
                session.device.firewire_guid.as_deref(),
                Some("000A2700AABBCCDD")
            );
            assert_eq!(session.device.total_bytes, 80_000_000_000);
            assert_eq!(session.device.free_bytes, 60_000_000_000);
            assert_eq!(session.device.used_bytes, 20_000_000_000);

            assert_eq!(session.library.tracks.len(), 2);
            let t1 = &session.library.tracks[0];
            assert_eq!(t1.id, 101);
            assert_eq!(t1.title, "Paranoid Android");
            assert_eq!(t1.artist, "Radiohead");
            assert_eq!(t1.album, "OK Computer");
            assert_eq!(t1.genre, "Alternative");
            assert_eq!(t1.track_number, 2);
            assert_eq!(t1.disc_number, 1);
            assert_eq!(t1.year, 1997);
            assert_eq!(t1.duration_ms, 383_000);
            assert_eq!(t1.bitrate_kbps, 945);
            assert_eq!(t1.play_count, 12);
            assert_eq!(t1.rating, 5);
            assert_eq!(t1.obfuscated_path, ":iPod_Control:Music:F04:ABCD.m4a");

            let t2 = &session.library.tracks[1];
            assert_eq!(t2.id, 102);
            assert_eq!(t2.title, "Everything In Its Right Place");
            assert_eq!(t2.album, "Kid A");
            assert_eq!(t2.year, 2000);
            assert_eq!(t2.rating, 4);

            assert_eq!(session.library.playlists.len(), 3);
            assert_eq!(session.library.playlists[0].name, "Classic 80GB");
            assert_eq!(session.library.playlists[0].kind, PlaylistKind::Master);
            assert_eq!(session.library.playlists[0].track_ids, vec![101, 102]);

            assert_eq!(session.library.playlists[1].name, "Road Trip");
            assert_eq!(session.library.playlists[1].kind, PlaylistKind::User);
            assert_eq!(session.library.playlists[1].track_ids, vec![102, 101]);

            assert_eq!(session.library.playlists[2].name, "Top Rated");
            assert_eq!(session.library.playlists[2].kind, PlaylistKind::Smart);
            assert_eq!(session.library.playlists[2].track_ids, vec![101]);
        }
        other => panic!("expected Connected, got {:?}", other),
    }

    // Verify lossless round-trip of the binary iTunesDB (ADR-0006)
    let roundtripped = engine
        .reserialize_device_db(&mount_path)
        .expect("reserialize iTunesDB");
    assert_eq!(roundtripped, original_db_bytes);
}

#[test]
fn merges_play_counts_file_into_loaded_device_tracks() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let mount_path: PathBuf = temp_dir.path().to_path_buf();

    let itunes_dir = mount_path.join("iPod_Control").join("iTunes");
    fs::create_dir_all(&itunes_dir).expect("create iTunes dir");

    let track1 = build_mhit_fixture(
        201,
        "Airbag",
        "Radiohead",
        "OK Computer",
        "Alternative",
        ":iPod_Control:Music:F00:TRK1.m4a",
        1,
        1,
        1997,
        284_000,
        920,
        3,
        3,
        vec![],
    );
    let track2 = build_mhit_fixture(
        202,
        "Subterranean Homesick Alien",
        "Radiohead",
        "OK Computer",
        "Alternative",
        ":iPod_Control:Music:F01:TRK2.m4a",
        3,
        1,
        1997,
        267_000,
        910,
        0,
        0,
        vec![],
    );

    let mut mhlt = vec![0u8; 92];
    mhlt[0..4].copy_from_slice(b"mhlt");
    mhlt[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlt[8..12].copy_from_slice(&2u32.to_le_bytes());
    mhlt.extend_from_slice(&track1);
    mhlt.extend_from_slice(&track2);

    let mut mhsd_tracks = vec![0u8; 96];
    mhsd_tracks[0..4].copy_from_slice(b"mhsd");
    mhsd_tracks[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_tracks[8..12].copy_from_slice(&((96 + mhlt.len()) as u32).to_le_bytes());
    mhsd_tracks[12..16].copy_from_slice(&1u32.to_le_bytes());
    mhsd_tracks.extend_from_slice(&mhlt);

    let master_pl = build_mhyp_fixture(1, "My iPod", true, vec![], &[201, 202]);
    let mut mhlp = vec![0u8; 92];
    mhlp[0..4].copy_from_slice(b"mhlp");
    mhlp[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlp[8..12].copy_from_slice(&1u32.to_le_bytes());
    mhlp.extend_from_slice(&master_pl);

    let mut mhsd_playlists = vec![0u8; 96];
    mhsd_playlists[0..4].copy_from_slice(b"mhsd");
    mhsd_playlists[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_playlists[8..12].copy_from_slice(&((96 + mhlp.len()) as u32).to_le_bytes());
    mhsd_playlists[12..16].copy_from_slice(&2u32.to_le_bytes());
    mhsd_playlists.extend_from_slice(&mhlp);

    let mhbd_header_len = 104u32;
    let total_len = mhbd_header_len + (mhsd_tracks.len() + mhsd_playlists.len()) as u32;
    let mut db_bytes = vec![0u8; mhbd_header_len as usize];
    db_bytes[0..4].copy_from_slice(b"mhbd");
    db_bytes[4..8].copy_from_slice(&mhbd_header_len.to_le_bytes());
    db_bytes[8..12].copy_from_slice(&total_len.to_le_bytes());
    db_bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
    db_bytes[16..20].copy_from_slice(&0x19u32.to_le_bytes());
    db_bytes[20..24].copy_from_slice(&2u32.to_le_bytes());
    db_bytes.extend_from_slice(&mhsd_tracks);
    db_bytes.extend_from_slice(&mhsd_playlists);

    fs::write(itunes_dir.join("iTunesDB"), &db_bytes).expect("write iTunesDB");

    // Write binary Play Counts file (mhdp header = 96 bytes, entry_len = 28 bytes, 2 entries)
    let mut play_counts = vec![0u8; 96];
    play_counts[0..4].copy_from_slice(b"mhdp");
    play_counts[4..8].copy_from_slice(&96u32.to_le_bytes());
    play_counts[8..12].copy_from_slice(&28u32.to_le_bytes());
    play_counts[12..16].copy_from_slice(&2u32.to_le_bytes());

    // Entry 0 (for track 201): +2 plays, last_played = 3_800_000_000, bookmark = 45_000 ms, rating = 100 (5 stars)
    let mut entry0 = vec![0u8; 28];
    entry0[0..4].copy_from_slice(&2u32.to_le_bytes());
    entry0[4..8].copy_from_slice(&3_800_000_000u32.to_le_bytes());
    entry0[8..12].copy_from_slice(&45_000u32.to_le_bytes());
    entry0[12..16].copy_from_slice(&100u32.to_le_bytes());
    play_counts.extend_from_slice(&entry0);

    // Entry 1 (for track 202): +1 play, last_played = 3_800_005_000, bookmark = 0, rating = 0 (unchanged)
    let mut entry1 = vec![0u8; 28];
    entry1[0..4].copy_from_slice(&1u32.to_le_bytes());
    entry1[4..8].copy_from_slice(&3_800_005_000u32.to_le_bytes());
    entry1[8..12].copy_from_slice(&0u32.to_le_bytes());
    entry1[12..16].copy_from_slice(&0u32.to_le_bytes());
    play_counts.extend_from_slice(&entry1);

    fs::write(itunes_dir.join("Play Counts"), &play_counts).expect("write Play Counts");

    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path,
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 25_000_000_000,
            usb_firewire_guid: None,
        }],
    };

    let engine = PodLoaderEngine::new(adapter);
    let state = engine.scan_device().expect("scan device with Play Counts");

    match state {
        DeviceConnectionState::Connected(session) => {
            assert!(session.has_pending_play_counts);
            assert_eq!(session.library.tracks.len(), 2);

            let t1 = &session.library.tracks[0];
            assert_eq!(t1.id, 201);
            assert_eq!(t1.play_count, 5); // 3 + 2
            assert_eq!(t1.last_played_timestamp, Some(3_800_000_000));
            assert_eq!(t1.bookmark_ms, 45_000);
            assert_eq!(t1.rating, 5); // updated from 3 to 5 stars

            let t2 = &session.library.tracks[1];
            assert_eq!(t2.id, 202);
            assert_eq!(t2.play_count, 1); // 0 + 1
            assert_eq!(t2.last_played_timestamp, Some(3_800_005_000));
            assert_eq!(t2.bookmark_ms, 0);
            assert_eq!(t2.rating, 0);
        }
        other => panic!("expected Connected, got {:?}", other),
    }
}

fn build_id3v23_mp3_fixture(
    title: &str,
    artist: &str,
    album: &str,
    track_number: u32,
    genre: &str,
    year: u32,
    duration_seconds: u32,
) -> Vec<u8> {
    fn encode_text_frame(frame_id: &[u8; 4], text: &str) -> Vec<u8> {
        let payload_len = 1 + text.len() as u32; // encoding byte 0 (ISO-8859-1/UTF-8) + text
        let mut frame = Vec::with_capacity(10 + payload_len as usize);
        frame.extend_from_slice(frame_id);
        frame.extend_from_slice(&payload_len.to_be_bytes());
        frame.extend_from_slice(&[0, 0]); // flags
        frame.push(3); // UTF-8 text encoding
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
    mp3.extend_from_slice(&[3, 0, 0]); // v2.3.0, no flags
    mp3.extend_from_slice(&synchsafe);
    mp3.extend_from_slice(&frames);

    // Append synthetic 128 kbps CBR MP3 payload (16_000 bytes per second)
    let audio_bytes = (duration_seconds as usize) * 16_000;
    let mut audio = vec![0u8; audio_bytes.max(4)];
    // MPEG-1 Layer III 128kbps 44.1kHz frame sync header: 0xFF 0xFB 0x90 0x00
    audio[0] = 0xFF;
    audio[1] = 0xFB;
    audio[2] = 0x90;
    audio[3] = 0x00;
    mp3.extend_from_slice(&audio);
    mp3
}

fn build_wav_fixture(sample_rate: u32, duration_seconds: u32) -> Vec<u8> {
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
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
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

fn build_m4a_alac_fixture(
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
        out.extend_from_slice(&1u32.to_be_bytes()); // UTF-8 type indicator
        out.extend_from_slice(&0u32.to_be_bytes()); // locale
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

    let mut m4a = Vec::new();
    // ftyp + alac codec marker
    m4a.extend_from_slice(&20u32.to_be_bytes());
    m4a.extend_from_slice(b"ftypM4A \x00\x00\x00\x00alac");

    // mvhd atom (version 0: 4 bytes version/flags + 8 bytes times + 4 bytes timescale + 4 bytes duration)
    let timescale = 44_100u32;
    let duration = timescale * duration_seconds;
    let mut moov_payload = Vec::new();
    moov_payload.extend_from_slice(&36u32.to_be_bytes());
    moov_payload.extend_from_slice(b"mvhd");
    moov_payload.extend_from_slice(&[0u8; 12]); // version 0 + flags + creation/mod time
    moov_payload.extend_from_slice(&timescale.to_be_bytes());
    moov_payload.extend_from_slice(&duration.to_be_bytes());
    moov_payload.extend_from_slice(&[0u8; 8]);

    // ilst atoms inside udta -> meta
    let mut ilst_payload = Vec::new();
    ilst_payload.extend_from_slice(&encode_ilst_text_atom(b"\xA9nam", title));
    ilst_payload.extend_from_slice(&encode_ilst_text_atom(b"\xA9ART", artist));
    ilst_payload.extend_from_slice(&encode_ilst_text_atom(b"\xA9alb", album));
    ilst_payload.extend_from_slice(&encode_trkn_atom(track_number));

    let mut meta_payload = vec![0u8; 4]; // version/flags
    let ilst_len = (8 + ilst_payload.len()) as u32;
    meta_payload.extend_from_slice(&ilst_len.to_be_bytes());
    meta_payload.extend_from_slice(b"ilst");
    meta_payload.extend_from_slice(&ilst_payload);

    let meta_len = (8 + meta_payload.len()) as u32;
    let mut udta_payload = Vec::new();
    udta_payload.extend_from_slice(&meta_len.to_be_bytes());
    udta_payload.extend_from_slice(b"meta");
    udta_payload.extend_from_slice(&meta_payload);

    let udta_len = (8 + udta_payload.len()) as u32;
    moov_payload.extend_from_slice(&udta_len.to_be_bytes());
    moov_payload.extend_from_slice(b"udta");
    moov_payload.extend_from_slice(&udta_payload);

    let moov_len = (8 + moov_payload.len()) as u32;
    m4a.extend_from_slice(&moov_len.to_be_bytes());
    m4a.extend_from_slice(b"moov");
    m4a.extend_from_slice(&moov_payload);
    m4a
}

#[test]
fn browses_host_directory_statelessly_and_extracts_audio_tags() {
    let host_temp = TempDir::new().expect("create host temp dir");
    let host_root = host_temp.path();

    fs::create_dir_all(host_root.join("Albums")).expect("create subfolder");
    fs::write(host_root.join("notes.txt"), "ignore non-audio files").expect("write txt");

    let mp3_bytes = build_id3v23_mp3_fixture(
        "Let Down",
        "Radiohead",
        "OK Computer",
        5,
        "Alternative",
        1997,
        10,
    );
    fs::write(host_root.join("05 - Let Down.mp3"), &mp3_bytes).expect("write mp3 fixture");

    let wav_bytes = build_wav_fixture(44_100, 2);
    fs::write(host_root.join("01 - Intro Jam.wav"), &wav_bytes).expect("write wav fixture");

    let alac_bytes = build_m4a_alac_fixture("Exit Music (For a Film)", "Radiohead", "OK Computer", 4, 8);
    fs::write(host_root.join("04 - Exit Music.m4a"), &alac_bytes).expect("write m4a fixture");

    let adapter = FakeHardwareAdapter { volumes: vec![] };
    let engine = PodLoaderEngine::new(adapter);

    let listing = engine
        .browse_host_directory(host_root)
        .expect("browse host directory");

    assert_eq!(listing.current_path, host_root);
    assert_eq!(listing.subdirectories.len(), 1);
    assert_eq!(listing.subdirectories[0].name, "Albums");

    assert_eq!(listing.files.len(), 3);

    // Track 1: WAV file (fallback to filename parsing for track # and title, accurate WAV duration & bitrate)
    let f1 = &listing.files[0];
    assert_eq!(f1.file_name, "01 - Intro Jam.wav");
    assert_eq!(f1.format, "WAV");
    assert_eq!(f1.track_number, 1);
    assert_eq!(f1.title, "Intro Jam");
    assert_eq!(f1.duration_ms, 2_000);
    assert_eq!(f1.bitrate_kbps, 1411);

    // Track 4: ALAC M4A file with MP4 mvhd + ilst metadata atoms
    let f2 = &listing.files[1];
    assert_eq!(f2.file_name, "04 - Exit Music.m4a");
    assert_eq!(f2.format, "ALAC");
    assert_eq!(f2.title, "Exit Music (For a Film)");
    assert_eq!(f2.artist, "Radiohead");
    assert_eq!(f2.album, "OK Computer");
    assert_eq!(f2.track_number, 4);
    assert_eq!(f2.duration_ms, 8_000);

    // Track 5: MP3 file with ID3v2.3 tags
    let f3 = &listing.files[2];
    assert_eq!(f3.file_name, "05 - Let Down.mp3");
    assert_eq!(f3.format, "MP3");
    assert_eq!(f3.title, "Let Down");
    assert_eq!(f3.artist, "Radiohead");
    assert_eq!(f3.album, "OK Computer");
    assert_eq!(f3.track_number, 5);
    assert_eq!(f3.genre, "Alternative");
    assert_eq!(f3.year, 1997);
    assert_eq!(f3.duration_ms, 10_000);
    assert_eq!(f3.bitrate_kbps, 128);
}

#[test]
fn stages_host_files_detects_duplicates_and_rejects_commit_when_free_space_insufficient() {
    let device_temp = TempDir::new().expect("create device temp dir");
    let mount_path = device_temp.path().to_path_buf();
    let itunes_dir = mount_path.join("iPod_Control").join("iTunes");
    fs::create_dir_all(&itunes_dir).expect("create iTunes dir");

    // Existing Device Track: Radiohead - OK Computer - #2 Paranoid Android
    let existing_track = build_mhit_fixture(
        101,
        "Paranoid Android",
        "Radiohead",
        "OK Computer",
        "Alternative",
        ":iPod_Control:Music:F04:ABCD.m4a",
        2,
        1,
        1997,
        383_000,
        945,
        12,
        5,
        vec![],
    );
    let mut mhlt = vec![0u8; 92];
    mhlt[0..4].copy_from_slice(b"mhlt");
    mhlt[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlt[8..12].copy_from_slice(&1u32.to_le_bytes());
    mhlt.extend_from_slice(&existing_track);

    let mut mhsd_tracks = vec![0u8; 96];
    mhsd_tracks[0..4].copy_from_slice(b"mhsd");
    mhsd_tracks[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_tracks[8..12].copy_from_slice(&((96 + mhlt.len()) as u32).to_le_bytes());
    mhsd_tracks[12..16].copy_from_slice(&1u32.to_le_bytes());
    mhsd_tracks.extend_from_slice(&mhlt);

    let master_pl = build_mhyp_fixture(1, "My iPod", true, vec![], &[101]);
    let mut mhlp = vec![0u8; 92];
    mhlp[0..4].copy_from_slice(b"mhlp");
    mhlp[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlp[8..12].copy_from_slice(&1u32.to_le_bytes());
    mhlp.extend_from_slice(&master_pl);

    let mut mhsd_playlists = vec![0u8; 96];
    mhsd_playlists[0..4].copy_from_slice(b"mhsd");
    mhsd_playlists[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_playlists[8..12].copy_from_slice(&((96 + mhlp.len()) as u32).to_le_bytes());
    mhsd_playlists[12..16].copy_from_slice(&2u32.to_le_bytes());
    mhsd_playlists.extend_from_slice(&mhlp);

    let mhbd_header_len = 104u32;
    let total_len = mhbd_header_len + (mhsd_tracks.len() + mhsd_playlists.len()) as u32;
    let mut db_bytes = vec![0u8; mhbd_header_len as usize];
    db_bytes[0..4].copy_from_slice(b"mhbd");
    db_bytes[4..8].copy_from_slice(&mhbd_header_len.to_le_bytes());
    db_bytes[8..12].copy_from_slice(&total_len.to_le_bytes());
    db_bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
    db_bytes[16..20].copy_from_slice(&0x19u32.to_le_bytes());
    db_bytes[20..24].copy_from_slice(&2u32.to_le_bytes());
    db_bytes.extend_from_slice(&mhsd_tracks);
    db_bytes.extend_from_slice(&mhsd_playlists);
    fs::write(itunes_dir.join("iTunesDB"), &db_bytes).expect("write iTunesDB");

    // Host files: one duplicate (normalized case/whitespace match) and one brand-new track
    let host_temp = TempDir::new().expect("create host temp dir");
    let dup_mp3 = build_id3v23_mp3_fixture(
        "  paranoid android ",
        "RADIOHEAD",
        "ok computer",
        2,
        "Alternative",
        1997,
        5,
    );
    let new_mp3 = build_id3v23_mp3_fixture(
        "Karma Police",
        "Radiohead",
        "OK Computer",
        6,
        "Alternative",
        1997,
        5,
    );
    let dup_path = host_temp.path().join("02 - Paranoid Android.mp3");
    let new_path = host_temp.path().join("06 - Karma Police.mp3");
    fs::write(&dup_path, &dup_mp3).expect("write dup mp3");
    fs::write(&new_path, &new_mp3).expect("write new mp3");

    // Simulate tight free space: enough for 1 track (100 KB), NOT enough for 2 tracks (~160 KB)
    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 100_000,
            usb_firewire_guid: None,
        }],
    };

    let mut engine = PodLoaderEngine::new(adapter);
    engine.scan_device().expect("initial scan");

    // Stage the entire host directory
    let batch = engine
        .stage_host_paths(&[host_temp.path().to_path_buf()], None)
        .expect("stage host folder");

    assert_eq!(batch.additions.len(), 2);
    let staged_dup = &batch.additions[0];
    assert!(staged_dup.is_duplicate);
    assert_eq!(staged_dup.matched_device_track_id, Some(101));
    assert_eq!(staged_dup.duplicate_resolution, DuplicateResolution::Skip);

    let staged_new = &batch.additions[1];
    assert!(!staged_new.is_duplicate);
    assert_eq!(staged_new.matched_device_track_id, None);
    assert_eq!(
        staged_new.duplicate_resolution,
        DuplicateResolution::ImportAnyway
    );

    // Because duplicate defaults to Skip, required_bytes only counts the new track (~80 KB <= 100 KB)
    assert_eq!(batch.required_bytes, new_mp3.len() as u64);

    // Override duplicate to ImportAnyway -> now required_bytes = dup + new (~160 KB > 100 KB free!)
    let updated_batch = engine
        .set_duplicate_resolution(staged_dup.staged_id, DuplicateResolution::ImportAnyway)
        .expect("override duplicate resolution");
    assert_eq!(
        updated_batch.required_bytes,
        (dup_mp3.len() + new_mp3.len()) as u64
    );

    // Commit must fail pre-flight space check without writing any audio files
    let err = engine
        .commit_sync_batch()
        .expect_err("expected InsufficientDiskSpace error");
    match err {
        PodLoaderError::InsufficientDiskSpace {
            required_bytes,
            available_bytes,
        } => {
            assert_eq!(required_bytes, (dup_mp3.len() + new_mp3.len()) as u64);
            assert_eq!(available_bytes, 100_000);
        }
        other => panic!("expected InsufficientDiskSpace, got {:?}", other),
    }
}

#[test]
fn commits_sync_batch_atomically_allocates_obfuscated_paths_creates_backup_deletes_play_counts_and_signs_hashes()
 {
    let device_temp = TempDir::new().expect("create device temp dir");
    let mount_path = device_temp.path().to_path_buf();
    let ipod_control = mount_path.join("iPod_Control");
    let music_dir = ipod_control.join("Music");
    for idx in 0..50 {
        fs::create_dir_all(music_dir.join(format!("F{:02}", idx))).expect("create Fxx");
    }
    let itunes_dir = ipod_control.join("iTunes");
    let device_dir = ipod_control.join("Device");
    fs::create_dir_all(&itunes_dir).expect("create iTunes dir");
    fs::create_dir_all(&device_dir).expect("create Device dir");

    // Write old file for track 101 so we can verify ReplaceExisting cleans it up
    let old_track_file = music_dir.join("F04").join("ABCD.m4a");
    fs::write(&old_track_file, b"old-audio-bytes").expect("write old track file");

    let track101 = build_mhit_fixture(
        101,
        "Paranoid Android",
        "Radiohead",
        "OK Computer",
        "Alternative",
        ":iPod_Control:Music:F04:ABCD.m4a",
        2,
        1,
        1997,
        383_000,
        945,
        3,
        4,
        vec![build_raw_mhod_fixture(99, &[0xDE, 0xAD, 0xBE, 0xEF])],
    );
    let mut mhlt = vec![0u8; 92];
    mhlt[0..4].copy_from_slice(b"mhlt");
    mhlt[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlt[8..12].copy_from_slice(&1u32.to_le_bytes());
    mhlt.extend_from_slice(&track101);

    let mut mhsd_tracks = vec![0u8; 96];
    mhsd_tracks[0..4].copy_from_slice(b"mhsd");
    mhsd_tracks[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_tracks[8..12].copy_from_slice(&((96 + mhlt.len()) as u32).to_le_bytes());
    mhsd_tracks[12..16].copy_from_slice(&1u32.to_le_bytes());
    mhsd_tracks.extend_from_slice(&mhlt);

    let master_pl = build_mhyp_fixture(1000, "Classic 160GB", true, vec![], &[101]);
    let user_pl = build_mhyp_fixture(2000, "Road Trip", false, vec![], &[]);
    let smart_pl = build_mhyp_fixture(
        3000,
        "Top Rated",
        false,
        vec![
            build_raw_mhod_fixture(50, &[1, 2, 3, 4]),
            build_raw_mhod_fixture(51, &[5, 6, 7, 8]),
        ],
        &[101],
    );
    let mut mhlp = vec![0u8; 92];
    mhlp[0..4].copy_from_slice(b"mhlp");
    mhlp[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlp[8..12].copy_from_slice(&3u32.to_le_bytes());
    mhlp.extend_from_slice(&master_pl);
    mhlp.extend_from_slice(&user_pl);
    mhlp.extend_from_slice(&smart_pl);

    let mut mhsd_playlists = vec![0u8; 96];
    mhsd_playlists[0..4].copy_from_slice(b"mhsd");
    mhsd_playlists[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_playlists[8..12].copy_from_slice(&((96 + mhlp.len()) as u32).to_le_bytes());
    mhsd_playlists[12..16].copy_from_slice(&2u32.to_le_bytes());
    mhsd_playlists.extend_from_slice(&mhlp);

    let mut mhsd_extra = vec![0u8; 96];
    mhsd_extra[0..4].copy_from_slice(b"mhsd");
    mhsd_extra[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_extra[8..12].copy_from_slice(&100u32.to_le_bytes());
    mhsd_extra[12..16].copy_from_slice(&4u32.to_le_bytes());
    mhsd_extra.extend_from_slice(&[0xCA, 0xFE, 0xBA, 0xBE]);

    let mhbd_header_len = 244u32;
    let total_len =
        mhbd_header_len + (mhsd_tracks.len() + mhsd_playlists.len() + mhsd_extra.len()) as u32;
    let mut initial_db = vec![0u8; mhbd_header_len as usize];
    initial_db[0..4].copy_from_slice(b"mhbd");
    initial_db[4..8].copy_from_slice(&mhbd_header_len.to_le_bytes());
    initial_db[8..12].copy_from_slice(&total_len.to_le_bytes());
    initial_db[12..16].copy_from_slice(&1u32.to_le_bytes());
    initial_db[16..20].copy_from_slice(&0x19u32.to_le_bytes());
    initial_db[20..24].copy_from_slice(&3u32.to_le_bytes());
    initial_db.extend_from_slice(&mhsd_tracks);
    initial_db.extend_from_slice(&mhsd_playlists);
    initial_db.extend_from_slice(&mhsd_extra);
    fs::write(itunes_dir.join("iTunesDB"), &initial_db).expect("write initial iTunesDB");

    // Pending Play Counts file (+2 plays for track 101)
    let mut play_counts = vec![0u8; 96];
    play_counts[0..4].copy_from_slice(b"mhdp");
    play_counts[4..8].copy_from_slice(&96u32.to_le_bytes());
    play_counts[8..12].copy_from_slice(&28u32.to_le_bytes());
    play_counts[12..16].copy_from_slice(&1u32.to_le_bytes());
    let mut pc_entry = vec![0u8; 28];
    pc_entry[0..4].copy_from_slice(&2u32.to_le_bytes());
    play_counts.extend_from_slice(&pc_entry);
    let play_counts_path = itunes_dir.join("Play Counts");
    fs::write(&play_counts_path, &play_counts).expect("write Play Counts");

    // Host files to stage
    let host_temp = TempDir::new().expect("create host temp");
    let replacement_mp3 = build_id3v23_mp3_fixture(
        "Paranoid Android",
        "Radiohead",
        "OK Computer",
        2,
        "Alternative",
        1997,
        6,
    );
    let karma_mp3 = build_id3v23_mp3_fixture(
        "Karma Police",
        "Radiohead",
        "OK Computer",
        6,
        "Alternative",
        1997,
        4,
    );
    let rep_path = host_temp.path().join("02 - Paranoid Android.mp3");
    let karma_path = host_temp.path().join("06 - Karma Police.mp3");
    fs::write(&rep_path, &replacement_mp3).expect("write replacement mp3");
    fs::write(&karma_path, &karma_mp3).expect("write karma mp3");

    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 160_000_000_000,
            free_bytes: 120_000_000_000,
            usb_firewire_guid: None,
        }],
    };
    let mut engine = PodLoaderEngine::new(adapter);
    engine.scan_device().expect("initial scan");

    // Set manual FireWire GUID (persists to SysInfo)
    let updated_session = engine
        .set_manual_firewire_guid("000A270011223344")
        .expect("set manual FireWire GUID");
    assert_eq!(
        updated_session.device.firewire_guid.as_deref(),
        Some("000A270011223344")
    );

    // Stage both host files targeting user playlist "Road Trip" (id 2000)
    let batch = engine
        .stage_host_paths(&[rep_path, karma_path], Some(2000))
        .expect("stage host paths");
    assert_eq!(batch.additions.len(), 2);
    let dup_staged_id = batch
        .additions
        .iter()
        .find(|a| a.is_duplicate)
        .expect("find dup")
        .staged_id;

    // Set duplicate resolution to ReplaceExisting
    engine
        .set_duplicate_resolution(dup_staged_id, DuplicateResolution::ReplaceExisting)
        .expect("set ReplaceExisting");

    // Commit Sync Batch
    let outcome = engine.commit_sync_batch().expect("commit sync batch");
    let committed_session = match outcome {
        CommitOutcome::Completed(s) => s,
        other => panic!("expected Completed, got {:?}", other),
    };

    // 1. Sync Batch is cleared after commit
    assert!(committed_session.sync_batch.additions.is_empty());

    // 2. iTunesDB.bak matches original database bytes, and Play Counts file is removed
    let bak_bytes = fs::read(itunes_dir.join("iTunesDB.bak")).expect("read iTunesDB.bak");
    assert_eq!(bak_bytes, initial_db);
    assert!(
        !play_counts_path.exists(),
        "expected consumed Play Counts file to be deleted on commit"
    );

    // 3. Old replaced file is deleted, and both tracks have valid 4-char Obfuscated Paths on disk
    assert!(
        !old_track_file.exists(),
        "expected replaced track's old Obfuscated Path file to be deleted"
    );
    assert_eq!(committed_session.library.tracks.len(), 2);
    for track in &committed_session.library.tracks {
        assert!(track.obfuscated_path.starts_with(":iPod_Control:Music:F"));
        let rel_path = track
            .obfuscated_path
            .trim_start_matches(':')
            .replace(':', "/");
        let full_disk_path = mount_path.join(&rel_path);
        assert!(
            full_disk_path.is_file(),
            "expected audio file at {:?} to exist on disk",
            full_disk_path
        );
        let stem = full_disk_path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("stem");
        assert_eq!(
            stem.len(),
            4,
            "expected 4-letter Obfuscated Path filename, got {}",
            stem
        );
    }

    // Merged play count (3 + 2 = 5) on replaced track 101 is preserved!
    let t101 = committed_session
        .library
        .tracks
        .iter()
        .find(|t| t.id == 101)
        .expect("track 101");
    assert_eq!(t101.play_count, 5);

    // 4. Both tracks are registered in Master Playlist (1000) and target Device Playlist "Road Trip" (2000)
    let master = committed_session
        .library
        .playlists
        .iter()
        .find(|p| p.id == 1000)
        .expect("master playlist");
    assert_eq!(master.track_ids.len(), 2);

    let road_trip = committed_session
        .library
        .playlists
        .iter()
        .find(|p| p.id == 2000)
        .expect("road trip playlist");
    assert_eq!(road_trip.track_ids.len(), 2);

    // 5. Smart Playlist "Top Rated" (3000) is preserved
    let smart = committed_session
        .library
        .playlists
        .iter()
        .find(|p| p.id == 3000)
        .expect("smart playlist");
    assert_eq!(smart.kind, PlaylistKind::Smart);

    // 6. Verify hash58 & hash72 signatures on the written iTunesDB
    let committed_db_bytes = fs::read(itunes_dir.join("iTunesDB")).expect("read committed iTunesDB");
    assert!(
        verify_itunesdb_signatures(&committed_db_bytes, "000A270011223344"),
        "expected valid hash58 and hash72 signatures on committed iTunesDB"
    );
    assert!(
        !verify_itunesdb_signatures(&committed_db_bytes, "000A270099999999"),
        "signature verification must fail against a different FireWire GUID"
    );
}

#[test]
fn recovers_from_mid_batch_interruption_with_rollback_or_commit_completed_tracks() {
    let device_temp = TempDir::new().expect("create device temp dir");
    let mount_path = device_temp.path().to_path_buf();
    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 25_000_000_000,
            usb_firewire_guid: Some("000A270055667788".to_string()),
        }],
    };
    let mut engine = PodLoaderEngine::new(adapter);
    engine
        .initialize_device(&mount_path, "Classic")
        .expect("init device");

    let host_temp = TempDir::new().expect("create host temp");
    let t1_path = host_temp.path().join("01 - First Track.mp3");
    let t2_path = host_temp.path().join("02 - Second Track.mp3");
    fs::write(
        &t1_path,
        build_id3v23_mp3_fixture("First Track", "Artist A", "Album A", 1, "Rock", 2001, 3),
    )
    .expect("write t1");
    fs::write(
        &t2_path,
        build_id3v23_mp3_fixture("Second Track", "Artist A", "Album A", 2, "Rock", 2001, 3),
    )
    .expect("write t2");

    engine
        .stage_host_paths(&[t1_path, t2_path], None)
        .expect("stage 2 tracks");

    // 1. Interrupt after copying 1 track -> Roll Back Batch
    let outcome1 = engine
        .commit_sync_batch_with_limit(Some(1))
        .expect("interrupt after 1 track");
    let first_written_disk_path = match outcome1 {
        CommitOutcome::Interrupted(summary) => {
            assert_eq!(summary.completed_track_count, 1);
            assert_eq!(summary.remaining_track_count, 1);
            assert_eq!(summary.written_obfuscated_paths.len(), 1);
            let rel = summary.written_obfuscated_paths[0]
                .trim_start_matches(':')
                .replace(':', "/");
            let disk_p = mount_path.join(rel);
            assert!(
                disk_p.is_file(),
                "expected copied file to exist before rollback"
            );
            disk_p
        }
        other => panic!("expected Interrupted, got {:?}", other),
    };

    let rolled_back = engine
        .resolve_interrupted_batch(InterruptedResolution::RollBack)
        .expect("roll back batch");
    assert!(
        !first_written_disk_path.exists(),
        "expected RollBack to delete newly copied Obfuscated Path file"
    );
    assert!(
        rolled_back.library.tracks.is_empty(),
        "expected iTunesDB to remain untouched on RollBack"
    );
    assert_eq!(
        rolled_back.sync_batch.additions.len(),
        2,
        "expected both staged items to remain in SyncBatch after RollBack"
    );

    // 2. Interrupt after copying 1 track -> Commit Completed Tracks
    let outcome2 = engine
        .commit_sync_batch_with_limit(Some(1))
        .expect("interrupt again after 1 track");
    match outcome2 {
        CommitOutcome::Interrupted(summary) => {
            assert_eq!(summary.completed_track_count, 1);
            assert_eq!(summary.remaining_track_count, 1);
        }
        other => panic!("expected Interrupted, got {:?}", other),
    }

    let partial_committed = engine
        .resolve_interrupted_batch(InterruptedResolution::CommitCompleted)
        .expect("commit completed tracks");
    assert_eq!(partial_committed.library.tracks.len(), 1);
    assert_eq!(partial_committed.library.tracks[0].title, "First Track");
    assert_eq!(
        partial_committed.sync_batch.additions.len(),
        1,
        "expected remaining 1 track to stay in SyncBatch"
    );
    assert_eq!(
        partial_committed.sync_batch.additions[0].host_file.title,
        "Second Track"
    );
}

#[test]
fn recovers_from_first_track_io_error_with_rollback() {
    let device_temp = TempDir::new().expect("create device temp dir");
    let mount_path = device_temp.path().to_path_buf();
    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 25_000_000_000,
            usb_firewire_guid: Some("000A270055667788".to_string()),
        }],
    };
    let mut engine = PodLoaderEngine::new(adapter);
    engine
        .initialize_device(&mount_path, "Classic")
        .expect("init device");

    let host_temp = TempDir::new().expect("create host temp");
    let t1_path = host_temp.path().join("01 - Transient Song.mp3");
    fs::write(
        &t1_path,
        build_id3v23_mp3_fixture("Transient Song", "Artist A", "Album A", 1, "Rock", 2001, 3),
    )
    .expect("write t1");

    engine
        .stage_host_paths(&[t1_path.clone()], None)
        .expect("stage track");

    // Delete source file before committing to trigger an I/O error on track 1
    fs::remove_file(&t1_path).expect("remove source file");

    let outcome = engine.commit_sync_batch().expect("commit with I/O error");
    match outcome {
        CommitOutcome::Interrupted(summary) => {
            assert_eq!(
                summary.completed_track_count, 0,
                "first track failed so 0 completed"
            );
            assert_eq!(summary.remaining_track_count, 1);
            assert!(
                summary.reason.to_lowercase().contains("no such file")
                    || summary.reason.to_lowercase().contains("os error"),
                "reason should capture the I/O error: {}",
                summary.reason
            );
        }
        other => panic!("expected Interrupted outcome, got {:?}", other),
    }

    let rolled_back = engine
        .resolve_interrupted_batch(InterruptedResolution::RollBack)
        .expect("rollback interrupted batch");
    assert!(rolled_back.library.tracks.is_empty());
    assert_eq!(rolled_back.sync_batch.additions.len(), 1);
}

#[test]
fn updates_master_playlist_index_mhods_mhsd_type3_and_re_signs_extracted_hash72() {
    use podloader_core::signing::{
        compute_itunesdb_sha1_for_hash72, extract_hash72_params, generate_hash72,
    };

    let device_temp = TempDir::new().expect("create device temp");
    let mount_path = device_temp.path().to_path_buf();
    fs::create_dir_all(mount_path.join("iPod_Control").join("iTunes")).expect("create iTunes dir");
    fs::create_dir_all(mount_path.join("iPod_Control").join("Device")).expect("create Device dir");
    for idx in 0..50 {
        fs::create_dir_all(
            mount_path
                .join("iPod_Control")
                .join("Music")
                .join(format!("F{:02}", idx)),
        )
        .expect("create Fxx");
    }

    // Build a database with two 624-byte mhits ("After Dark" at index 0 and "Zebra" at index 1),
    // mhsd type 3 + mhsd type 2 Master Playlists containing mhod 52 & 53,
    // and a valid hash72 signature (without setting FireWireGuid in SysInfo).
    let expand_mhit_to_624 = |mhit: Vec<u8>| -> Vec<u8> {
        let old_hdr_len = u32::from_le_bytes([mhit[4], mhit[5], mhit[6], mhit[7]]) as usize;
        let body_bytes = mhit[old_hdr_len..].to_vec();
        let mut hdr624 = vec![0u8; 624];
        hdr624[..old_hdr_len].copy_from_slice(&mhit[..old_hdr_len]);
        let new_total = (624 + body_bytes.len()) as u32;
        hdr624[4..8].copy_from_slice(&624u32.to_le_bytes());
        hdr624[8..12].copy_from_slice(&new_total.to_le_bytes());
        hdr624[208..212].copy_from_slice(&1u32.to_le_bytes()); // media_type = Audio
        hdr624.extend_from_slice(&body_bytes);
        hdr624
    };

    let mhit_a = expand_mhit_to_624(build_mhit_fixture(
        101,
        "After Dark",
        "Artist A",
        "Album A",
        "Rock",
        ":iPod_Control:Music:F00:EXSA.mp3",
        1,
        1,
        2024,
        200_000,
        320,
        0,
        0,
        vec![],
    ));
    let mhit_z = expand_mhit_to_624(build_mhit_fixture(
        102,
        "Zebra",
        "Artist Z",
        "Album Z",
        "Rock",
        ":iPod_Control:Music:F00:EXSZ.mp3",
        2,
        1,
        2024,
        200_000,
        320,
        0,
        0,
        vec![],
    ));

    let mut mhod52 = vec![0u8; 80];
    mhod52[0..4].copy_from_slice(b"mhod");
    mhod52[4..8].copy_from_slice(&24u32.to_le_bytes());
    mhod52[8..12].copy_from_slice(&80u32.to_le_bytes());
    mhod52[12..16].copy_from_slice(&52u32.to_le_bytes());
    mhod52[24..28].copy_from_slice(&3u32.to_le_bytes()); // sort_type = 3 (Title)
    mhod52[28..32].copy_from_slice(&2u32.to_le_bytes()); // count = 2
    mhod52[72..76].copy_from_slice(&0u32.to_le_bytes()); // index 0 ("After Dark")
    mhod52[76..80].copy_from_slice(&1u32.to_le_bytes()); // index 1 ("Zebra")

    let mut mhod53 = vec![0u8; 64];
    mhod53[0..4].copy_from_slice(b"mhod");
    mhod53[4..8].copy_from_slice(&24u32.to_le_bytes());
    mhod53[8..12].copy_from_slice(&64u32.to_le_bytes());
    mhod53[12..16].copy_from_slice(&53u32.to_le_bytes());
    mhod53[24..28].copy_from_slice(&3u32.to_le_bytes());
    mhod53[28..32].copy_from_slice(&2u32.to_le_bytes()); // 2 entries ('A' and 'Z')
    // Entry 0: 'A' (0x0041), start = 0, count = 1
    mhod53[40..42].copy_from_slice(&(b'A' as u16).to_le_bytes());
    mhod53[44..48].copy_from_slice(&0u32.to_le_bytes());
    mhod53[48..52].copy_from_slice(&1u32.to_le_bytes());
    // Entry 1: 'Z' (0x005A), start = 1, count = 1
    mhod53[52..54].copy_from_slice(&(b'Z' as u16).to_le_bytes());
    mhod53[56..60].copy_from_slice(&1u32.to_le_bytes());
    mhod53[60..64].copy_from_slice(&1u32.to_le_bytes());

    let master_mhyp =
        build_mhyp_fixture(1000, "My iPod", true, vec![mhod52, mhod53], &[101, 102]);

    let mut mhlt = vec![0u8; 92];
    mhlt[0..4].copy_from_slice(b"mhlt");
    mhlt[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlt[8..12].copy_from_slice(&2u32.to_le_bytes());
    mhlt.extend_from_slice(&mhit_a);
    mhlt.extend_from_slice(&mhit_z);

    let mut mhsd_tracks = vec![0u8; 96];
    mhsd_tracks[0..4].copy_from_slice(b"mhsd");
    mhsd_tracks[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_tracks[8..12].copy_from_slice(&((96 + mhlt.len()) as u32).to_le_bytes());
    mhsd_tracks[12..16].copy_from_slice(&1u32.to_le_bytes());
    mhsd_tracks.extend_from_slice(&mhlt);

    let mut mhlp = vec![0u8; 92];
    mhlp[0..4].copy_from_slice(b"mhlp");
    mhlp[4..8].copy_from_slice(&92u32.to_le_bytes());
    mhlp[8..12].copy_from_slice(&1u32.to_le_bytes());
    mhlp.extend_from_slice(&master_mhyp);

    let mut mhsd_playlists_type3 = vec![0u8; 96];
    mhsd_playlists_type3[0..4].copy_from_slice(b"mhsd");
    mhsd_playlists_type3[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_playlists_type3[8..12].copy_from_slice(&((96 + mhlp.len()) as u32).to_le_bytes());
    mhsd_playlists_type3[12..16].copy_from_slice(&3u32.to_le_bytes());
    mhsd_playlists_type3.extend_from_slice(&mhlp);

    let mut mhsd_playlists_type2 = vec![0u8; 96];
    mhsd_playlists_type2[0..4].copy_from_slice(b"mhsd");
    mhsd_playlists_type2[4..8].copy_from_slice(&96u32.to_le_bytes());
    mhsd_playlists_type2[8..12].copy_from_slice(&((96 + mhlp.len()) as u32).to_le_bytes());
    mhsd_playlists_type2[12..16].copy_from_slice(&2u32.to_le_bytes());
    mhsd_playlists_type2.extend_from_slice(&mhlp);

    let mhbd_header_len = 244u32;
    let total_len = mhbd_header_len
        + (mhsd_tracks.len() + mhsd_playlists_type3.len() + mhsd_playlists_type2.len()) as u32;
    let mut db_bytes = vec![0u8; mhbd_header_len as usize];
    db_bytes[0..4].copy_from_slice(b"mhbd");
    db_bytes[4..8].copy_from_slice(&mhbd_header_len.to_le_bytes());
    db_bytes[8..12].copy_from_slice(&total_len.to_le_bytes());
    db_bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
    db_bytes[16..20].copy_from_slice(&0x73u32.to_le_bytes());
    db_bytes[20..24].copy_from_slice(&3u32.to_le_bytes());
    db_bytes.extend_from_slice(&mhsd_tracks);
    db_bytes.extend_from_slice(&mhsd_playlists_type3);
    db_bytes.extend_from_slice(&mhsd_playlists_type2);

    let iv = [0x26u8; 16];
    let rand12 = [0xD5u8; 12];
    let sha1 = compute_itunesdb_sha1_for_hash72(&db_bytes);
    let sig72 = generate_hash72(&sha1, &iv, &rand12);
    db_bytes[0x72..0x72 + 46].copy_from_slice(&sig72);
    assert_eq!(extract_hash72_params(&db_bytes), Some((iv, rand12)));

    let itunesdb_path = mount_path
        .join("iPod_Control")
        .join("iTunes")
        .join("iTunesDB");
    fs::write(&itunesdb_path, &db_bytes).expect("write initial iTunesDB");

    let host_temp = TempDir::new().expect("create host temp");
    let mp3_path = host_temp.path().join("drunken_sailor.mp3");
    fs::write(
        &mp3_path,
        build_id3v23_mp3_fixture(
            "drunken_sailor",
            "Sea Shanties",
            "Sailing Songs",
            3,
            "Folk",
            2025,
            4,
        ),
    )
    .expect("write mp3");

    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 160_000_000_000,
            free_bytes: 100_000_000_000,
            usb_firewire_guid: None,
        }],
    };
    let mut engine = PodLoaderEngine::new(adapter);
    engine.stage_host_paths(&[mp3_path], None).expect("stage");
    let outcome = engine.commit_sync_batch().expect("commit");
    match outcome {
        CommitOutcome::Completed(session) => {
            assert_eq!(session.library.tracks.len(), 3);
            // UI playlists should not be duplicated even though both mhsd type 3 and type 2 exist
            assert_eq!(session.library.playlists.len(), 1);
            assert_eq!(session.library.playlists[0].track_ids.len(), 3);
        }
        other => panic!("expected Completed, got {:?}", other),
    }

    let updated_bytes = fs::read(&itunesdb_path).expect("read updated iTunesDB");
    // Verify hash72 was re-signed with the extracted (iv, rand12) even without FireWireGuid!
    assert_eq!(
        extract_hash72_params(&updated_bytes),
        Some((iv, rand12)),
        "expected updated iTunesDB to have a valid re-signed hash72"
    );

    // Verify mhod 52 sorted "drunken_sailor" (mhlt index 2) between "After Dark" (0) and "Zebra" (1) -> [0, 2, 1],
    // and mhod 53 jump table now has 3 entries: 'A' (0, 1), 'D' (1, 1), 'Z' (2, 1).
    let mut found_mhod52 = 0usize;
    let mut found_mhod53 = 0usize;
    for pos in 0..updated_bytes.len().saturating_sub(76) {
        if &updated_bytes[pos..pos + 4] == b"mhod"
            && u32::from_le_bytes(updated_bytes[pos + 4..pos + 8].try_into().unwrap()) == 24
        {
            let mtype = u32::from_le_bytes(updated_bytes[pos + 12..pos + 16].try_into().unwrap());
            if mtype == 52 {
                found_mhod52 += 1;
                let idx0 =
                    u32::from_le_bytes(updated_bytes[pos + 72..pos + 76].try_into().unwrap());
                let idx1 =
                    u32::from_le_bytes(updated_bytes[pos + 76..pos + 80].try_into().unwrap());
                let idx2 =
                    u32::from_le_bytes(updated_bytes[pos + 80..pos + 84].try_into().unwrap());
                assert_eq!(
                    [idx0, idx1, idx2],
                    [0, 2, 1],
                    "expected 'drunken_sailor' (idx 2) to be sorted between 'After Dark' (0) and 'Zebra' (1)"
                );
            } else if mtype == 53 {
                found_mhod53 += 1;
                let entry_cnt =
                    u32::from_le_bytes(updated_bytes[pos + 28..pos + 32].try_into().unwrap());
                assert_eq!(entry_cnt, 3);
                let ch_d =
                    u16::from_le_bytes(updated_bytes[pos + 52..pos + 54].try_into().unwrap());
                let start_d =
                    u32::from_le_bytes(updated_bytes[pos + 56..pos + 60].try_into().unwrap());
                let count_d =
                    u32::from_le_bytes(updated_bytes[pos + 60..pos + 64].try_into().unwrap());
                assert_eq!((ch_d, start_d, count_d), (b'D' as u16, 1, 1));
                let ch_z =
                    u16::from_le_bytes(updated_bytes[pos + 64..pos + 66].try_into().unwrap());
                let start_z =
                    u32::from_le_bytes(updated_bytes[pos + 68..pos + 72].try_into().unwrap());
                let count_z =
                    u32::from_le_bytes(updated_bytes[pos + 72..pos + 76].try_into().unwrap());
                assert_eq!((ch_z, start_z, count_z), (b'Z' as u16, 2, 1));
            }
        }
    }
    assert_eq!(found_mhod52, 2, "expected mhod 52 in both mhsd type 3 and type 2");
    assert_eq!(found_mhod53, 2, "expected mhod 53 in both mhsd type 3 and type 2");
}

#[test]
fn stages_flac_ogg_and_opus_files_marked_for_alac_transcoding_while_passing_through_native_formats() {
    let device_temp = TempDir::new().expect("create device temp dir");
    let mount_path = device_temp.path().to_path_buf();

    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 29_950_000_000,
            usb_firewire_guid: None,
        }],
    };
    let mut engine = PodLoaderEngine::new(adapter);
    let state = engine
        .initialize_device(&mount_path, "My iPod")
        .expect("initialize device");
    match state {
        DeviceConnectionState::Connected(session) => {
            assert_eq!(session.transcode_preset, TranscodePreset::Alac);
        }
        other => panic!("expected Connected, got {:?}", other),
    }

    let host_temp = TempDir::new().expect("create host temp dir");
    let flac_path = host_temp.path().join("01 - Everything In Its Right Place.flac");
    let ogg_path = host_temp.path().join("02 - Kid A.ogg");
    let opus_path = host_temp.path().join("03 - The National Anthem.opus");
    let mp3_path = host_temp.path().join("04 - How to Disappear Completely.mp3");
    let wav_path = host_temp.path().join("05 - Treefingers.wav");

    fs::write(&flac_path, b"fLaC\x00\x00\x00\x22").expect("write flac stub");
    fs::write(&ogg_path, b"OggS\x00\x02").expect("write ogg stub");
    fs::write(&opus_path, b"OggS\x00\x02OpusHead").expect("write opus stub");
    fs::write(
        &mp3_path,
        build_id3v23_mp3_fixture(
            "How to Disappear Completely",
            "Radiohead",
            "Kid A",
            4,
            "Electronic",
            2000,
            4,
        ),
    )
    .expect("write mp3");
    fs::write(&wav_path, build_wav_fixture(44_100, 2)).expect("write wav");

    let listing = engine
        .browse_host_directory(host_temp.path())
        .expect("browse host directory");
    assert_eq!(listing.files.len(), 5);

    let batch = engine
        .stage_host_paths(
            &[flac_path, ogg_path, opus_path, mp3_path, wav_path],
            None,
        )
        .expect("stage host paths");

    assert_eq!(batch.additions.len(), 5);

    // FLAC, OGG, Opus are marked for transcoding with active preset (Alac)
    assert_eq!(batch.additions[0].host_file.format, "FLAC");
    assert!(batch.additions[0].requires_transcode);
    assert_eq!(
        batch.additions[0].target_format,
        Some(TranscodePreset::Alac)
    );

    assert_eq!(batch.additions[1].host_file.format, "OGG");
    assert!(batch.additions[1].requires_transcode);
    assert_eq!(
        batch.additions[1].target_format,
        Some(TranscodePreset::Alac)
    );

    assert_eq!(batch.additions[2].host_file.format, "OPUS");
    assert!(batch.additions[2].requires_transcode);
    assert_eq!(
        batch.additions[2].target_format,
        Some(TranscodePreset::Alac)
    );

    // Native MP3 and WAV pass through untouched
    assert_eq!(batch.additions[3].host_file.format, "MP3");
    assert!(!batch.additions[3].requires_transcode);
    assert_eq!(batch.additions[3].target_format, None);

    assert_eq!(batch.additions[4].host_file.format, "WAV");
    assert!(!batch.additions[4].requires_transcode);
    assert_eq!(batch.additions[4].target_format, None);
}

fn crc8_flac(data: &[u8]) -> u8 {
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

fn crc16_flac(data: &[u8]) -> u16 {
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

fn crc32_ogg(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            if (crc & 0x8000_0000) != 0 {
                crc = (crc << 1) ^ 0x04C1_1DB7;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

fn build_vorbis_comment_payload(comments: &[(&str, &str)]) -> Vec<u8> {
    let vendor = b"PodLoaderTest";
    let mut payload = Vec::new();
    payload.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    payload.extend_from_slice(vendor);
    payload.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for (k, v) in comments {
        let entry = format!("{}={}", k, v);
        payload.extend_from_slice(&(entry.len() as u32).to_le_bytes());
        payload.extend_from_slice(entry.as_bytes());
    }
    payload
}

fn build_flac_picture_payload(mime: &str, data: &[u8]) -> Vec<u8> {
    let mut pic = Vec::new();
    pic.extend_from_slice(&3u32.to_be_bytes()); // Front cover
    pic.extend_from_slice(&(mime.len() as u32).to_be_bytes());
    pic.extend_from_slice(mime.as_bytes());
    pic.extend_from_slice(&0u32.to_be_bytes()); // description len
    pic.extend_from_slice(&64u32.to_be_bytes()); // width
    pic.extend_from_slice(&64u32.to_be_bytes()); // height
    pic.extend_from_slice(&24u32.to_be_bytes()); // color depth
    pic.extend_from_slice(&0u32.to_be_bytes()); // indexed colors
    pic.extend_from_slice(&(data.len() as u32).to_be_bytes());
    pic.extend_from_slice(data);
    pic
}

fn build_flac_streaminfo_bytes(sample_rate: u32, channels: u8, bits_per_sample: u8, total_samples: u64) -> [u8; 34] {
    let mut si = [0u8; 34];
    let block_size = 4096u16;
    si[0..2].copy_from_slice(&block_size.to_be_bytes());
    si[2..4].copy_from_slice(&block_size.to_be_bytes());
    // bytes 4..10: min/max frame size = 0
    let packed: u64 = ((sample_rate as u64 & 0xFFFFF) << 44)
        | ((((channels - 1) as u64) & 0x7) << 41)
        | ((((bits_per_sample - 1) as u64) & 0x1F) << 36)
        | (total_samples & 0xF_FFFF_FFFF);
    si[10..18].copy_from_slice(&packed.to_be_bytes());
    // bytes 18..34: MD5 = 0 (unset)
    si
}

fn build_flac_audio_frame(frame_number: u8, sample_rate: u32, sample_val_left: i16, sample_val_right: i16) -> Vec<u8> {
    // Fixed blocksize (sync = 0xFFF8), block_size_enc = 0xC (4096 samples),
    // sample_rate_enc = 0x9 (44100 Hz) or 0xA (48000 Hz),
    // channels_enc = 0x1 (2 channels: independent left/right),
    // bits_per_sample_enc = 0x4 (16 bits), reserved = 0 -> 0x08
    let sr_code: u8 = if sample_rate == 48_000 { 0xCA } else { 0xC9 };
    let mut frame = vec![0xFF, 0xF8, sr_code, 0x18, frame_number];
    let crc8 = crc8_flac(&frame);
    frame.push(crc8);

    // Subframe 0 (Left): Constant subframe (0b0_000000_0 = 0x00) + 16-bit big-endian sample
    frame.push(0x00);
    frame.extend_from_slice(&sample_val_left.to_be_bytes());
    // Subframe 1 (Right): Constant subframe (0x00) + 16-bit big-endian sample
    frame.push(0x00);
    frame.extend_from_slice(&sample_val_right.to_be_bytes());

    let crc16 = crc16_flac(&frame);
    frame.extend_from_slice(&crc16.to_be_bytes());
    frame
}

fn build_valid_flac_fixture(
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

fn wrap_ogg_page(
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

fn build_valid_ogg_fixture(
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

    // Packet 0: Ogg-FLAC identification packet (51 bytes)
    let mut id_pkt = Vec::with_capacity(51);
    id_pkt.push(0x7F);
    id_pkt.extend_from_slice(b"FLAC");
    id_pkt.push(1); // major
    id_pkt.push(0); // minor
    id_pkt.extend_from_slice(&2u16.to_be_bytes()); // 2 metadata packets follow
    id_pkt.extend_from_slice(b"fLaC");
    id_pkt.push(0x00); // StreamInfo block header
    id_pkt.extend_from_slice(&[0x00, 0x00, 34]);
    id_pkt.extend_from_slice(&si);

    // Packet 1: VorbisComment metadata block
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

    // Packet 2: Picture metadata block (last metadata block = 0x86)
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

fn build_valid_opus_fixture(
    title: &str,
    artist: &str,
    album: &str,
    track_number: u32,
    disc_number: u32,
    genre: &str,
    year: u32,
    num_packets: u32,
) -> Vec<u8> {
    let serial = 0xCAFE_BABEu32;
    // Packet 0: OpusHead (19 bytes)
    let mut head = Vec::with_capacity(19);
    head.extend_from_slice(b"OpusHead");
    head.push(1); // version 1
    head.push(2); // 2 channels
    head.extend_from_slice(&0u16.to_le_bytes()); // pre_skip = 0
    head.extend_from_slice(&48_000u32.to_le_bytes()); // input sample rate = 48000 Hz
    head.extend_from_slice(&0i16.to_le_bytes()); // output gain = 0
    head.push(0); // channel mapping family = 0

    // Packet 1: OpusTags
    let mut tags = Vec::new();
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&build_vorbis_comment_payload(&[
        ("TITLE", title),
        ("ARTIST", artist),
        ("ALBUM", album),
        ("TRACKNUMBER", &track_number.to_string()),
        ("DISCNUMBER", &disc_number.to_string()),
        ("GENRE", genre),
        ("DATE", &year.to_string()),
    ]));

    let mut opus = Vec::new();
    opus.extend_from_slice(&wrap_ogg_page(0x02, 0, serial, 0, &head));
    opus.extend_from_slice(&wrap_ogg_page(0x00, 0, serial, 1, &tags));

    // Each packet: config 15 (20ms = 960 samples at 48kHz), stereo (s=1), 1 frame (c=0): TOC byte = (15 << 3) | 0x04 = 0x7C
    for p in 0..num_packets {
        let is_last = p + 1 == num_packets;
        let hdr_type = if is_last { 0x04 } else { 0x00 };
        let granule = ((p + 1) as u64) * 960;
        let pkt = [0x7Cu8, 0xFF, 0xFE, (p as u8).wrapping_mul(17)];
        opus.extend_from_slice(&wrap_ogg_page(hdr_type, granule, serial, 2 + p, &pkt));
    }
    opus
}

#[test]
fn commits_flac_ogg_and_opus_tracks_via_pure_rust_alac_transcoder_preserving_metadata_artwork_and_itunesdb_fields() {
    let device_temp = TempDir::new().expect("create device temp dir");
    let mount_path = device_temp.path().to_path_buf();

    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 29_950_000_000,
            usb_firewire_guid: None,
        }],
    };
    let mut engine = PodLoaderEngine::new(adapter);
    engine
        .initialize_device(&mount_path, "Lossless iPod")
        .expect("initialize device");

    let host_temp = TempDir::new().expect("create host temp dir");
    let artwork_png: Vec<u8> = vec![
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xAA, 0xBB, 0xCC, 0xDD,
    ];

    // 1. 48 kHz stereo FLAC file (12 frames * 4096 = 49152 samples -> 1024 ms) with Vorbis comments + Picture block
    let flac_path = host_temp.path().join("01 - Everything In Its Right Place.flac");
    let flac_bytes = build_valid_flac_fixture(
        "Everything In Its Right Place",
        "Radiohead",
        "Kid A",
        1,
        2,
        "Electronic",
        2000,
        48_000,
        12,
        &artwork_png,
    );
    fs::write(&flac_path, &flac_bytes).expect("write flac fixture");

    // 2. 44.1 kHz stereo OGG file (11 frames * 4096 = 45056 samples -> ~1021 ms) with Vorbis comments + Picture block
    let ogg_path = host_temp.path().join("02 - Kid A.ogg");
    let ogg_bytes = build_valid_ogg_fixture(
        "Kid A",
        "Radiohead",
        "Kid A",
        2,
        2,
        "Electronic",
        2000,
        11,
        &artwork_png,
    );
    fs::write(&ogg_path, &ogg_bytes).expect("write ogg fixture");

    // 3. 48 kHz stereo Opus file (50 packets * 960 = 48000 samples -> 1000 ms) with OpusTags
    let opus_path = host_temp.path().join("03 - The National Anthem.opus");
    let opus_bytes = build_valid_opus_fixture(
        "The National Anthem",
        "Radiohead",
        "Kid A",
        3,
        2,
        "Post-Punk",
        2000,
        50,
    );
    fs::write(&opus_path, &opus_bytes).expect("write opus fixture");

    // Stage all 3 files and commit Sync Batch
    let batch = engine
        .stage_host_paths(&[flac_path, ogg_path, opus_path], None)
        .expect("stage lossless/ogg/opus files");
    assert_eq!(batch.additions.len(), 3);
    assert!(batch.additions.iter().all(|a| a.requires_transcode));

    let outcome = engine.commit_sync_batch().expect("commit sync batch");
    let session = match outcome {
        CommitOutcome::Completed(s) => s,
        other => panic!("expected Completed, got {:?}", other),
    };

    assert_eq!(session.library.tracks.len(), 3);

    let t1 = &session.library.tracks[0];
    assert_eq!(t1.title, "Everything In Its Right Place");
    assert_eq!(t1.artist, "Radiohead");
    assert_eq!(t1.album, "Kid A");
    assert_eq!(t1.track_number, 1);
    assert_eq!(t1.disc_number, 2);
    assert_eq!(t1.genre, "Electronic");
    assert_eq!(t1.year, 2000);
    assert_eq!(t1.duration_ms, 1024);
    assert!(t1.bitrate_kbps > 0);
    assert!(
        t1.obfuscated_path.ends_with(".m4a"),
        "expected transcoded ALAC track at .m4a obfuscated path, got {}",
        t1.obfuscated_path
    );

    let t2 = &session.library.tracks[1];
    assert_eq!(t2.title, "Kid A");
    assert_eq!(t2.artist, "Radiohead");
    assert_eq!(t2.album, "Kid A");
    assert_eq!(t2.track_number, 2);
    assert_eq!(t2.disc_number, 2);
    assert_eq!(t2.genre, "Electronic");
    assert_eq!(t2.year, 2000);
    assert_eq!(t2.duration_ms, 1021);
    assert!(t2.obfuscated_path.ends_with(".m4a"));

    let t3 = &session.library.tracks[2];
    assert_eq!(t3.title, "The National Anthem");
    assert_eq!(t3.artist, "Radiohead");
    assert_eq!(t3.album, "Kid A");
    assert_eq!(t3.track_number, 3);
    assert_eq!(t3.disc_number, 2);
    assert_eq!(t3.genre, "Post-Punk");
    assert_eq!(t3.year, 2000);
    assert_eq!(t3.duration_ms, 1000);
    assert!(t3.obfuscated_path.ends_with(".m4a"));

    // Verify the written .m4a files on the Device volume are valid ALAC containers with preserved tags and embedded artwork
    for (idx, dt) in session.library.tracks.iter().enumerate() {
        let rel = dt.obfuscated_path.trim_start_matches(':').replace(':', "/");
        let disk_path = mount_path.join(rel);
        assert!(disk_path.is_file(), "expected output .m4a file at {:?}", disk_path);
        let m4a_bytes = fs::read(&disk_path).expect("read output .m4a");
        assert!(
            m4a_bytes.windows(4).any(|w| w == b"alac"),
            "expected alac sample entry in .m4a container"
        );
        // Inspecting the output .m4a through browse_host_directory must read back all copied metadata tags
        let parent_dir = disk_path.parent().expect("music folder");
        let inspected = engine
            .browse_host_directory(parent_dir)
            .expect("browse output folder");
        let matched = inspected
            .files
            .iter()
            .find(|f| f.path == disk_path)
            .expect("find output .m4a in directory listing");
        assert_eq!(matched.format, "ALAC");
        assert_eq!(matched.title, dt.title);
        assert_eq!(matched.artist, dt.artist);
        assert_eq!(matched.album, dt.album);
        assert_eq!(matched.track_number, dt.track_number);
        assert_eq!(matched.disc_number, dt.disc_number);
        assert_eq!(matched.genre, dt.genre);
        assert_eq!(matched.year, dt.year);

        if idx < 2 {
            // Tracks 1 and 2 had embedded PNG artwork; verify covr atom contains the artwork bytes
            assert!(
                m4a_bytes.windows(4).any(|w| w == b"covr"),
                "expected covr artwork atom in transcoded .m4a"
            );
            assert!(
                m4a_bytes
                    .windows(artwork_png.len())
                    .any(|w| w == artwork_png.as_slice()),
                "expected embedded artwork payload preserved inside .m4a container"
            );
        }
    }

    // Verify iTunesDB mhit headers registered accurate sample rates (48000 << 16 for track 1, 44100 << 16 for track 2)
    let itunesdb_bytes = fs::read(mount_path.join("iPod_Control/iTunes/iTunesDB")).expect("read iTunesDB");
    let expected_sr_48k = (48_000u32 << 16).to_le_bytes();
    let expected_sr_44k = (44_100u32 << 16).to_le_bytes();
    assert!(
        itunesdb_bytes.windows(4).any(|w| w == expected_sr_48k),
        "expected 48 kHz fixed-point sample rate (48000 << 16) in iTunesDB mhit"
    );
    assert!(
        itunesdb_bytes.windows(4).any(|w| w == expected_sr_44k),
        "expected 44.1 kHz fixed-point sample rate (44100 << 16) in iTunesDB mhit"
    );
}

struct ConfigurableTranscodeAdapter {
    volumes: Vec<MountedVolume>,
    ffmpeg_available: bool,
}

impl HardwareDeviceAdapter for ConfigurableTranscodeAdapter {
    fn list_mounted_volumes(&self) -> Result<Vec<MountedVolume>, PodLoaderError> {
        Ok(self.volumes.clone())
    }

    fn lookup_usb_firewire_guid(&self, mount_path: &std::path::Path) -> Option<String> {
        self.volumes
            .iter()
            .find(|v| v.mount_path == mount_path)
            .and_then(|v| v.usb_firewire_guid.clone())
    }

    fn is_ffmpeg_available(&self) -> bool {
        self.ffmpeg_available
    }

    fn transcode_lossy(
        &self,
        _input_path: &std::path::Path,
        output_path: &std::path::Path,
        preset: TranscodePreset,
        host_file: &podloader_core::HostFile,
    ) -> Result<(), PodLoaderError> {
        if !self.ffmpeg_available {
            return Err(PodLoaderError::FfmpegUnavailable);
        }
        let duration_secs = (host_file.duration_ms / 1000).max(1);
        let bytes = match preset {
            TranscodePreset::Mp3 => build_id3v23_mp3_fixture(
                &host_file.title,
                &host_file.artist,
                &host_file.album,
                host_file.track_number,
                &host_file.genre,
                host_file.year,
                duration_secs,
            ),
            TranscodePreset::Aac | TranscodePreset::Alac => build_m4a_alac_fixture(
                &host_file.title,
                &host_file.artist,
                &host_file.album,
                host_file.track_number as u16,
                duration_secs,
            ),
        };
        fs::write(output_path, bytes)?;
        Ok(())
    }
}

#[test]
fn configures_per_device_transcode_presets_enforces_ffmpeg_availability_and_emits_commit_progress() {
    let device_temp = TempDir::new().expect("create device temp dir");
    let mount_path = device_temp.path().to_path_buf();

    // 1. When ffmpeg is absent, lossy presets (AAC / MP3) are rejected while ALAC succeeds
    let no_ffmpeg_adapter = ConfigurableTranscodeAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 29_950_000_000,
            usb_firewire_guid: None,
        }],
        ffmpeg_available: false,
    };
    let mut engine_no_ffmpeg = PodLoaderEngine::new(no_ffmpeg_adapter);
    let init_state = engine_no_ffmpeg
        .initialize_device(&mount_path, "Preset iPod")
        .expect("init device");
    match init_state {
        DeviceConnectionState::Connected(s) => {
            assert!(!s.ffmpeg_available);
            assert_eq!(s.transcode_preset, TranscodePreset::Alac);
        }
        other => panic!("expected Connected, got {:?}", other),
    }

    let err = engine_no_ffmpeg
        .set_transcode_preset(TranscodePreset::Aac)
        .expect_err("expected FfmpegUnavailable when selecting AAC without ffmpeg");
    assert!(matches!(err, PodLoaderError::FfmpegUnavailable));

    // 2. When ffmpeg is available, switching to MP3 updates staged transcodable tracks and commits with progress
    let with_ffmpeg_adapter = ConfigurableTranscodeAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 29_950_000_000,
            usb_firewire_guid: None,
        }],
        ffmpeg_available: true,
    };
    let mut engine = PodLoaderEngine::new(with_ffmpeg_adapter);
    engine.scan_device().expect("scan device");

    let host_temp = TempDir::new().expect("create host temp dir");
    let flac_path = host_temp.path().join("01 - Airbag.flac");
    let flac_bytes = build_valid_flac_fixture(
        "Airbag",
        "Radiohead",
        "OK Computer",
        1,
        1,
        "Alternative",
        1997,
        44_100,
        10,
        &[],
    );
    fs::write(&flac_path, &flac_bytes).expect("write flac");

    let mp3_path = host_temp.path().join("02 - Paranoid Android.mp3");
    fs::write(
        &mp3_path,
        build_id3v23_mp3_fixture(
            "Paranoid Android",
            "Radiohead",
            "OK Computer",
            2,
            "Alternative",
            1997,
            4,
        ),
    )
    .expect("write mp3");

    let batch = engine
        .stage_host_paths(&[flac_path, mp3_path], None)
        .expect("stage tracks");
    assert_eq!(batch.additions[0].target_format, Some(TranscodePreset::Alac));
    assert_eq!(batch.additions[1].target_format, None);

    // Switch device preset to MP3 -> staged FLAC track updates to Some(TranscodePreset::Mp3), native MP3 stays None
    let updated_session = engine
        .set_transcode_preset(TranscodePreset::Mp3)
        .expect("set MP3 preset");
    assert_eq!(updated_session.transcode_preset, TranscodePreset::Mp3);
    assert!(updated_session.ffmpeg_available);
    assert_eq!(
        updated_session.sync_batch.additions[0].target_format,
        Some(TranscodePreset::Mp3)
    );
    assert_eq!(updated_session.sync_batch.additions[1].target_format, None);

    // Commit with progress callback and verify per-track & overall progress events
    let mut progress_events: Vec<TransferProgress> = Vec::new();
    let outcome = engine
        .commit_sync_batch_with_progress(None, |prog| {
            progress_events.push(prog);
        })
        .expect("commit with progress");

    let final_session = match outcome {
        CommitOutcome::Completed(s) => s,
        other => panic!("expected Completed, got {:?}", other),
    };
    assert_eq!(final_session.library.tracks.len(), 2);
    assert!(
        final_session.library.tracks[0]
            .obfuscated_path
            .ends_with(".mp3"),
        "expected FLAC track transcoded to .mp3 under MP3 preset, got {}",
        final_session.library.tracks[0].obfuscated_path
    );

    assert!(
        !progress_events.is_empty(),
        "expected progress events during commit"
    );
    assert!(progress_events
        .iter()
        .any(|p| p.current_track_title == "Airbag" && p.is_transcoding));
    assert!(progress_events
        .iter()
        .any(|p| p.current_track_title == "Paranoid Android" && !p.is_transcoding));
    let last_event = progress_events.last().expect("last progress event");
    assert_eq!(last_event.total_tracks, 2);
    assert_eq!(last_event.track_percent, 100);
    assert_eq!(last_event.overall_percent, 100);
}

#[test]
fn stages_and_commits_device_track_metadata_edits_updating_itunesdb_and_audio_tags() {
    let device_temp = TempDir::new().expect("create device temp dir");
    let mount_path = device_temp.path().to_path_buf();

    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 29_950_000_000,
            usb_firewire_guid: None,
        }],
    };
    let mut engine = PodLoaderEngine::new(adapter);
    engine
        .initialize_device(&mount_path, "Edit iPod")
        .expect("init device");

    let host_temp = TempDir::new().expect("create host temp dir");
    let mp3_path = host_temp.path().join("01 - Airbag.mp3");
    fs::write(
        &mp3_path,
        build_id3v23_mp3_fixture(
            "Airbag",
            "Radiohead",
            "OK Computer",
            1,
            "Alternative",
            1997,
            4,
        ),
    )
    .expect("write mp3");

    let m4a_path = host_temp.path().join("02 - Paranoid Android.m4a");
    fs::write(
        &m4a_path,
        build_m4a_alac_fixture("Paranoid Android", "Radiohead", "OK Computer", 2, 4),
    )
    .expect("write m4a");

    // Stage and commit initial tracks
    engine
        .stage_host_paths(&[mp3_path, m4a_path], None)
        .expect("stage tracks");
    let outcome = engine.commit_sync_batch().expect("commit initial tracks");
    let session = match outcome {
        CommitOutcome::Completed(s) => s,
        other => panic!("expected Completed, got {:?}", other),
    };
    assert_eq!(session.library.tracks.len(), 2);
    let track1_id = session.library.tracks[0].id;
    let track2_id = session.library.tracks[1].id;
    let track1_rel = session.library.tracks[0]
        .obfuscated_path
        .trim_start_matches(':')
        .replace(':', "/");
    let track1_disk_path = mount_path.join(track1_rel);
    let track2_rel = session.library.tracks[1]
        .obfuscated_path
        .trim_start_matches(':')
        .replace(':', "/");
    let track2_disk_path = mount_path.join(track2_rel);

    // 1. Stage single-track edit on Track 1
    let single_edit = podloader_core::TrackMetadataEdits {
        title: Some("Airbag (Remastered)".to_string()),
        artist: Some("Radiohead".to_string()),
        album: Some("OK Computer (Collector Edition)".to_string()),
        album_artist: Some("Radiohead".to_string()),
        track_number: Some(1),
        disc_number: Some(1),
        genre: Some("Art Rock".to_string()),
        year: Some(2009),
        is_compilation: Some(true),
        rating: Some(5),
    };
    let batch = engine
        .stage_track_metadata_edits(&[track1_id], single_edit)
        .expect("stage track 1 edits");
    assert_eq!(batch.edits.len(), 1);
    assert_eq!(batch.edits[0].track_id, track1_id);
    assert_eq!(
        batch.edits[0].edits.title.as_deref(),
        Some("Airbag (Remastered)")
    );

    // 2. Stage batch edit across Track 1 and Track 2 (updating shared genre and album_artist)
    let batch_edit = podloader_core::TrackMetadataEdits {
        genre: Some("Progressive Rock".to_string()),
        album_artist: Some("Radiohead Band".to_string()),
        ..Default::default()
    };
    let batch = engine
        .stage_track_metadata_edits(&[track1_id, track2_id], batch_edit)
        .expect("stage batch edits");
    assert_eq!(batch.edits.len(), 2);

    // 3. Commit the metadata edits
    let commit_outcome = engine.commit_sync_batch().expect("commit metadata edits");
    let updated_session = match commit_outcome {
        CommitOutcome::Completed(s) => s,
        other => panic!("expected Completed, got {:?}", other),
    };

    let updated_t1 = updated_session
        .library
        .tracks
        .iter()
        .find(|t| t.id == track1_id)
        .expect("find track 1");
    assert_eq!(updated_t1.title, "Airbag (Remastered)");
    assert_eq!(updated_t1.album, "OK Computer (Collector Edition)");
    assert_eq!(updated_t1.genre, "Progressive Rock");
    assert_eq!(updated_t1.album_artist.as_deref(), Some("Radiohead Band"));
    assert_eq!(updated_t1.year, 2009);
    assert_eq!(updated_t1.rating, 5);
    assert!(updated_t1.is_compilation);

    let updated_t2 = updated_session
        .library
        .tracks
        .iter()
        .find(|t| t.id == track2_id)
        .expect("find track 2");
    assert_eq!(updated_t2.title, "Paranoid Android");
    assert_eq!(updated_t2.genre, "Progressive Rock");
    assert_eq!(updated_t2.album_artist.as_deref(), Some("Radiohead Band"));

    // 4. Verify binary iTunesDB has updated mhit and mhod fields
    let itunesdb_bytes = fs::read(mount_path.join("iPod_Control/iTunes/iTunesDB")).expect("read iTunesDB");
    // Rating 5 -> raw byte 100 at mhit offset 30
    assert!(
        itunesdb_bytes.windows(2).any(|w| w == [100, 1]),
        "expected rating 100 and compilation flag 1 in mhit header bytes"
    );

    // 5. Verify on-disk audio files had their embedded tags updated
    let inspected_mp3 = engine
        .browse_host_directory(track1_disk_path.parent().unwrap())
        .expect("browse folder");
    let disk_mp3 = inspected_mp3
        .files
        .iter()
        .find(|f| f.path == track1_disk_path)
        .expect("find mp3 on disk");
    assert_eq!(disk_mp3.title, "Airbag (Remastered)");
    assert_eq!(disk_mp3.album, "OK Computer (Collector Edition)");
    assert_eq!(disk_mp3.genre, "Progressive Rock");
    assert_eq!(disk_mp3.year, 2009);

    let inspected_m4a = engine
        .browse_host_directory(track2_disk_path.parent().unwrap())
        .expect("browse m4a folder");
    let disk_m4a = inspected_m4a
        .files
        .iter()
        .find(|f| f.path == track2_disk_path)
        .expect("find m4a on disk");
    assert_eq!(disk_m4a.genre, "Progressive Rock");
}

#[test]
fn stages_and_commits_track_removals_cleaning_up_playlists_and_deleting_audio_files() {
    let device_temp = TempDir::new().expect("create device temp dir");
    let mount_path = device_temp.path().to_path_buf();

    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 29_950_000_000,
            usb_firewire_guid: None,
        }],
    };
    let mut engine = PodLoaderEngine::new(adapter);
    engine
        .initialize_device(&mount_path, "Removal iPod")
        .expect("init device");

    let host_temp = TempDir::new().expect("create host temp dir");
    let track_a_path = host_temp.path().join("01 - Track A.mp3");
    let track_b_path = host_temp.path().join("02 - Track B.mp3");
    let track_c_path = host_temp.path().join("03 - Track C.mp3");

    fs::write(
        &track_a_path,
        build_id3v23_mp3_fixture("Track A", "Artist A", "Album", 1, "Rock", 2020, 3),
    )
    .expect("write a");
    fs::write(
        &track_b_path,
        build_id3v23_mp3_fixture("Track B", "Artist B", "Album", 2, "Rock", 2020, 3),
    )
    .expect("write b");
    fs::write(
        &track_c_path,
        build_id3v23_mp3_fixture("Track C", "Artist C", "Album", 3, "Rock", 2020, 3),
    )
    .expect("write c");

    // Stage and commit 3 tracks
    engine
        .stage_host_paths(&[track_a_path, track_b_path, track_c_path], None)
        .expect("stage tracks");
    let outcome = engine.commit_sync_batch().expect("commit tracks");
    let session = match outcome {
        CommitOutcome::Completed(s) => s,
        other => panic!("expected Completed, got {:?}", other),
    };
    assert_eq!(session.library.tracks.len(), 3);
    let track_b_id = session.library.tracks[1].id;
    let track_b_rel = session.library.tracks[1]
        .obfuscated_path
        .trim_start_matches(':')
        .replace(':', "/");
    let track_b_disk = mount_path.join(track_b_rel);
    assert!(track_b_disk.is_file(), "expected track B file to exist on disk before removal");

    // 1. Stage removal of Track B
    let batch = engine
        .stage_track_removals(&[track_b_id])
        .expect("stage removal of track b");
    assert_eq!(batch.removals.len(), 1);
    assert_eq!(batch.removals[0].track_id, track_b_id);
    assert_eq!(batch.removals[0].track_title, "Track B");

    // 2. Commit the removal
    let outcome = engine.commit_sync_batch().expect("commit removal");
    let updated_session = match outcome {
        CommitOutcome::Completed(s) => s,
        other => panic!("expected Completed, got {:?}", other),
    };

    // 3. Verify Track B is completely gone from Device Library tracks and Master Playlist
    assert_eq!(updated_session.library.tracks.len(), 2);
    assert!(
        !updated_session.library.tracks.iter().any(|t| t.id == track_b_id),
        "expected Track B to be removed from Device Library tracks"
    );
    let master_pl = updated_session
        .library
        .playlists
        .iter()
        .find(|p| p.kind == PlaylistKind::Master)
        .expect("find master playlist");
    assert_eq!(master_pl.track_ids.len(), 2);
    assert!(
        !master_pl.track_ids.contains(&track_b_id),
        "expected Track B to be removed from Master Playlist"
    );

    // 4. Verify Track B file was deleted from disk
    assert!(
        !track_b_disk.exists(),
        "expected Track B file {:?} to be deleted from disk",
        track_b_disk
    );
}

#[test]
fn manages_device_playlists_create_rename_delete_add_remove_and_reorder() {
    let device_temp = TempDir::new().expect("create device temp dir");
    let mount_path = device_temp.path().to_path_buf();

    let adapter = FakeHardwareAdapter {
        volumes: vec![MountedVolume {
            mount_path: mount_path.clone(),
            volume_name: "IPOD".to_string(),
            total_bytes: 30_000_000_000,
            free_bytes: 29_950_000_000,
            usb_firewire_guid: None,
        }],
    };
    let mut engine = PodLoaderEngine::new(adapter);
    engine
        .initialize_device(&mount_path, "Playlist iPod")
        .expect("init device");

    let host_temp = TempDir::new().expect("create host temp dir");
    let t1 = host_temp.path().join("01 - Song 1.mp3");
    let t2 = host_temp.path().join("02 - Song 2.mp3");
    let t3 = host_temp.path().join("03 - Song 3.mp3");
    fs::write(&t1, build_id3v23_mp3_fixture("Song 1", "Artist", "Album", 1, "Pop", 2021, 3)).unwrap();
    fs::write(&t2, build_id3v23_mp3_fixture("Song 2", "Artist", "Album", 2, "Pop", 2021, 3)).unwrap();
    fs::write(&t3, build_id3v23_mp3_fixture("Song 3", "Artist", "Album", 3, "Pop", 2021, 3)).unwrap();

    engine.stage_host_paths(&[t1, t2, t3], None).unwrap();
    let outcome = engine.commit_sync_batch().unwrap();
    let session = match outcome {
        CommitOutcome::Completed(s) => s,
        other => panic!("expected Completed, got {:?}", other),
    };
    let id1 = session.library.tracks[0].id;
    let id2 = session.library.tracks[1].id;
    let id3 = session.library.tracks[2].id;

    // 1. Create a new user Device Playlist
    let created_pl = engine
        .create_device_playlist("Road Trip")
        .expect("create playlist");
    assert_eq!(created_pl.name, "Road Trip");
    assert_eq!(created_pl.kind, PlaylistKind::User);
    assert!(created_pl.track_ids.is_empty());
    let pl_id = created_pl.id;

    // 2. Add tracks [id1, id2, id3] to the playlist
    let updated_pl = engine
        .add_tracks_to_playlist(pl_id, &[id1, id2, id3])
        .expect("add tracks to playlist");
    assert_eq!(updated_pl.track_ids, vec![id1, id2, id3]);

    // 3. Reorder tracks to [id3, id1, id2]
    let reordered_pl = engine
        .reorder_playlist_tracks(pl_id, &[id3, id1, id2])
        .expect("reorder playlist");
    assert_eq!(reordered_pl.track_ids, vec![id3, id1, id2]);

    // 4. Remove track id1 from playlist -> [id3, id2]
    let filtered_pl = engine
        .remove_tracks_from_playlist(pl_id, &[id1])
        .expect("remove track from playlist");
    assert_eq!(filtered_pl.track_ids, vec![id3, id2]);

    // 5. Rename playlist to "Summer Road Trip"
    let renamed_pl = engine
        .rename_device_playlist(pl_id, "Summer Road Trip")
        .expect("rename playlist");
    assert_eq!(renamed_pl.name, "Summer Road Trip");

    // Verify iTunesDB contains the playlist on reload
    let state = engine.scan_device().expect("scan device");
    let current_session = match state {
        DeviceConnectionState::Connected(s) => s,
        other => panic!("expected Connected, got {:?}", other),
    };
    let found_pl = current_session
        .library
        .playlists
        .iter()
        .find(|p| p.id == pl_id)
        .expect("find user playlist in library");
    assert_eq!(found_pl.name, "Summer Road Trip");
    assert_eq!(found_pl.track_ids, vec![id3, id2]);

    // 6. Delete the playlist
    engine
        .delete_device_playlist(pl_id)
        .expect("delete playlist");
    let post_delete_state = engine.scan_device().expect("scan device");
    let post_delete_session = match post_delete_state {
        DeviceConnectionState::Connected(s) => s,
        other => panic!("expected Connected, got {:?}", other),
    };
    assert!(
        !post_delete_session.library.playlists.iter().any(|p| p.id == pl_id),
        "expected user playlist to be deleted"
    );
    // Tracks themselves must remain untouched in library
    assert_eq!(post_delete_session.library.tracks.len(), 3);
}









