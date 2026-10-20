use crate::sysinfo::normalize_firewire_guid;
use crate::PodLoaderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FireWireGuid(pub [u8; 8]);

impl FireWireGuid {
    pub fn parse(guid_str: &str) -> Result<Self, PodLoaderError> {
        let cleaned = normalize_firewire_guid(guid_str);
        if cleaned.len() != 16 || !cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(PodLoaderError::InvalidFireWireGuid(format!(
                "Expected 16 hex characters, got '{}'",
                guid_str
            )));
        }
        let mut out = [0u8; 8];
        for i in 0..8 {
            out[i] = u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16)
                .map_err(|_| PodLoaderError::InvalidFireWireGuid(guid_str.to_string()))?;
        }
        Ok(Self(out))
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(16);
        for b in &self.0 {
            s.push_str(&format!("{:02X}", b));
        }
        s
    }
}

pub fn parse_firewire_guid_bytes(guid_str: &str) -> Result<[u8; 8], PodLoaderError> {
    Ok(FireWireGuid::parse(guid_str)?.0)
}

/// Extract `(iv, random_bytes)` from an existing valid `iTunesDB` that contains a `hash72` signature
/// (`itdb_hash72.c` `hash_extract`). Allows re-signing `hash72` without needing a `FireWireGuid`.
pub fn extract_hash72_params(original_db_bytes: &[u8]) -> Option<([u8; 16], [u8; 12])> {
    if original_db_bytes.len() < 0xA0 || original_db_bytes.get(0..4) != Some(b"mhbd") {
        return None;
    }
    let sig = &original_db_bytes[0x72..0x72 + 46];
    if sig[0] != 0x01 || sig[1] != 0x00 {
        return None;
    }
    let sha1 = compute_itunesdb_sha1_for_hash72(original_db_bytes);
    let mut plaintext = [0u8; 32];
    plaintext[0..20].copy_from_slice(&sha1);
    plaintext[20..32].copy_from_slice(&sig[2..14]);

    let round_keys = aes128_expand_key(&HASH72_AES_KEY);
    let c0: [u8; 16] = sig[14..30].try_into().ok()?;
    let c1: [u8; 16] = sig[30..46].try_into().ok()?;

    let d0 = aes128_decrypt_block(&c0, &round_keys);
    let d1 = aes128_decrypt_block(&c1, &round_keys);

    let mut out1 = [0u8; 16];
    for i in 0..16 {
        out1[i] = d1[i] ^ c0[i];
    }
    if out1 != plaintext[16..32] {
        return None;
    }

    let mut iv = [0u8; 16];
    for i in 0..16 {
        iv[i] = d0[i] ^ plaintext[i];
    }
    let mut rand12 = [0u8; 12];
    rand12.copy_from_slice(&sig[2..14]);
    Some((iv, rand12))
}

/// Re-sign `hash72` at `0x72..0xA0` using `(iv, random_bytes)` extracted from the original `iTunesDB`.
pub fn resign_itunesdb_hash72_with_params(
    db_bytes: &mut [u8],
    iv: &[u8; 16],
    random_bytes: &[u8; 12],
) {
    if db_bytes.len() < 0xA0 || db_bytes.get(0..4) != Some(b"mhbd") {
        return;
    }
    let sha1 = compute_itunesdb_sha1_for_hash72(db_bytes);
    let hash72 = generate_hash72(&sha1, iv, random_bytes);
    db_bytes[0x72..0x72 + 46].copy_from_slice(&hash72);
}

