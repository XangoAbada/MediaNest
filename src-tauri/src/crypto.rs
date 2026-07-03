//! Szyfrowanie albumów chronionych: Argon2id (klucz z hasła) +
//! XChaCha20-Poly1305 w blokach 4 MB (losowy dostęp do wideo).
//!
//! Format pliku .mnlock:
//!   "MNST1" (5 B) | base_nonce (16 B) | plaintext_len (8 B LE) |
//!   bloki: ciphertext(4 MB + 16 B tagu) ...
//! Nonce bloku i = base_nonce || (i as u64 LE) → 24 B nonce XChaCha.
//! AEAD wykrywa każdą modyfikację — pliku nie da się po cichu uszkodzić.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

pub const MAGIC: &[u8; 5] = b"MNST1";
pub const CHUNK: usize = 4 * 1024 * 1024;
const TAG: usize = 16;
pub const HEADER: usize = 5 + 16 + 8;
pub const LOCK_EXT: &str = "mnlock";

pub type Key = [u8; 32];

/// Klucz albumu z hasła (Argon2id, parametry domyślne OWASP).
pub fn derive_key(password: &str, salt: &[u8]) -> Result<Key, String> {
    use argon2::Argon2;
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

/// Weryfikator klucza przechowywany w bazie (hasła nie zapisujemy nigdzie).
pub fn key_verifier(key: &Key) -> Vec<u8> {
    blake3::keyed_hash(b"MediaNest-key-verifier-context-1", key)
        .as_bytes()
        .to_vec()
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).expect("os rng");
    buf
}

fn chunk_nonce(base: &[u8], index: u64) -> XNonce {
    let mut nonce = [0u8; 24];
    nonce[..16].copy_from_slice(base);
    nonce[16..].copy_from_slice(&index.to_le_bytes());
    nonce.into()
}

