//! Albumy chronione: klucze sesyjne, szyfrowanie plików i miniaturek.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::crypto::{self, Key};
use crate::indexer;

/// Klucze odblokowanych albumów — wyłącznie w pamięci, na czas sesji.
/// Zeroizing czyści pamięć przy usunięciu/zamknięciu aplikacji.
#[derive(Default)]
pub struct SessionKeys(pub Mutex<HashMap<i64, Zeroizing<Key>>>);

impl SessionKeys {
    pub fn get(&self, album_id: i64) -> Option<Key> {
        self.0.lock().unwrap().get(&album_id).map(|k| **k)
    }
    pub fn unlocked_ids(&self) -> Vec<i64> {
        self.0.lock().unwrap().keys().copied().collect()
    }
    pub fn insert(&self, album_id: i64, key: Key) {
        self.0.lock().unwrap().insert(album_id, Zeroizing::new(key));
    }
    pub fn lock_all(&self) {
        self.0.lock().unwrap().clear();
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

pub fn create_album(conn: &Connection, name: &str, password: &str) -> Result<(i64, Key), String> {
    if password.len() < 4 {
        return Err("Hasło musi mieć co najmniej 4 znaki".into());
    }
    let salt = crypto::random_bytes(16);
    let key = crypto::derive_key(password, &salt)?;
    conn.execute(
        "INSERT INTO albums (name, type, created_at, key_salt, key_verifier)
         VALUES (?1, 'protected', ?2, ?3, ?4)",
        rusqlite::params![name.trim(), now(), salt, crypto::key_verifier(&key)],
    )
    .map_err(|e| e.to_string())?;
    Ok((conn.last_insert_rowid(), key))
}

pub fn unlock(conn: &Connection, album_id: i64, password: &str) -> Result<Key, String> {
    let (salt, verifier): (Vec<u8>, Vec<u8>) = conn
        .query_row(
            "SELECT key_salt, key_verifier FROM albums WHERE id = ?1 AND type = 'protected'",
            [album_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "Album nie istnieje".to_string())?;
    let key = crypto::derive_key(password, &salt)?;
    if crypto::key_verifier(&key) != verifier {
        return Err("Nieprawidłowe hasło".into());
    }
    Ok(key)
}

/// Czy inna, niechroniona kopia treści (ten sam hash) nadal istnieje?
fn hash_visible_elsewhere(conn: &Connection, hash: &[u8], except_id: i64) -> bool {
    conn.query_row(
        "SELECT 1 FROM files WHERE hash = ?1 AND protected_album IS NULL AND id != ?2 LIMIT 1",
        rusqlite::params![hash, except_id],
        |_| Ok(()),
    )
    .is_ok()
}

/// Dodaje pliki do chronionego albumu: szyfruje na dysku (atomowo),
/// szyfruje miniaturkę (jeśli nie współdzieli jej jawna kopia).
pub fn add_files(
    conn: &Connection,
    root: &Path,
    thumbs_dir: &Path,
    key: &Key,
    album_id: i64,
    file_ids: &[i64],
) -> Result<u64, String> {
    let mut done = 0u64;
    for id in file_ids {
        let row: Option<(String, Option<Vec<u8>>, Option<i64>)> = conn
            .query_row(
                "SELECT path, hash, protected_album FROM files WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        let Some((rel, hash, already)) = row else {
            continue;
        };
        if already.is_some() {
            continue; // już chroniony
        }
        let src = root.join(&rel);
        let locked_rel = format!("{rel}.{}", crypto::LOCK_EXT);
        let dst = root.join(&locked_rel);
        crypto::encrypt_file(key, &src, &dst).map_err(|e| e.to_string())?;
        std::fs::remove_file(&src).map_err(|e| e.to_string())?;

        // miniaturka: szyfrujemy tylko, gdy nie należy też do jawnego duplikatu
        if let Some(hash) = &hash {
            if !hash_visible_elsewhere(conn, hash, *id) {
                let hex = indexer::hex(hash);
                let tpath = indexer::thumb_path(thumbs_dir, &hex);
                if tpath.exists() {
                    let tlocked = tpath.with_extension("webp.mnlock");
                    crypto::encrypt_file(key, &tpath, &tlocked).map_err(|e| e.to_string())?;
                    std::fs::remove_file(&tpath).ok();
                }
                let spath = indexer::sprite_path(thumbs_dir, &hex);
                if spath.exists() {
                    std::fs::remove_file(&spath).ok(); // scrubbing zbędny dla chronionych
                }
            }
        }

        conn.execute(
            "UPDATE files SET path = ?2, protected_album = ?3 WHERE id = ?1",
            rusqlite::params![id, locked_rel, album_id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO album_files (album_id, file_id, added_at) VALUES (?1,?2,?3)",
            rusqlite::params![album_id, id, now()],
        )
        .map_err(|e| e.to_string())?;
        done += 1;
    }
    Ok(done)
}

/// Wyjmuje pliki z chronionego albumu — odszyfrowuje z powrotem.
pub fn remove_files(
    conn: &Connection,
    root: &Path,
    thumbs_dir: &Path,
    key: &Key,
    album_id: i64,
    file_ids: &[i64],
) -> Result<u64, String> {
    let mut done = 0u64;
    for id in file_ids {
        let row: Option<(String, Option<Vec<u8>>)> = conn
            .query_row(
                "SELECT path, hash FROM files WHERE id = ?1 AND protected_album = ?2",
                rusqlite::params![id, album_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let Some((locked_rel, hash)) = row else {
            continue;
        };
        let plain_rel = locked_rel
            .strip_suffix(&format!(".{}", crypto::LOCK_EXT))
            .unwrap_or(&locked_rel)
            .to_string();
        crypto::decrypt_file(key, &root.join(&locked_rel), &root.join(&plain_rel))
            .map_err(|e| e.to_string())?;
        std::fs::remove_file(root.join(&locked_rel)).ok();

        if let Some(hash) = &hash {
            let hex = indexer::hex(hash);
            let tpath = indexer::thumb_path(thumbs_dir, &hex);
            let tlocked = tpath.with_extension("webp.mnlock");
            if !tpath.exists() && tlocked.exists() {
                crypto::decrypt_file(key, &tlocked, &tpath).map_err(|e| e.to_string())?;
                std::fs::remove_file(&tlocked).ok();
            }
        }

        conn.execute(
            "UPDATE files SET path = ?2, protected_album = NULL WHERE id = ?1",
            rusqlite::params![id, plain_rel],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM album_files WHERE album_id = ?1 AND file_id = ?2",
            [album_id, *id],
        )
        .map_err(|e| e.to_string())?;
        done += 1;
    }
    Ok(done)
}

/// Zmiana hasła: re-szyfrowanie wszystkich plików albumu nowym kluczem.
pub fn change_password(
    conn: &Connection,
    root: &Path,
    thumbs_dir: &Path,
    old_key: &Key,
    album_id: i64,
    new_password: &str,
) -> Result<Key, String> {
    let ids: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT file_id FROM album_files WHERE album_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows: Vec<i64> = match stmt.query_map([album_id], |r| r.get(0)) {
            Ok(mapped) => mapped.filter_map(Result::ok).collect(),
            Err(e) => return Err(e.to_string()),
        };
        rows
    };
    // wyjmij starym kluczem, włóż nowym — wolne, ale poprawne i atomowe per plik
    remove_files(conn, root, thumbs_dir, old_key, album_id, &ids)?;
    let salt = crypto::random_bytes(16);
    let new_key = crypto::derive_key(new_password, &salt)?;
    conn.execute(
        "UPDATE albums SET key_salt = ?2, key_verifier = ?3 WHERE id = ?1",
        rusqlite::params![album_id, salt, crypto::key_verifier(&new_key)],
    )
    .map_err(|e| e.to_string())?;
    add_files(conn, root, thumbs_dir, &new_key, album_id, &ids)?;
    Ok(new_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn setup(name: &str) -> (Connection, std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("medianest-test-protected-{name}"));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("lib");
        let thumbs = base.join("thumbs");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&thumbs).unwrap();
        (db::open(&base.join("data")).unwrap(), root, thumbs)
    }

    fn add_file(conn: &Connection, root: &Path, thumbs: &Path, rel: &str, content: &[u8]) -> i64 {
        std::fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
        std::fs::write(root.join(rel), content).unwrap();
        let hash = blake3::hash(content).as_bytes().to_vec();
        let hex = indexer::hex(&hash);
        let tpath = indexer::thumb_path(thumbs, &hex);
        std::fs::create_dir_all(tpath.parent().unwrap()).unwrap();
        std::fs::write(&tpath, b"miniaturka").unwrap();
        conn.execute(
            "INSERT INTO files (path,parent,name,kind,size,mtime,hash,thumb,blurhash)
             VALUES (?1,'',?1,0,1,1,?2,1,'LKO2?U')",
            rusqlite::params![rel, hash],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn protect_unprotect_roundtrip() {
        let (conn, root, thumbs) = setup("roundtrip");
        let content = b"sekretne zdjecie";
        let id = add_file(&conn, &root, &thumbs, "prywatne.jpg", content);

        let (album_id, key) = create_album(&conn, "Sejf", "haslo123").unwrap();
        add_files(&conn, &root, &thumbs, &key, album_id, &[id]).unwrap();

        // oryginał zniknął, jest .mnlock; miniaturka zaszyfrowana
        assert!(!root.join("prywatne.jpg").exists());
        assert!(root.join("prywatne.jpg.mnlock").exists());
        let hash = blake3::hash(content).as_bytes().to_vec();
        let hex = indexer::hex(&hash);
        assert!(!indexer::thumb_path(&thumbs, &hex).exists());
        assert!(indexer::thumb_path(&thumbs, &hex)
            .with_extension("webp.mnlock")
            .exists());

        // odblokowanie działa tylko poprawnym hasłem
        assert!(unlock(&conn, album_id, "zle").is_err());
        let key2 = unlock(&conn, album_id, "haslo123").unwrap();
        assert_eq!(key, key2);

        // wyjęcie z albumu przywraca plik i miniaturkę
        remove_files(&conn, &root, &thumbs, &key2, album_id, &[id]).unwrap();
        assert_eq!(std::fs::read(root.join("prywatne.jpg")).unwrap(), content);
        assert!(indexer::thumb_path(&thumbs, &hex).exists());
        let prot: Option<i64> = conn
            .query_row("SELECT protected_album FROM files WHERE id=?1", [id], |r| r.get(0))
            .unwrap();
        assert!(prot.is_none());
    }

    #[test]
    fn shared_thumb_stays_when_duplicate_visible() {
        let (conn, root, thumbs) = setup("shared");
        let content = b"wspolna tresc";
        let id1 = add_file(&conn, &root, &thumbs, "a.jpg", content);
        let _id2 = add_file(&conn, &root, &thumbs, "b.jpg", content); // duplikat jawny

        let (album_id, key) = create_album(&conn, "Sejf", "haslo123").unwrap();
        add_files(&conn, &root, &thumbs, &key, album_id, &[id1]).unwrap();

        // miniaturka zostaje jawna — należy też do widocznego b.jpg
        let hex = indexer::hex(blake3::hash(content).as_bytes());
        assert!(indexer::thumb_path(&thumbs, &hex).exists());
    }

    #[test]
    fn change_password_reencrypts() {
        let (conn, root, thumbs) = setup("chpass");
        let id = add_file(&conn, &root, &thumbs, "x.jpg", b"tresc");
        let (album_id, key) = create_album(&conn, "Sejf", "stare").unwrap();
        add_files(&conn, &root, &thumbs, &key, album_id, &[id]).unwrap();

        let new_key = change_password(&conn, &root, &thumbs, &key, album_id, "nowe").unwrap();
        assert!(unlock(&conn, album_id, "stare").is_err());
        assert_eq!(unlock(&conn, album_id, "nowe").unwrap(), new_key);
        // plik nadal zaszyfrowany i czytelny nowym kluczem
        let mut enc =
            crypto::EncryptedFile::open(&new_key, &root.join("x.jpg.mnlock")).unwrap();
        assert_eq!(enc.read_all().unwrap(), b"tresc");
    }
}