pub fn sign_itunesdb(db_bytes: &mut [u8], firewire_guid_hex: &str) -> Result<(), PodLoaderError> {
    let guid = FireWireGuid::parse(firewire_guid_hex)?;
    if db_bytes.len() < 0xA0 || db_bytes.get(0..4) != Some(b"mhbd") {
        return Err(PodLoaderError::InvalidItunesDb(
            "mhbd header too short for hash58/hash72 signing".into(),
        ));
    }

    // 1. Extract existing hash72 params (if any) before mutating header fields
    let (iv, rand12) = extract_hash72_params(db_bytes).unwrap_or_else(|| {
        let mut r = [0u8; 12];
        r[0..8].copy_from_slice(&guid.0);
        r[8..12].copy_from_slice(&guid.0[0..4]);
        let mut i = [0u8; 16];
        i[0..8].copy_from_slice(&guid.0);
        i[8..16].copy_from_slice(&guid.0);
        (i, r)
    });

    // 2. Ensure hashing_scheme is non-zero before computing hash72 & hash58
    if db_bytes[0x30] == 0 && db_bytes[0x31] == 0 {
        db_bytes[0x30] = 1;
        db_bytes[0x31] = 0;
    }

    // 3. Write hash72 first so hash58 covers the final hash72 bytes at 0x72..0xA0
    let sha1_72 = compute_itunesdb_sha1_for_hash72(db_bytes);
    let hash72 = generate_hash72(&sha1_72, &iv, &rand12);
    db_bytes[0x72..0x72 + 46].copy_from_slice(&hash72);

    // 4. Compute and write hash58
    let hash58 = compute_hash58(db_bytes, &guid);
    db_bytes[0x58..0x58 + 20].copy_from_slice(&hash58);

    Ok(())
}

pub fn verify_itunesdb_signatures(db_bytes: &[u8], firewire_guid_hex: &str) -> bool {
    let Ok(guid) = FireWireGuid::parse(firewire_guid_hex) else {
        return false;
    };
    if db_bytes.len() < 0xA0 || db_bytes.get(0..4) != Some(b"mhbd") {
        return false;
    }
    let expected_58 = compute_hash58(db_bytes, &guid);
    let hash58_ok = db_bytes[0x58..0x58 + 20] == expected_58;
    let hash72_ok = extract_hash72_params(db_bytes).is_some();
    hash58_ok && hash72_ok
}

// Exact tables and constants from libgpod's itdb_hash58.c and itdb_hash72.c
const TABLE1: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const TABLE2: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

const HASH58_FIXED: [u8; 18] = [
    0x67, 0x23, 0xFE, 0x30, 0x45, 0x33, 0xF8, 0x90, 0x99, 0x21, 0x07, 0xC1, 0xD0, 0x12, 0xB2, 0xA1,
    0x07, 0x81,
];

const HASH72_AES_KEY: [u8; 16] = [
    0x61, 0x8C, 0xA1, 0x0D, 0xC7, 0xF5, 0x7F, 0xD3, 0xB4, 0x72, 0x3E, 0x08, 0x15, 0x74, 0x63, 0xD7,
];

fn gcd_i32(mut a: i32, mut b: i32) -> i32 {
    loop {
        a %= b;
        if a == 0 {
            return b;
        }
        b %= a;
        if b == 0 {
            return a;
        }
    }
}

fn lcm_i32(a: i32, b: i32) -> i32 {
    if a == 0 || b == 0 {
        return 1;
    }
    (a * b) / gcd_i32(a, b)
}

/// Exact port of `generate_key` from `libgpod/src/itdb_hash58.c`.
pub fn generate_hash58_key(firewire_id: &[u8; 8]) -> [u8; 64] {
    let mut y = [0u8; 16];
    for i in 0..4 {
        let a = firewire_id[i * 2] as i32;
        let b = firewire_id[i * 2 + 1] as i32;
        let cur_lcm = lcm_i32(a, b);
        let hi = ((cur_lcm & 0xFF00) >> 8) as usize;
        let lo = (cur_lcm & 0xFF) as usize;

        y[i * 4] = TABLE1[hi];
        y[i * 4 + 1] = TABLE2[hi];
        y[i * 4 + 2] = TABLE1[lo];
        y[i * 4 + 3] = TABLE2[lo];
    }

    let mut seed = Vec::with_capacity(34);
    seed.extend_from_slice(&HASH58_FIXED);
    seed.extend_from_slice(&y);
    let digest = sha1_digest(&seed);

    let mut key = [0u8; 64];
    key[0..20].copy_from_slice(&digest);
    key
}