/// Szyfruje plik atomowo: zapis do `.tmp` → fsync → rename → dopiero wtedy
/// wolno usunąć oryginał. Przerwanie w dowolnym momencie nie traci danych.
pub fn encrypt_file(key: &Key, src: &Path, dst: &Path) -> anyhow::Result<()> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let base = random_bytes(16);
    let mut input = std::fs::File::open(src)?;
    let len = input.metadata()?.len();

    let tmp = dst.with_extension("mnlock.tmp");
    let mut out = std::fs::File::create(&tmp)?;
    out.write_all(MAGIC)?;
    out.write_all(&base)?;
    out.write_all(&len.to_le_bytes())?;

    let mut buf = vec![0u8; CHUNK];
    let mut index = 0u64;
    loop {
        let mut filled = 0;
        // dopełnij blok (read może zwracać mniej)
        while filled < CHUNK {
            let n = input.read(&mut buf[filled..])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 && index > 0 {
            break;
        }
        let ct = cipher
            .encrypt(&chunk_nonce(&base, index), &buf[..filled])
            .map_err(|_| anyhow::anyhow!("szyfrowanie nie powiodło się"))?;
        out.write_all(&ct)?;
        index += 1;
        if filled < CHUNK {
            break;
        }
    }
    out.sync_all()?;
    drop(out);
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

pub struct EncryptedFile {
    file: std::fs::File,
    cipher: XChaCha20Poly1305,
    base: [u8; 16],
    pub plaintext_len: u64,
}

impl EncryptedFile {
    pub fn open(key: &Key, path: &Path) -> anyhow::Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let mut header = [0u8; HEADER];
        file.read_exact(&mut header)?;
        if &header[..5] != MAGIC {
            anyhow::bail!("to nie jest plik MediaNest");
        }
        let mut base = [0u8; 16];
        base.copy_from_slice(&header[5..21]);
        let plaintext_len = u64::from_le_bytes(header[21..29].try_into().unwrap());
        Ok(Self {
            file,
            cipher: XChaCha20Poly1305::new(key.into()),
            base,
            plaintext_len,
        })
    }

    fn read_chunk(&mut self, index: u64) -> anyhow::Result<Vec<u8>> {
        let chunk_count = self.plaintext_len.div_ceil(CHUNK as u64).max(1);
        if index >= chunk_count {
            anyhow::bail!("blok poza plikiem");
        }
        let is_last = index == chunk_count - 1;
        let pt_len = if is_last {
            (self.plaintext_len - index * CHUNK as u64) as usize
        } else {
            CHUNK
        };
        let offset = HEADER as u64 + index * (CHUNK + TAG) as u64;
        self.file.seek(SeekFrom::Start(offset))?;
        let mut ct = vec![0u8; pt_len + TAG];
        self.file.read_exact(&mut ct)?;
        self.cipher
            .decrypt(&chunk_nonce(&self.base, index), ct.as_slice())
            .map_err(|_| anyhow::anyhow!("odszyfrowanie nie powiodło się (złe hasło lub uszkodzony plik)"))
    }

    /// Odszyfrowuje zakres bajtów — losowy dostęp do przewijania wideo.
    pub fn read_range(&mut self, start: u64, len: usize) -> anyhow::Result<Vec<u8>> {
        let end = (start + len as u64).min(self.plaintext_len);
        if start >= end {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity((end - start) as usize);
        let mut pos = start;
        while pos < end {
            let chunk_index = pos / CHUNK as u64;
            let within = (pos % CHUNK as u64) as usize;
            let chunk = self.read_chunk(chunk_index)?;
            let take = ((end - pos) as usize).min(chunk.len() - within);
            out.extend_from_slice(&chunk[within..within + take]);
            pos += take as u64;
        }
        Ok(out)
    }

    pub fn read_all(&mut self) -> anyhow::Result<Vec<u8>> {
        self.read_range(0, self.plaintext_len as usize)
    }
}

/// Odszyfrowuje plik na dysk (atomowo, przez plik tymczasowy).
pub fn decrypt_file(key: &Key, src: &Path, dst: &Path) -> anyhow::Result<()> {
    let mut enc = EncryptedFile::open(key, src)?;
    let tmp = dst.with_extension("mndec.tmp");
    let mut out = std::fs::File::create(&tmp)?;
    let chunk_count = enc.plaintext_len.div_ceil(CHUNK as u64).max(1);
    for i in 0..chunk_count {
        let chunk = enc.read_chunk(i)?;
        out.write_all(&chunk)?;
    }
    out.sync_all()?;
    drop(out);
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("medianest-test-crypto-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn roundtrip_and_wrong_password() {
        let dir = setup("roundtrip");
        let salt = random_bytes(16);
        let key = derive_key("tajne haslo", &salt).unwrap();

        // dane większe niż jeden blok, żeby przetestować podział
        let data: Vec<u8> = (0..CHUNK + 12345).map(|i| (i % 251) as u8).collect();
        let src = dir.join("orig.jpg");
        std::fs::write(&src, &data).unwrap();

        let locked = dir.join("orig.jpg.mnlock");
        encrypt_file(&key, &src, &locked).unwrap();
        assert!(locked.exists());
        // zaszyfrowany plik nie zawiera plaintextu
        let ct = std::fs::read(&locked).unwrap();
        assert!(!ct.windows(64).any(|w| w == &data[..64]));

        // pełny odczyt
        let mut enc = EncryptedFile::open(&key, &locked).unwrap();
        assert_eq!(enc.plaintext_len, data.len() as u64);
        assert_eq!(enc.read_all().unwrap(), data);

        // dostęp swobodny (zakres przez granicę bloków)
        let range = enc.read_range(CHUNK as u64 - 100, 200).unwrap();
        assert_eq!(range, &data[CHUNK - 100..CHUNK + 100]);

        // złe hasło = odmowa, nie śmieci
        let bad = derive_key("zle haslo", &salt).unwrap();
        let mut enc = EncryptedFile::open(&bad, &locked).unwrap();
        assert!(enc.read_all().is_err());

        // odszyfrowanie na dysk
        let restored = dir.join("restored.jpg");
        decrypt_file(&key, &locked, &restored).unwrap();
        assert_eq!(std::fs::read(&restored).unwrap(), data);
    }

    #[test]
    fn verifier_detects_password() {
        let salt = random_bytes(16);
        let k1 = derive_key("haslo", &salt).unwrap();
        let k2 = derive_key("haslo", &salt).unwrap();
        let k3 = derive_key("inne", &salt).unwrap();
        assert_eq!(key_verifier(&k1), key_verifier(&k2));
        assert_ne!(key_verifier(&k1), key_verifier(&k3));
    }

    #[test]
    fn tampered_file_is_rejected() {
        let dir = setup("tamper");
        let salt = random_bytes(16);
        let key = derive_key("x", &salt).unwrap();
        let src = dir.join("a.jpg");
        std::fs::write(&src, b"dane wrazliwe").unwrap();
        let locked = dir.join("a.jpg.mnlock");
        encrypt_file(&key, &src, &locked).unwrap();

        // przestaw jeden bajt ciphertextu
        let mut bytes = std::fs::read(&locked).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        std::fs::write(&locked, &bytes).unwrap();

        let mut enc = EncryptedFile::open(&key, &locked).unwrap();
        assert!(enc.read_all().is_err());
    }

    #[test]
    fn empty_file_roundtrip() {
        let dir = setup("empty");
        let key = derive_key("x", &random_bytes(16)).unwrap();
        let src = dir.join("empty.jpg");
        std::fs::write(&src, b"").unwrap();
        let locked = dir.join("empty.jpg.mnlock");
        encrypt_file(&key, &src, &locked).unwrap();
        let mut enc = EncryptedFile::open(&key, &locked).unwrap();
        assert_eq!(enc.read_all().unwrap(), Vec::<u8>::new());
    }
}
