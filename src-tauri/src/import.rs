//! Import plików z zewnętrznych folderów/nośników: szablony organizacji,
//! weryfikacja hashem, poczekalnia duplikatów, log operacji.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use tauri::{AppHandle, Emitter, Manager};

use crate::db;
use crate::indexer;

/// Rozwiązuje szablon ścieżki docelowej. Tokeny: {rok} {miesiac} {dzien} {typ} {folder}
pub fn apply_template(template: &str, taken_at: i64, kind: i64, src_folder: &str) -> String {
    let dt = chrono::DateTime::from_timestamp(taken_at, 0).unwrap_or_default();
    let fmt = |f: &str| dt.format(f).to_string();
    template
        .replace("{rok}", &fmt("%Y"))
        .replace("{miesiac}", &fmt("%m"))
        .replace("{dzien}", &fmt("%d"))
        .replace("{typ}", if kind == 0 { "Zdjęcia" } else { "Wideo" })
        .replace("{folder}", src_folder)
        .trim_matches('/')
        .to_string()
}

struct SourceFile {
    abs: PathBuf,
    kind: i64,
    size: i64,
    dst_dir: String,
}

fn scan_source(
    source: &Path,
    photo_template: &str,
    video_template: &str,
) -> Vec<SourceFile> {
    walkdir::WalkDir::new(source)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let kind = indexer::kind_of(e.path())?;
            let meta = e.metadata().ok()?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            // dla zdjęć data z EXIF, fallback mtime
            let taken_at = if kind == 0 {
                indexer::read_exif(e.path()).1.unwrap_or(mtime)
            } else {
                mtime
            };
            let src_folder = e
                .path()
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let template = if kind == 0 { photo_template } else { video_template };
            Some(SourceFile {
                abs: e.path().to_path_buf(),
                kind,
                size: meta.len() as i64,
                dst_dir: apply_template(template, taken_at, kind, &src_folder),
            })
        })
        .collect()
}

#[derive(serde::Serialize)]
pub struct ImportPlan {
    pub total: u64,
    pub total_size: i64,
    pub photos: u64,
    pub videos: u64,
    pub tree: Vec<(String, u64)>,
}