/// Exact port of `itdb_hash58_write_hash` + `itdb_compute_hash` from `libgpod/src/itdb_hash58.c`.
pub fn compute_hash58(db_bytes: &[u8], guid: &FireWireGuid) -> [u8; 20] {
    let mut normalized = db_bytes.to_vec();
    normalized[0x18..0x20].fill(0); // db_id (8 bytes)
    normalized[0x32..0x46].fill(0); // unk_0x32 (20 bytes)
    normalized[0x58..0x6C].fill(0); // hash58 (20 bytes)

    let key = generate_hash58_key(&guid.0);
    let mut k_ipad = [0u8; 64];
    let mut k_opad = [0u8; 64];
    for i in 0..64 {
        k_ipad[i] = key[i] ^ 0x36;
        k_opad[i] = key[i] ^ 0x5C;
    }

    let mut inner_input = Vec::with_capacity(64 + normalized.len());
    inner_input.extend_from_slice(&k_ipad);
    inner_input.extend_from_slice(&normalized);
    let inner_hash = sha1_digest(&inner_input);

    let mut outer_input = Vec::with_capacity(84);
    outer_input.extend_from_slice(&k_opad);
    outer_input.extend_from_slice(&inner_hash);
    sha1_digest(&outer_input)
}

/// Exact port of `itdb_hash72_compute_itunesdb_sha1` from `libgpod/src/itdb_hash72.c`.
pub fn compute_itunesdb_sha1_for_hash72(db_bytes: &[u8]) -> [u8; 20] {
    let mut normalized = db_bytes.to_vec();
    normalized[0x18..0x20].fill(0); // db_id (8 bytes)
    normalized[0x32..0x46].fill(0); // unk_0x32 (20 bytes)
    normalized[0x58..0x6C].fill(0); // hash58 (20 bytes)
    normalized[0x72..0xA0].fill(0); // hash72 (46 bytes)
    sha1_digest(&normalized)
}

/// Exact port of `hash_generate` from `libgpod/src/itdb_hash72.c`.
pub fn generate_hash72(sha1: &[u8; 20], iv: &[u8; 16], random_bytes: &[u8; 12]) -> [u8; 46] {
    let mut plaintext = [0u8; 32];
    plaintext[0..20].copy_from_slice(sha1);
    plaintext[20..32].copy_from_slice(random_bytes);

    let ciphertext = aes128_cbc_encrypt(&plaintext, &HASH72_AES_KEY, iv);

    let mut signature = [0u8; 46];
    signature[0] = 0x01;
    signature[1] = 0x00;
    signature[2..14].copy_from_slice(random_bytes);
    signature[14..46].copy_from_slice(&ciphertext);
    signature
}

fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        if (b & 1) != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1B;
        }
        b >>= 1;
    }
    p
}

fn gmul2(x: u8) -> u8 {
    if (x & 0x80) != 0 {
        (x << 1) ^ 0x1B
    } else {
        x << 1
    }
}

fn aes128_expand_key(key: &[u8; 16]) -> [[u8; 16]; 11] {
    const RCON: [u8; 10] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1B, 0x36];
    let mut round_keys = [[0u8; 16]; 11];
    round_keys[0] = *key;

    for r in 1..=10 {
        let prev = round_keys[r - 1];
        let mut cur = [0u8; 16];
        let t = [
            TABLE1[prev[13] as usize] ^ RCON[r - 1],
            TABLE1[prev[14] as usize],
            TABLE1[prev[15] as usize],
            TABLE1[prev[12] as usize],
        ];
        for i in 0..4 {
            cur[i] = prev[i] ^ t[i];
        }
        for col in 1..4 {
            for row in 0..4 {
                let idx = col * 4 + row;
                cur[idx] = prev[idx] ^ cur[idx - 4];
            }
        }
        round_keys[r] = cur;
    }
    round_keys
}

