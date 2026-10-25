//! Encrypted CEF UI asset bundle ("ui.bin").
//!
//! Replaces the old setup where `cef/assets/*.html` sat on disk as plain,
//! directly-readable/editable files. The UI is built once (see
//! `scripts/pack_ui_assets.py`) into a single AES-256-GCM-encrypted
//! container that ships to players instead of the raw files. The
//! decryption key is handed to the client over the network by the CEF
//! server (see `AssetManifest`) right after a successful join, and is kept
//! only in memory — never written to disk, never baked into the binary.
//!
//! Container format ("UIB1"):
//!
//! ```text
//! offset  size        content
//! 0       4           magic b"UIB1"
//! 4       12          index_nonce
//! 16      4           index_len (u32 LE)
//! 20      index_len   index_ciphertext  (AES-256-GCM, AAD = b"idx")
//! 20+idx  ..          blob (concatenation of each file's ciphertext)
//! ```
//!
//! The decrypted index is JSON: a list of
//! `{ "p": "<relative/path>", "o": <offset into blob>, "s": <ciphertext len>, "n": "<12-byte nonce, hex>" }`.
//! Each file in the blob is independently AES-256-GCM encrypted with its
//! own nonce and with the file's relative path (UTF-8 bytes) as AAD, so a
//! ciphertext chunk cannot be silently relabelled as a different file.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

pub const KEY_LEN: usize = 32;
const MAGIC: &[u8; 4] = b"UIB1";
const INDEX_AAD: &[u8] = b"idx";

#[derive(serde::Deserialize)]
struct IndexEntry {
    p: String,
    o: u64,
    s: u64,
    n: String,
}

struct FileSlice {
    offset: u64,
    len: u64,
    nonce: [u8; 12],
}

pub struct AssetBundle {
    cipher: Aes256Gcm,
    blob: Vec<u8>,
    index: HashMap<String, FileSlice>,
}

impl AssetBundle {
    /// Parses and decrypts the container's index. The per-file bodies in
    /// `blob` stay encrypted until individually requested via `read`, so a
    /// single compromised file request never exposes the whole bundle.
    pub fn parse(raw: &[u8], key: &[u8; KEY_LEN]) -> Result<AssetBundle, String> {
        if raw.len() < 20 || &raw[0..4] != MAGIC {
            return Err("bad magic".into());
        }

        let index_nonce = &raw[4..16];
        let index_len = u32::from_le_bytes(raw[16..20].try_into().unwrap()) as usize;

        let index_start: usize = 20;
        let index_end = index_start
            .checked_add(index_len)
            .ok_or("index_len overflow")?;
        if raw.len() < index_end {
            return Err("truncated index".into());
        }

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));

        let index_plain = cipher
            .decrypt(
                Nonce::from_slice(index_nonce),
                Payload {
                    msg: &raw[index_start..index_end],
                    aad: INDEX_AAD,
                },
            )
            .map_err(|_| "cannot decrypt bundle index (wrong key or corrupted file)".to_string())?;

        let entries: Vec<IndexEntry> =
            serde_json::from_slice(&index_plain).map_err(|e| format!("bad index json: {e}"))?;

        let blob = raw[index_end..].to_vec();
        let mut index = HashMap::with_capacity(entries.len());

        for entry in entries {
            let nonce_bytes = decode_hex_12(&entry.n).ok_or("bad nonce hex in index")?;
            index.insert(
                normalize_rel_path(&entry.p),
                FileSlice {
                    offset: entry.o,
                    len: entry.s,
                    nonce: nonce_bytes,
                },
            );
        }

        Ok(AssetBundle {
            cipher,
            blob,
            index,
        })
    }

    /// Decrypts and returns the bytes of one file by its path relative to
    /// the bundle root (e.g. "Iternal_HUD.html" or
    /// "Iternal_InventoryIcons/Iternal_Item_beer.png").
    pub fn read(&self, rel_path: &str) -> Option<Vec<u8>> {
        let rel_path = normalize_rel_path(rel_path);
        let slice = self.index.get(&rel_path)?;

        let start = usize::try_from(slice.offset).ok()?;
        let len = usize::try_from(slice.len).ok()?;
        let end = start.checked_add(len)?;
        let ciphertext = self.blob.get(start..end)?;

        self.cipher
            .decrypt(
                Nonce::from_slice(&slice.nonce),
                Payload {
                    msg: ciphertext,
                    aad: rel_path.as_bytes(),
                },
            )
            .ok()
    }

    pub fn contains(&self, rel_path: &str) -> bool {
        self.index.contains_key(&normalize_rel_path(rel_path))
    }
}

fn normalize_rel_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_string()
}

fn decode_hex_12(s: &str) -> Option<[u8; 12]> {
    if s.len() != 24 {
        return None;
    }

    let mut out = [0u8; 12];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Some(out)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn sha256_hex_of_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| sha256_hex(&bytes))
}

static BUNDLE: OnceLock<Mutex<Option<AssetBundle>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<AssetBundle>> {
    BUNDLE.get_or_init(|| Mutex::new(None))
}

/// Installs the decrypted-index bundle as the active one. Called once the
/// client has a verified, up-to-date `ui.bin` on disk and the per-session
/// key from `AssetManifest`.
pub fn install(bundle: AssetBundle) {
    *slot().lock() = Some(bundle);
    tracing::info!("CEF UI asset bundle installed");
}

pub fn clear() {
    *slot().lock() = None;
}

/// Reads one file's bytes from the currently installed bundle, if any.
pub fn read(rel_path: &str) -> Option<Vec<u8>> {
    slot().lock().as_ref().and_then(|bundle| bundle.read(rel_path))
}

pub fn is_loaded() -> bool {
    slot().lock().is_some()
}