pub fn plan(source: &Path, photo_template: &str, video_template: &str) -> ImportPlan {
    let files = scan_source(source, photo_template, video_template);
    let mut tree: BTreeMap<String, u64> = BTreeMap::new();
    let (mut photos, mut videos, mut total_size) = (0u64, 0u64, 0i64);
    for f in &files {
        *tree.entry(f.dst_dir.clone()).or_default() += 1;
        total_size += f.size;
        if f.kind == 0 {
            photos += 1;
        } else {
            videos += 1;
        }
    }
    ImportPlan {
        total: files.len() as u64,
        total_size,
        photos,
        videos,
        tree: tree.into_iter().collect(),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn hash_of(path: &Path, kind: i64) -> std::io::Result<Vec<u8>> {
    if kind == 0 {
        indexer::full_hash(path)
    } else {
        indexer::quick_hash(path)
    }
}

/// Unikalna ścieżka docelowa — kolizje nazw dostają sufiks _1, _2, …
pub(crate) fn unique_dst(root: &Path, dir: &str, name: &str) -> (PathBuf, String) {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    for i in 0..1000 {
        let candidate = if i == 0 {
            format!("{stem}{ext}")
        } else {
            format!("{stem}_{i}{ext}")
        };
        let rel = if dir.is_empty() {
            candidate.clone()
        } else {
            format!("{dir}/{candidate}")
        };
        let abs = root.join(&rel);
        if !abs.exists() {
            return (abs, rel);
        }
    }
    unreachable!("1000 kolizji nazw");
}

/// Kopiuje z weryfikacją i rejestruje plik w bibliotece. Zwraca dst_rel.
fn copy_verified(
    conn: &Connection,
    root: &Path,
    src: &Path,
    kind: i64,
    src_hash: &[u8],
    dst_dir: &str,
    op_id: i64,
) -> anyhow::Result<String> {
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "plik".into());
    let (dst_abs, dst_rel) = unique_dst(root, dst_dir, &name);
    std::fs::create_dir_all(dst_abs.parent().unwrap())?;
    std::fs::copy(src, &dst_abs)?;
    // ponytail: kopiuj + przelicz hash celu; strumieniowe hashowanie w trakcie
    // kopiowania, gdy import stanie się wąskim gardłem
    let dst_hash = hash_of(&dst_abs, kind)?;
    if dst_hash != src_hash {
        std::fs::remove_file(&dst_abs).ok();
        anyhow::bail!("weryfikacja hash nie powiodła się: {}", src.display());
    }
    let meta = std::fs::metadata(&dst_abs)?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let parent = dst_rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    let fname = dst_rel.rsplit('/').next().unwrap_or(&dst_rel);
    conn.execute(
        "INSERT INTO files (path, parent, name, kind, size, mtime, hash)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(path) DO UPDATE SET size=excluded.size, mtime=excluded.mtime,
             hash=excluded.hash, thumb=0, status=0",
        rusqlite::params![dst_rel, parent, fname, kind, meta.len() as i64, mtime, src_hash],
    )?;
    conn.execute(
        "INSERT INTO operation_items (op_id, src, dst) VALUES (?1, ?2, ?3)",
        rusqlite::params![op_id, src.to_string_lossy(), dst_rel],
    )?;
    Ok(dst_rel)
}

#[derive(serde::Serialize, Clone)]
pub struct ImportProgress {
    pub done: u64,
    pub total: u64,
    /// pliki bez duplikatu wykryte podczas skanu (jeszcze NIE skopiowane)
    pub new_files: u64,
    pub duplicates: u64,
    pub errors: u64,
}

/// Rdzeń importu — skanuje, liczy hash i wykrywa duplikaty, ale NIC nie kopiuje.
/// Każdy plik (nowy lub duplikat) trafia do poczekalni `import_pending`; o tym,
/// co ostatecznie ląduje w bibliotece, decyduje użytkownik przez `resolve_pending`.
/// Bez zależności od Tauri, testowalny headless.
pub fn run_core(
    conn: &Connection,
    source: &Path,
    photo_template: &str,
    video_template: &str,
    mut on_progress: impl FnMut(&ImportProgress),
) -> ImportProgress {
    let files = scan_source(source, photo_template, video_template);
    let total = files.len() as u64;

    let mut progress = ImportProgress {
        done: 0,
        total,
        new_files: 0,
        duplicates: 0,
        errors: 0,
    };

    for f in files {
        let result: anyhow::Result<()> = (|| {
            let hash = hash_of(&f.abs, f.kind)?;
            let dup: Option<i64> = conn
                .query_row(
                    "SELECT id FROM files WHERE hash = ?1 AND status = 0 LIMIT 1",
                    rusqlite::params![hash],
                    |r| r.get(0),
                )
                .ok();
            // nowy plik i duplikat trafiają do tej samej poczekalni; dup_file_id
            // rozróżnia przypadki. Plik zostaje w źródle — kopia dopiero na wybór.
            conn.execute(
                "INSERT OR IGNORE INTO import_pending
                 (src, dst, hash, dup_file_id, kind, size, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![
                    f.abs.to_string_lossy(),
                    f.dst_dir,
                    hash,
                    dup,
                    f.kind,
                    f.size,
                    now()
                ],
            )?;
            if dup.is_some() {
                progress.duplicates += 1;
            } else {
                progress.new_files += 1;
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("import error {}: {e}", f.abs.display());
            progress.errors += 1;
        }
        progress.done += 1;
        if progress.done % 5 == 0 || progress.done == total {
            on_progress(&progress);
        }
    }
    progress
}

/// Właściwy import — uruchamiany w wątku, raportuje eventami Tauri.
pub fn run(app: AppHandle, source: PathBuf, photo_template: String, video_template: String) {
    let data_dir = app.path().app_data_dir().expect("app data dir");
    let conn = db::open(&data_dir).expect("import db");
    // biblioteka musi być ustawiona, żeby resolve_pending miał gdzie kopiować
    if db::get_setting(&conn, "library_path").is_none() {
        return;
    }
    let app2 = app.clone();
    let progress = run_core(&conn, &source, &photo_template, &video_template, move |p| {
        app2.emit("import-progress", p.clone()).ok();
    });
    app.emit("import-done", progress).ok();
}

#[derive(serde::Serialize)]
pub struct PendingItem {
    pub id: i64,
    pub src: String,
    pub dst: String,
    pub size: i64,
    pub kind: i64,
    /// pola `dup_*` wypełnione tylko dla duplikatów (istnieje wiersz w `files`);
    /// dla nowych plików są `None`
    pub dup_id: Option<i64>,
    pub dup_path: Option<String>,
    pub dup_name: Option<String>,
    pub dup_size: Option<i64>,
    pub dup_thumb: Option<String>,
    pub dup_taken_at: Option<i64>,
}

pub fn list_pending(conn: &Connection) -> Vec<PendingItem> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT p.id, p.src, p.dst, p.size, p.kind, f.id, f.path, f.name, f.size,
                    CASE WHEN f.thumb = 1 THEN lower(hex(f.hash)) END, f.taken_at
             FROM import_pending p LEFT JOIN files f ON f.id = p.dup_file_id
             ORDER BY p.dup_file_id IS NULL DESC, p.id",
        )
        .expect("stmt");
    stmt.query_map([], |r| {
        Ok(PendingItem {
            id: r.get(0)?,
            src: r.get(1)?,
            dst: r.get(2)?,
            size: r.get(3)?,
            kind: r.get(4)?,
            dup_id: r.get(5)?,
            dup_path: r.get(6)?,
            dup_name: r.get(7)?,
            dup_size: r.get(8)?,
            dup_thumb: r.get(9)?,
            dup_taken_at: r.get(10)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// Rozstrzyga pozycje poczekalni: "import" | "skip" | "delete_source".
pub fn resolve_pending(
    conn: &Connection,
    root: &Path,
    ids: &[i64],
    action: &str,
) -> Result<(), String> {
    let op_id = if action == "import" {
        conn.execute(
            "INSERT INTO operations (kind, label, created_at) VALUES ('import', 'poczekalnia', ?1)",
            rusqlite::params![now()],
        )
        .map_err(|e| e.to_string())?;
        conn.last_insert_rowid()
    } else {
        0
    };
    for id in ids {
        let row: Option<(String, String, Vec<u8>, i64)> = conn
            .query_row(
                "SELECT src, dst, hash, kind FROM import_pending WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        let Some((src, dst, hash, kind)) = row else {
            continue;
        };
        match action {
            "import" => {
                copy_verified(conn, root, Path::new(&src), kind, &hash, &dst, op_id)
                    .map_err(|e| e.to_string())?;
            }
            "delete_source" => {
                // do systemowego kosza — źródło może być cenne
                trash::delete(&src).map_err(|e| e.to_string())?;
            }
            _ => {} // skip — tylko usunięcie z poczekalni
        }
        conn.execute("DELETE FROM import_pending WHERE id = ?1", [id])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer;

    #[test]
    fn template_tokens() {
        // 2023-07-15 12:00:00 UTC
        let ts = 1689422400;
        assert_eq!(apply_template("{rok}/{miesiac}", ts, 0, "DCIM"), "2023/07");
        assert_eq!(
            apply_template("{rok}/{typ}/{rok}-{miesiac}-{dzien}", ts, 1, "x"),
            "2023/Wideo/2023-07-15"
        );
        assert_eq!(apply_template("{folder}", ts, 0, "DCIM"), "DCIM");
        assert_eq!(apply_template("/{rok}/", ts, 0, ""), "2023");
    }

    #[test]
    fn import_full_flow_with_pending() {
        let base = std::env::temp_dir().join("medianest-test-import-flow");
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("lib");
        let source = base.join("sdcard/DCIM");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        let conn = crate::db::open(&base.join("data")).unwrap();

        // istniejący plik w bibliotece (zaindeksowany z hashem)
        let existing = b"istniejace-zdjecie".to_vec();
        std::fs::create_dir_all(root.join("2020/01")).unwrap();
        std::fs::write(root.join("2020/01/old.jpg"), &existing).unwrap();
        let hash = indexer::full_hash(&root.join("2020/01/old.jpg")).unwrap();
        conn.execute(
            "INSERT INTO files (path,parent,name,kind,size,mtime,hash,thumb)
             VALUES ('2020/01/old.jpg','2020/01','old.jpg',0,18,1,?1,1)",
            rusqlite::params![hash],
        )
        .unwrap();

        // źródło: 2 nowe pliki + 1 duplikat istniejącego
        std::fs::write(source.join("new1.jpg"), b"nowe-1").unwrap();
        std::fs::write(source.join("new2.jpg"), b"nowe-22").unwrap();
        std::fs::write(source.join("dup.jpg"), &existing).unwrap();

        let progress = run_core(&conn, &source, "{rok}/{miesiac}", "{rok}", |_| {});
        assert_eq!(progress.new_files, 2);
        assert_eq!(progress.duplicates, 1);
        assert_eq!(progress.errors, 0);

        // skan NIC nie kopiuje: wszystko czeka w poczekalni, biblioteka bez zmian
        let pending = list_pending(&conn);
        assert_eq!(pending.len(), 3); // 2 nowe + 1 duplikat
        let dups: Vec<&PendingItem> = pending.iter().filter(|p| p.dup_id.is_some()).collect();
        let news: Vec<&PendingItem> = pending.iter().filter(|p| p.dup_id.is_none()).collect();
        assert_eq!(dups.len(), 1);
        assert_eq!(news.len(), 2);
        assert_eq!(dups[0].dup_name.as_deref(), Some("old.jpg"));
        let files_count: i64 = conn
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(files_count, 1); // tylko istniejący old.jpg

        // "importuj" wybranych: 2 nowe + duplikat → kopiowane do biblioteki
        let new_ids: Vec<i64> = news.iter().map(|p| p.id).collect();
        resolve_pending(&conn, &root, &new_ids, "import").unwrap();
        resolve_pending(&conn, &root, &[dups[0].id], "import").unwrap();
        assert!(list_pending(&conn).is_empty());
        let files_count: i64 = conn
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(files_count, 4); // old + 2 nowe + duplikat
        // źródło nietknięte
        assert!(source.join("dup.jpg").exists());

        // ponowny import: wszystko to teraz duplikaty
        let progress = run_core(&conn, &source, "{rok}/{miesiac}", "{rok}", |_| {});
        assert_eq!(progress.new_files, 0);
        assert_eq!(progress.duplicates, 3);
        // "pomiń" czyści poczekalnię bez kopiowania
        let ids: Vec<i64> = list_pending(&conn).iter().map(|p| p.id).collect();
        resolve_pending(&conn, &root, &ids, "skip").unwrap();
        assert!(list_pending(&conn).is_empty());
    }

    #[test]
    fn unique_dst_suffixes_collisions() {
        let dir = std::env::temp_dir().join("medianest-test-unique");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        let (_, rel1) = unique_dst(&dir, "a", "x.jpg");
        assert_eq!(rel1, "a/x.jpg");
        std::fs::write(dir.join("a/x.jpg"), b"1").unwrap();
        let (_, rel2) = unique_dst(&dir, "a", "x.jpg");
        assert_eq!(rel2, "a/x_1.jpg");
    }
}