fn aes128_encrypt_block(block: &[u8; 16], round_keys: &[[u8; 16]; 11]) -> [u8; 16] {
    let mut state = *block;
    for i in 0..16 {
        state[i] ^= round_keys[0][i];
    }

    for r in 1..=10 {
        for b in state.iter_mut() {
            *b = TABLE1[*b as usize];
        }
        let s = state;
        state[1] = s[5];
        state[5] = s[9];
        state[9] = s[13];
        state[13] = s[1];

        state[2] = s[10];
        state[6] = s[14];
        state[10] = s[2];
        state[14] = s[6];

        state[3] = s[15];
        state[7] = s[3];
        state[11] = s[7];
        state[15] = s[11];

        if r < 10 {
            for col in 0..4 {
                let c = col * 4;
                let a0 = state[c];
                let a1 = state[c + 1];
                let a2 = state[c + 2];
                let a3 = state[c + 3];
                let t = a0 ^ a1 ^ a2 ^ a3;
                state[c] ^= t ^ gmul2(a0 ^ a1);
                state[c + 1] ^= t ^ gmul2(a1 ^ a2);
                state[c + 2] ^= t ^ gmul2(a2 ^ a3);
                state[c + 3] ^= t ^ gmul2(a3 ^ a0);
            }
        }

        for i in 0..16 {
            state[i] ^= round_keys[r][i];
        }
    }
    state
}

fn aes128_decrypt_block(block: &[u8; 16], round_keys: &[[u8; 16]; 11]) -> [u8; 16] {
    let mut state = *block;
    for i in 0..16 {
        state[i] ^= round_keys[10][i];
    }

    for r in (0..10).rev() {
        let s = state;
        state[1] = s[13];
        state[5] = s[1];
        state[9] = s[5];
        state[13] = s[9];

        state[2] = s[10];
        state[6] = s[14];
        state[10] = s[2];
        state[14] = s[6];

        state[3] = s[7];
        state[7] = s[11];
        state[11] = s[15];
        state[15] = s[3];

        for b in state.iter_mut() {
            *b = TABLE2[*b as usize];
        }

        for i in 0..16 {
            state[i] ^= round_keys[r][i];
        }

        if r > 0 {
            for col in 0..4 {
                let c = col * 4;
                let a0 = state[c];
                let a1 = state[c + 1];
                let a2 = state[c + 2];
                let a3 = state[c + 3];
                state[c] = gmul(a0, 0x0E) ^ gmul(a1, 0x0B) ^ gmul(a2, 0x0D) ^ gmul(a3, 0x09);
                state[c + 1] = gmul(a0, 0x09) ^ gmul(a1, 0x0E) ^ gmul(a2, 0x0B) ^ gmul(a3, 0x0D);
                state[c + 2] = gmul(a0, 0x0D) ^ gmul(a1, 0x09) ^ gmul(a2, 0x0E) ^ gmul(a3, 0x0B);
                state[c + 3] = gmul(a0, 0x0B) ^ gmul(a1, 0x0D) ^ gmul(a2, 0x09) ^ gmul(a3, 0x0E);
            }
        }
    }
    state
}

fn aes128_cbc_encrypt(plaintext: &[u8; 32], key: &[u8; 16], iv: &[u8; 16]) -> [u8; 32] {
    let round_keys = aes128_expand_key(key);
    let mut out = [0u8; 32];
    let mut prev = *iv;
    for block_idx in 0..2 {
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = plaintext[block_idx * 16 + i] ^ prev[i];
        }
        let enc = aes128_encrypt_block(&block, &round_keys);
        out[block_idx * 16..(block_idx + 1) * 16].copy_from_slice(&enc);
        prev = enc;
    }
    out
}

pub fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}
