use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use podloader_core::{
    DeviceConnectionState, HardwareDeviceAdapter, MountedVolume, PlaylistKind, PodLoaderEngine,
    PodLoaderError,
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


