use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use image::DynamicImage;
use rayon::prelude::*;
use rusqlite::Connection;
use tauri::{AppHandle, Emitter, Manager};

use crate::db;
use crate::video;

pub const PHOTO_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "webp", "gif", "bmp", "tiff", "tif", "dng", "cr2", "nef", "arw",
];
pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "avi", "mkv", "m4v", "mts", "wmv", "3gp"];

const BATCH: usize = 64;
const THUMB_SIZE: u32 = 256;

/// Współdzielony stan sterujący indekserem.
pub struct IndexerCtl {
    /// Folder oglądany w UI — jego pliki mają priorytet w kolejce.
    pub focus: Mutex<Option<String>>,
    /// Żądanie pełnego skanu biblioteki (start, watcher, zmiana folderu).
    pub rescan: AtomicBool,
}

impl Default for IndexerCtl {
    fn default() -> Self {
        Self {
            focus: Mutex::new(None),
            rescan: AtomicBool::new(true),
        }
    }
}

pub fn kind_of(path: &Path) -> Option<i64> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if PHOTO_EXTS.contains(&ext.as_str()) {
        Some(0)
    } else if VIDEO_EXTS.contains(&ext.as_str()) {
        Some(1)
    } else {
        None
    }
}

pub fn thumb_path(thumbs_dir: &Path, hash_hex: &str) -> PathBuf {
    thumbs_dir.join(&hash_hex[..2]).join(format!("{hash_hex}.webp"))
}

pub fn sprite_path(thumbs_dir: &Path, hash_hex: &str) -> PathBuf {
    thumbs_dir
        .join(&hash_hex[..2])
        .join(format!("{hash_hex}.sprite.webp"))
}

#[derive(serde::Serialize, Clone)]
struct Progress {
    pending: i64,
    total: i64,
}

/// Główna pętla indeksera — uruchamiana raz, w dedykowanym wątku.
/// ponytail: pojedynczy wątek koordynujący + rayon wewnątrz batcha;
/// osobne kolejki per typ zadania, jeśli przepustowość przestanie wystarczać.
pub fn run(app: AppHandle) {
    let data_dir = app.path().app_data_dir().expect("app data dir");
    let thumbs_dir = data_dir.join("thumbs");
    let conn = db::open(&data_dir).expect("indexer db");
    let ctl = app.state::<std::sync::Arc<IndexerCtl>>().inner().clone();
    let mut total: i64 = 0;

    loop {
        let Some(root) = db::get_setting(&conn, "library_path").map(PathBuf::from) else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };

        if ctl.rescan.swap(false, Ordering::SeqCst) {
            if let Err(e) = scan(&conn, &root) {
                eprintln!("scan error: {e}");
            }
            total = pending_count(&conn);
            emit_progress(&app, &conn, total);
        }

        let focus = ctl.focus.lock().unwrap().clone();
        let batch = fetch_batch(&conn, focus);
        if batch.is_empty() {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        process_batch(&conn, &root, &thumbs_dir, &batch);
        emit_progress(&app, &conn, total);
    }
}

fn process_batch(conn: &Connection, root: &Path, thumbs_dir: &Path, batch: &[(i64, String, i64)]) {
    let results: Vec<Done> = batch
        .par_iter()
        .map(|(id, rel, kind)| process(root, thumbs_dir, *id, rel, *kind))
        .collect();

    let tx = conn.unchecked_transaction().expect("tx");
    for d in &results {
        tx.execute(
            "UPDATE files SET hash=?2, width=?3, height=?4, taken_at=?5,
                              blurhash=?6, thumb=?7, error=?8, duration=?9,
                              phash=?10, dhash=?11, exif_date=?12 WHERE id=?1",
            rusqlite::params![
                d.id, d.hash, d.width, d.height, d.taken_at, d.blurhash, d.thumb, d.error,
                d.duration, d.phash, d.dhash, d.exif_date
            ],
        )
        .ok();
    }
    tx.commit().ok();
}

