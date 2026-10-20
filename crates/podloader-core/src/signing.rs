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

pub fn sign_itunesdb(db_bytes: &mut [u8], firewire_guid_hex: &str) -> Result<(), PodLoaderError> {
    let guid = FireWireGuid::parse(firewire_guid_hex)?;
    if db_bytes.len() < 0xA0 || db_bytes.get(0..4) != Some(b"mhbd") {
        return Err(PodLoaderError::InvalidItunesDb(
            "mhbd header too short for hash58/hash72 signing".into(),
        ));
    }

    let (hash58, hash72) = compute_hashes(db_bytes, &guid);
    db_bytes[0x58..0x58 + 20].copy_from_slice(&hash58);
    db_bytes[0x72..0x72 + 46].copy_from_slice(&hash72);
    Ok(())
}

pub fn verify_itunesdb_signatures(db_bytes: &[u8], firewire_guid_hex: &str) -> bool {
    let Ok(guid) = FireWireGuid::parse(firewire_guid_hex) else {
        return false;
    };
    if db_bytes.len() < 0xA0 || db_bytes.get(0..4) != Some(b"mhbd") {
        return false;
    }
    let (expected_58, expected_72) = compute_hashes(db_bytes, &guid);
    db_bytes[0x58..0x58 + 20] == expected_58 && db_bytes[0x72..0x72 + 46] == expected_72
}

const HASH58_FIXED_SALT: [u8; 16] = [
    0x61, 0x6C, 0x74, 0x7A, 0x7A, 0x39, 0x62, 0x6E, 0x6B, 0x32, 0x39, 0x72, 0x76, 0x31, 0x30, 0x70,
];

const HASH72_AES_KEY: [u8; 16] = [
    0x61, 0x8C, 0xA1, 0x0D, 0xC7, 0xF5, 0x7F, 0xD3, 0xB4, 0x72, 0x3E, 0x08, 0x15, 0x74, 0x63, 0xD7,
];

fn gcd_u8(mut a: u8, mut b: u8) -> u8 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

fn lcm_u8(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 1;
    }
    let g = gcd_u8(a, b) as u16;
    (((a as u16) * (b as u16)) / g) as u8
}

fn derive_hash58_hmac_key(guid: &[u8; 8]) -> [u8; 64] {
    let mut y = [0u8; 16];
    for i in 0..8 {
        let a = guid[i];
        let b = HASH58_FIXED_SALT[i * 2];
        let c = HASH58_FIXED_SALT[i * 2 + 1];
        y[i * 2] = gcd_u8(a, b);
        y[i * 2 + 1] = lcm_u8(a, c);
    }
    let mut seed = Vec::with_capacity(24);
    seed.extend_from_slice(&y);
    seed.extend_from_slice(guid);
    let digest = sha1_digest(&seed);

    let mut key_block = [0u8; 64];
    for (i, slot) in key_block.iter_mut().enumerate() {
        *slot = digest[i % 20].wrapping_add(y[i % 16]);
    }
    key_block
}

fn compute_hashes(db_bytes: &[u8], guid: &FireWireGuid) -> ([u8; 20], [u8; 46]) {
    let mut normalized = db_bytes.to_vec();
    // Zero fields excluded by iPod firmware when computing database hashes
    normalized[0x18..0x20].fill(0);
    normalized[0x32..0x36].fill(0);
    normalized[0x58..0x58 + 20].fill(0);
    normalized[0x72..0x72 + 46].fill(0);

    // 1. Compute hash58 via GCD/LCM key schedule + HMAC-SHA1
    let key_block = derive_hash58_hmac_key(&guid.0);
    let mut k_ipad = [0x36u8; 64];
    let mut k_opad = [0x5Cu8; 64];
    for i in 0..64 {
        k_ipad[i] ^= key_block[i];
        k_opad[i] ^= key_block[i];
    }

    let mut inner_input = Vec::with_capacity(64 + normalized.len());
    inner_input.extend_from_slice(&k_ipad);
    inner_input.extend_from_slice(&normalized);
    let inner_hash = sha1_digest(&inner_input);

    let mut outer_input = Vec::with_capacity(84);
    outer_input.extend_from_slice(&k_opad);
    outer_input.extend_from_slice(&inner_hash);
    let hash58 = sha1_digest(&outer_input);

    // 2. Compute hash72 (0x01 0x00 header + 12-byte random/GUID nonce + 32-byte AES-128-CBC encrypted SHA-1 block)
    let body_sha1 = sha1_digest(&normalized);
    let mut nonce = [0u8; 12];
    nonce[0..8].copy_from_slice(&guid.0);
    nonce[8..12].copy_from_slice(&body_sha1[0..4]);

    let mut iv = [0u8; 16];
    iv[0..12].copy_from_slice(&nonce);
    iv[12..16].copy_from_slice(&guid.0[0..4]);

    let mut plaintext = [0u8; 32];
    plaintext[0..20].copy_from_slice(&body_sha1);
    plaintext[20..32].copy_from_slice(&nonce);

    let ciphertext = aes128_cbc_encrypt(&plaintext, &HASH72_AES_KEY, &iv);

    let mut hash72 = [0u8; 46];
    hash72[0] = 0x01;
    hash72[1] = 0x00;
    hash72[2..14].copy_from_slice(&nonce);
    hash72[14..46].copy_from_slice(&ciphertext);

    (hash58, hash72)
}

const AES_SBOX: [u8; 256] = [
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
            AES_SBOX[prev[13] as usize] ^ RCON[r - 1],
            AES_SBOX[prev[14] as usize],
            AES_SBOX[prev[15] as usize],
            AES_SBOX[prev[12] as usize],
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
            *b = AES_SBOX[*b as usize];
        }
        // ShiftRows
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
            // MixColumns
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