/// Synchroniczne przetworzenie całej kolejki — benchmark i narzędzia dev.
pub fn process_all_pending(conn: &Connection, root: &Path, thumbs_dir: &Path) {
    loop {
        let batch = fetch_batch(conn, None);
        if batch.is_empty() {
            break;
        }
        process_batch(conn, root, thumbs_dir, &batch);
    }
}

fn pending_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM files
         WHERE status=0 AND (hash IS NULL OR thumb=0 OR (kind=0 AND phash IS NULL AND thumb=1))",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

fn emit_progress(app: &AppHandle, conn: &Connection, total: i64) {
    let pending = pending_count(conn);
    app.emit("index-progress", Progress { pending, total }).ok();
}

fn fetch_batch(conn: &Connection, focus: Option<String>) -> Vec<(i64, String, i64)> {
    // wideo czekają w kolejce, dopóki ffmpeg nie będzie gotowy
    let videos_ok = video::ready();
    // do kolejki wchodzą też zdjęcia bez phash (migracja v5 na istniejących bazach)
    let mut stmt = conn
        .prepare_cached(
            "SELECT id, path, kind FROM files
             WHERE status=0
               AND (hash IS NULL OR thumb=0 OR (kind=0 AND phash IS NULL AND thumb=1))
               AND (kind=0 OR ?3)
             ORDER BY (parent = ?1) DESC, id LIMIT ?2",
        )
        .expect("stmt");
    stmt.query_map(rusqlite::params![focus, BATCH as i64, videos_ok], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// Skan przyrostowy: porównuje dysk z bazą po (path, size, mtime).
pub fn scan(conn: &Connection, root: &Path) -> anyhow::Result<()> {
    let mut known: HashMap<String, (i64, i64, i64, i64)> = HashMap::new();
    {
        // pliki chronione (.mnlock) są niewidoczne dla walkdir (filtr rozszerzeń),
        // więc nie mogą brać udziału w diffie — inaczej zostałyby oznaczone jako brakujące
        let mut stmt = conn.prepare(
            "SELECT path, id, size, mtime, status FROM files
             WHERE status != 2 AND protected_album IS NULL",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?),
            ))
        })?;
        for row in rows.flatten() {
            known.insert(row.0, row.1);
        }
    }

    let tx = conn.unchecked_transaction()?;
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let Some(kind) = kind_of(entry.path()) else {
            continue;
        };
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len() as i64;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        match known.remove(&rel) {
            None => {
                let parent = rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
                let name = rel.rsplit('/').next().unwrap_or(&rel);
                tx.execute(
                    "INSERT INTO files (path, parent, name, kind, size, mtime) VALUES (?1,?2,?3,?4,?5,?6)",
                    rusqlite::params![rel, parent, name, kind, size, mtime],
                )?;
            }
            Some((id, ksize, kmtime, kstatus)) => {
                if ksize != size || kmtime != mtime {
                    tx.execute(
                        "UPDATE files SET size=?2, mtime=?3, hash=NULL, thumb=0, status=0, error=NULL WHERE id=?1",
                        rusqlite::params![id, size, mtime],
                    )?;
                } else if kstatus != 0 {
                    tx.execute("UPDATE files SET status=0 WHERE id=?1", [id])?;
                }
            }
        }
    }
    // co zostało w known, tego nie ma na dysku
    for (id, _, _, status) in known.values() {
        if *status == 0 {
            tx.execute("UPDATE files SET status=1 WHERE id=?1", [id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

struct Done {
    id: i64,
    hash: Option<Vec<u8>>,
    width: Option<i64>,
    height: Option<i64>,
    duration: Option<f64>,
    taken_at: Option<i64>,
    blurhash: Option<String>,
    phash: Option<i64>,
    dhash: Option<i64>,
    exif_date: bool,
    thumb: i64,
    error: Option<String>,
}

fn process(root: &Path, thumbs_dir: &Path, id: i64, rel: &str, kind: i64) -> Done {
    let abs = root.join(rel);
    let mut done = Done {
        id,
        hash: None,
        width: None,
        height: None,
        duration: None,
        taken_at: None,
        blurhash: None,
        phash: None,
        dhash: None,
        exif_date: false,
        thumb: 2,
        error: None,
    };

    let hash = if kind == 0 {
        full_hash(&abs)
    } else {
        quick_hash(&abs)
    };
    let hash = match hash {
        Ok(h) => h,
        Err(e) => {
            done.error = Some(format!("hash: {e}"));
            done.hash = Some(Vec::new()); // opuszcza kolejkę
            return done;
        }
    };
    let hash_hex = hex(&hash);
    done.hash = Some(hash);

    // fallback: data modyfikacji pliku
    done.taken_at = std::fs::metadata(&abs)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    if kind != 0 {
        process_video(&abs, thumbs_dir, &hash_hex, &mut done);
        return done;
    }

    let (orientation, exif_date) = read_exif(&abs);
    if let Some(d) = exif_date {
        done.taken_at = Some(d);
        done.exif_date = true;
    }

    match image::open(&abs) {
        Ok(img) => {
            let img = apply_orientation(img, orientation);
            done.width = Some(img.width() as i64);
            done.height = Some(img.height() as i64);

            let thumb = img.thumbnail(THUMB_SIZE, THUMB_SIZE);
            match write_thumb(thumbs_dir, &hash_hex, &thumb) {
                Ok(()) => done.thumb = 1,
                Err(e) => done.error = Some(format!("thumb: {e}")),
            }
            done.blurhash = make_blurhash(&thumb);
            // hashe percepcyjne z miniaturki (dekodujemy raz, liczymy z małej)
            let (phash, dhash) = crate::dedup::perceptual_hashes(&thumb);
            done.phash = Some(phash);
            done.dhash = Some(dhash);
        }
        Err(e) => {
            // HEIC/RAW: dekodowanie w Fazie 7 — plik zindeksowany, bez miniaturki
            done.error = Some(format!("decode: {e}"));
        }
    }
    done
}

fn process_video(abs: &Path, thumbs_dir: &Path, hash_hex: &str, done: &mut Done) {
    match video::probe(abs) {
        Ok(meta) => {
            done.duration = meta.duration;
            done.width = meta.width.map(i64::from);
            done.height = meta.height.map(i64::from);
        }
        Err(e) => {
            done.error = Some(format!("probe: {e}"));
            return;
        }
    }
    let tpath = thumb_path(thumbs_dir, hash_hex);
    if tpath.exists() || video::thumbnail(abs, done.duration, &tpath) {
        done.thumb = 1;
        // blurhash z gotowej miniaturki webp
        if let Ok(img) = image::open(&tpath) {
            done.blurhash = make_blurhash(&img);
        }
    } else {
        done.error = Some("thumb: ffmpeg nie wygenerował miniaturki".into());
    }
    if let Some(dur) = done.duration {
        let spath = sprite_path(thumbs_dir, hash_hex);
        if !spath.exists() {
            video::sprite(abs, dur, &spath);
        }
    }
}

pub(crate) fn full_hash(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_mmap_rayon(path)?;
    Ok(hasher.finalize().as_bytes().to_vec())
}

/// Szybki hash wideo: pierwsze 1 MB + ostatnie 1 MB + rozmiar.
/// ponytail: pełny hash tylko przy weryfikacji importu / kolizji.
pub(crate) fn quick_hash(path: &Path) -> std::io::Result<Vec<u8>> {
    const CHUNK: u64 = 1024 * 1024;
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&len.to_le_bytes());
    let mut buf = vec![0u8; CHUNK as usize];
    let n = f.read(&mut buf)?;
    hasher.update(&buf[..n]);
    if len > CHUNK * 2 {
        f.seek(SeekFrom::End(-(CHUNK as i64)))?;
        let n = f.read(&mut buf)?;
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().as_bytes().to_vec())
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn read_exif(path: &Path) -> (u32, Option<i64>) {
    let Ok(file) = std::fs::File::open(path) else {
        return (1, None);
    };
    let mut reader = std::io::BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return (1, None);
    };
    let orientation = exif
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .unwrap_or(1);
    let date = exif
        .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        .or_else(|| exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY))
        .map(|f| f.display_value().to_string())
        .and_then(|s| {
            chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(&s, "%Y:%m:%d %H:%M:%S"))
                .ok()
        })
        .map(|dt| dt.and_utc().timestamp());
    (orientation, date)
}

fn apply_orientation(img: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

fn write_thumb(thumbs_dir: &Path, hash_hex: &str, thumb: &DynamicImage) -> anyhow::Result<()> {
    let path = thumb_path(thumbs_dir, hash_hex);
    if path.exists() {
        return Ok(()); // duplikat treści — miniaturka już jest
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let rgb = thumb.to_rgb8();
    let encoded = webp::Encoder::from_rgb(&rgb, rgb.width(), rgb.height()).encode(80.0);
    std::fs::write(path, &*encoded)?;
    Ok(())
}

fn make_blurhash(thumb: &DynamicImage) -> Option<String> {
    let small = thumb.thumbnail(32, 32).to_rgba8();
    blurhash::encode(4, 3, small.width(), small.height(), &small).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_detects_new_changed_missing() {
        let dir = std::env::temp_dir().join("medianest-test-scan");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib/sub")).unwrap();
        let db_dir = dir.join("db");
        let conn = crate::db::open(&db_dir).unwrap();
        let root = dir.join("lib");

        // nowe pliki (nie-media ignorowane)
        std::fs::write(root.join("a.jpg"), b"aaa").unwrap();
        std::fs::write(root.join("sub/b.mp4"), b"bbbb").unwrap();
        std::fs::write(root.join("c.txt"), b"x").unwrap();
        scan(&conn, &root).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
        let parent: String = conn
            .query_row("SELECT parent FROM files WHERE name='b.mp4'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(parent, "sub");

        // zmiana pliku resetuje hash
        conn.execute("UPDATE files SET hash=x'01', thumb=1", []).unwrap();
        std::fs::write(root.join("a.jpg"), b"aaaa-changed").unwrap();
        scan(&conn, &root).unwrap();
        let (hash, thumb): (Option<Vec<u8>>, i64) = conn
            .query_row("SELECT hash, thumb FROM files WHERE name='a.jpg'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert!(hash.is_none());
        assert_eq!(thumb, 0);

        // usunięcie oznacza status=1, powrót przywraca
        std::fs::remove_file(root.join("sub/b.mp4")).unwrap();
        scan(&conn, &root).unwrap();
        let status: i64 = conn
            .query_row("SELECT status FROM files WHERE name='b.mp4'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, 1);
        std::fs::write(root.join("sub/b.mp4"), b"bbbb").unwrap();
        scan(&conn, &root).unwrap();
        let status: i64 = conn
            .query_row("SELECT status FROM files WHERE name='b.mp4'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, 0);
    }

    #[test]
    fn quick_hash_differs_on_content_and_size() {
        let dir = std::env::temp_dir().join("medianest-test-qh");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        std::fs::write(&a, vec![1u8; 3 * 1024 * 1024]).unwrap();
        std::fs::write(&b, vec![1u8; 3 * 1024 * 1024 + 1]).unwrap();
        assert_ne!(quick_hash(&a).unwrap(), quick_hash(&b).unwrap());
        assert_eq!(quick_hash(&a).unwrap(), quick_hash(&a).unwrap());
    }

    #[test]
    fn photo_pipeline_generates_thumb_and_blurhash() {
        let dir = std::env::temp_dir().join("medianest-test-photo");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("thumbs")).unwrap();
        let img = image::DynamicImage::new_rgb8(640, 480);
        img.save(dir.join("test.png")).unwrap();

        let done = process(&dir, &dir.join("thumbs"), 1, "test.png", 0);
        assert!(done.error.is_none(), "{:?}", done.error);
        assert_eq!(done.thumb, 1);
        assert_eq!(done.width, Some(640));
        assert!(done.blurhash.is_some());
        let hash_hex = hex(done.hash.as_ref().unwrap());
        assert!(thumb_path(&dir.join("thumbs"), &hash_hex).exists());
    }
}
