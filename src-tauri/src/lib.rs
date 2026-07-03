pub mod catalog;
pub mod crypto;
pub mod db;
pub mod dedup;
pub mod import;
pub mod indexer;
pub mod ops;
pub mod protected;
pub mod video;

/// Deterministyczny obraz testowy o wyraźnej strukturze niskoczęstotliwościowej
/// zależnej od ziarna (duży prostokąt + kierunek gradientu) — pHash pracuje na
/// niskich częstotliwościach.
#[cfg(test)]
pub fn indexer_test_image(w: u32, h: u32, seed: u32) -> image::DynamicImage {
    let s = seed.wrapping_mul(2654435761);
    let rx = (s >> 4) % (w / 2);
    let ry = (s >> 12) % (h / 2);
    let rw = w / 4 + ((s >> 20) % (w / 4));
    let rh = h / 4 + ((s >> 24) % (h / 4));
    let dir = s % 4;
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
        let inside = x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        let t = (match dir {
            0 => x * 255 / w,
            1 => y * 255 / h,
            2 => (x + y) * 255 / (w + h),
            _ => (x * 255 / w + 255 - y * 255 / h) / 2,
        }) as u8;
        if inside {
            image::Rgb([255 - t, (s >> 8) as u8, t])
        } else {
            image::Rgb([t, t / 2, (s >> 16) as u8])
        }
    }))
}

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indexer::IndexerCtl;
use rusqlite::Connection;
use tauri::{Emitter, Manager};

pub struct AppState {
    pub db: Mutex<Connection>,
}

#[derive(serde::Serialize)]
struct Settings {
    library_path: Option<String>,
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Settings {
    let conn = state.db.lock().unwrap();
    Settings {
        library_path: db::get_setting(&conn, "library_path"),
    }
}

#[tauri::command]
fn set_library_path(
    state: tauri::State<AppState>,
    ctl: tauri::State<Arc<IndexerCtl>>,
    path: String,
) -> Result<(), String> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(format!("Folder nie istnieje: {path}"));
    }
    let conn = state.db.lock().unwrap();
    db::set_setting(&conn, "library_path", &path).map_err(|e| e.to_string())?;
    // ponytail: zmiana biblioteki czyści indeks; osierocone miniaturki w cache są
    // nieszkodliwe (adresowane hashem treści) — sprzątanie cache w Fazie 7
    conn.execute("DELETE FROM files", []).map_err(|e| e.to_string())?;
    ctl.rescan.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

#[derive(serde::Serialize)]
struct FileItem {
    id: i64,
    name: String,
    kind: i64,
    blurhash: Option<String>,
    thumb: Option<String>,
    duration: Option<f64>,
    rating: i64,
    protected_album: Option<i64>,
    locked: bool,
}

#[derive(serde::Serialize)]
struct FileInfo {
    id: i64,
    path: String,
    name: String,
    kind: i64,
    size: i64,
    width: Option<i64>,
    height: Option<i64>,
    duration: Option<f64>,
    taken_at: Option<i64>,
}


#[tauri::command]
fn count_files(state: tauri::State<AppState>, q: catalog::ListQuery) -> Result<i64, String> {
    let conn = state.db.lock().unwrap();
    let (where_sql, params) = catalog::build_where(&q);
    conn.query_row(
        &format!("SELECT count(*) FROM files f WHERE {where_sql}"),
        rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_files(
    state: tauri::State<AppState>,
    keys: tauri::State<protected::SessionKeys>,
    q: catalog::ListQuery,
    offset: i64,
    limit: i64,
) -> Result<Vec<FileItem>, String> {
    let unlocked: std::collections::HashSet<i64> = keys.unlocked_ids().into_iter().collect();
    let conn = state.db.lock().unwrap();
    let (where_sql, mut params) = catalog::build_where(&q);
    let (o, l) = (params.len() + 1, params.len() + 2);
    params.push(Box::new(offset));
    params.push(Box::new(limit));
    let sql = format!(
        "SELECT f.id, f.name, f.kind, f.blurhash,
                CASE WHEN f.thumb = 1 THEN lower(hex(f.hash)) END, f.duration, f.rating,
                f.protected_album
         FROM files f WHERE {where_sql}
         ORDER BY {} LIMIT ?{l} OFFSET ?{o}",
        catalog::order_clause(&q.sort)
    );
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
            |r| {
                let protected_album: Option<i64> = r.get(7)?;
                let locked =
                    protected_album.map(|a| !unlocked.contains(&a)).unwrap_or(false);
                Ok(FileItem {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    blurhash: r.get(3)?,
                    thumb: r.get(4)?,
                    duration: r.get(5)?,
                    rating: r.get(6)?,
                    protected_album,
                    locked,
                })
            },
        )
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

// ── Oceny, tagi, albumy, oś czasu ───────────────────────────────────────

#[tauri::command]
fn set_rating(state: tauri::State<AppState>, id: i64, rating: i64) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    catalog::set_rating(&conn, id, rating).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_tags(state: tauri::State<AppState>) -> Vec<(i64, String, i64)> {
    let conn = state.db.lock().unwrap();
    catalog::list_tags(&conn)
}

#[tauri::command]
fn get_file_tags(state: tauri::State<AppState>, id: i64) -> Vec<(i64, String)> {
    let conn = state.db.lock().unwrap();
    catalog::file_tags(&conn, id)
}

#[tauri::command]
fn tag_file(state: tauri::State<AppState>, id: i64, name: String) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    catalog::tag_file(&conn, id, &name).map_err(|e| e.to_string())
}

#[tauri::command]
fn untag_file(state: tauri::State<AppState>, id: i64, tag_id: i64) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    catalog::untag_file(&conn, id, tag_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_albums(state: tauri::State<AppState>) -> Vec<catalog::Album> {
    let conn = state.db.lock().unwrap();
    catalog::list_albums(&conn)
}

#[tauri::command]
fn create_album(
    state: tauri::State<AppState>,
    name: String,
    album_type: String,
) -> Result<i64, String> {
    let conn = state.db.lock().unwrap();
    let root = db::get_setting(&conn, "library_path").ok_or("brak biblioteki")?;
    catalog::create_album(&conn, std::path::Path::new(&root), &name, &album_type)
}

#[tauri::command]
fn delete_album(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    // chroniony album z plikami: najpierw trzeba je odszyfrować (wyjąć),
    // inaczej pliki zostałyby zaszyfrowane bez klucza w bazie
    let protected_files: i64 = conn
        .query_row(
            "SELECT count(*) FROM files WHERE protected_album = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if protected_files > 0 {
        return Err(format!(
            "Album zawiera {protected_files} zaszyfrowanych plików — najpierw je wyjmij (odszyfruj)"
        ));
    }
    catalog::delete_album(&conn, id)
}

#[tauri::command]
fn add_to_album(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    keys: tauri::State<protected::SessionKeys>,
    album_id: i64,
    file_ids: Vec<i64>,
) -> Result<u64, String> {
    let conn = state.db.lock().unwrap();
    let root = db::get_setting(&conn, "library_path").ok_or("brak biblioteki")?;
    let album_type: String = conn
        .query_row("SELECT type FROM albums WHERE id = ?1", [album_id], |r| r.get(0))
        .map_err(|_| "Album nie istnieje".to_string())?;
    let n = if album_type == "protected" {
        let key = keys
            .get(album_id)
            .ok_or("Album jest zablokowany — odblokuj go hasłem przed dodaniem plików")?;
        let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        protected::add_files(
            &conn,
            std::path::Path::new(&root),
            &data_dir.join("thumbs"),
            &key,
            album_id,
            &file_ids,
        )?
    } else {
        catalog::add_to_album(&conn, std::path::Path::new(&root), album_id, &file_ids)?
    };
    drop(conn);
    app.emit("library-changed", ()).ok();
    Ok(n)
}

#[tauri::command]
fn remove_from_album(
    state: tauri::State<AppState>,
    album_id: i64,
    file_ids: Vec<i64>,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    catalog::remove_from_album(&conn, album_id, &file_ids)
}

#[tauri::command]
fn timeline_months(state: tauri::State<AppState>) -> Vec<(String, i64)> {
    let conn = state.db.lock().unwrap();
    catalog::timeline_months(&conn)
}

#[tauri::command]
fn timeline_histogram(
    state: tauri::State<AppState>,
    q: catalog::ListQuery,
) -> Vec<(String, i64)> {
    let conn = state.db.lock().unwrap();
    catalog::timeline_for_query(&conn, &q)
}

#[tauri::command]
fn library_stats(state: tauri::State<AppState>) -> catalog::LibraryStats {
    let conn = state.db.lock().unwrap();
    catalog::library_stats(&conn)
}

#[tauri::command]
fn reconcile_missing(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
) -> Result<(u64, u64), String> {
    let conn = state.db.lock().unwrap();
    let result = catalog::reconcile_missing(&conn)?;
    drop(conn);
    app.emit("library-changed", ()).ok();
    Ok(result)
}

// ── Albumy chronione ────────────────────────────────────────────────────

#[tauri::command]
fn create_protected_album(
    state: tauri::State<AppState>,
    keys: tauri::State<protected::SessionKeys>,
    name: String,
    password: String,
) -> Result<i64, String> {
    let conn = state.db.lock().unwrap();
    let (id, key) = protected::create_album(&conn, &name, &password)?;
    keys.insert(id, key); // świeżo utworzony album jest odblokowany
    Ok(id)
}

#[tauri::command]
fn unlock_album(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    keys: tauri::State<protected::SessionKeys>,
    id: i64,
    password: String,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    let key = protected::unlock(&conn, id, &password)?;
    keys.insert(id, key);
    drop(conn);
    app.emit("library-changed", ()).ok();
    Ok(())
}

#[tauri::command]
fn lock_albums(app: tauri::AppHandle, keys: tauri::State<protected::SessionKeys>) {
    keys.lock_all();
    app.emit("library-changed", ()).ok();
}

#[tauri::command]
fn unlocked_albums(keys: tauri::State<protected::SessionKeys>) -> Vec<i64> {
    keys.unlocked_ids()
}

#[tauri::command]
fn unprotect_files(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    keys: tauri::State<protected::SessionKeys>,
    album_id: i64,
    file_ids: Vec<i64>,
) -> Result<u64, String> {
    let key = keys.get(album_id).ok_or("Album jest zablokowany")?;
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let conn = state.db.lock().unwrap();
    let root = db::get_setting(&conn, "library_path").ok_or("brak biblioteki")?;
    let n = protected::remove_files(
        &conn,
        std::path::Path::new(&root),
        &data_dir.join("thumbs"),
        &key,
        album_id,
        &file_ids,
    )?;
    drop(conn);
    app.emit("library-changed", ()).ok();
    Ok(n)
}

#[tauri::command]
fn change_album_password(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    keys: tauri::State<protected::SessionKeys>,
    album_id: i64,
    new_password: String,
) -> Result<(), String> {
    let old_key = keys.get(album_id).ok_or("Album jest zablokowany")?;
    if new_password.len() < 4 {
        return Err("Hasło musi mieć co najmniej 4 znaki".into());
    }
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let conn = state.db.lock().unwrap();
    let root = db::get_setting(&conn, "library_path").ok_or("brak biblioteki")?;
    let new_key = protected::change_password(
        &conn,
        std::path::Path::new(&root),
        &data_dir.join("thumbs"),
        &old_key,
        album_id,
        &new_password,
    )?;
    keys.insert(album_id, new_key);
    Ok(())
}

#[tauri::command]
fn get_file_info(state: tauri::State<AppState>, id: i64) -> Result<FileInfo, String> {
    let conn = state.db.lock().unwrap();
    conn.query_row(
        "SELECT id, path, name, kind, size, width, height, duration, taken_at
         FROM files WHERE id = ?1",
        [id],
        |r| {
            Ok(FileInfo {
                id: r.get(0)?,
                path: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                size: r.get(4)?,
                width: r.get(5)?,
                height: r.get(6)?,
                duration: r.get(7)?,
                taken_at: r.get(8)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_folders(state: tauri::State<AppState>) -> Result<Vec<(String, i64)>, String> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn
        .prepare_cached("SELECT parent, count(*) FROM files WHERE status = 0 GROUP BY parent")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[tauri::command]
fn set_focus_folder(ctl: tauri::State<Arc<IndexerCtl>>, parent: Option<String>) {
    *ctl.focus.lock().unwrap() = parent;
}

#[tauri::command]
fn rescan(ctl: tauri::State<Arc<IndexerCtl>>) {
    ctl.rescan.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
fn dedup_scan(state: tauri::State<AppState>, threshold: u32) -> Vec<dedup::DupGroup> {
    let conn = state.db.lock().unwrap();
    dedup::scan(&conn, threshold)
}

// ── Import ──────────────────────────────────────────────────────────────

#[tauri::command]
fn import_plan(
    source: String,
    photo_template: String,
    video_template: String,
) -> Result<import::ImportPlan, String> {
    let path = std::path::PathBuf::from(&source);
    if !path.is_dir() {
        return Err(format!("Folder nie istnieje: {source}"));
    }
    Ok(import::plan(&path, &photo_template, &video_template))
}

#[tauri::command]
fn import_run(
    app: tauri::AppHandle,
    source: String,
    photo_template: String,
    video_template: String,
) {
    std::thread::spawn(move || {
        import::run(app, source.into(), photo_template, video_template)
    });
}

#[tauri::command]
fn list_import_pending(state: tauri::State<AppState>) -> Vec<import::PendingItem> {
    let conn = state.db.lock().unwrap();
    import::list_pending(&conn)
}

#[tauri::command]
fn resolve_import_pending(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    ids: Vec<i64>,
    action: String,
) -> Result<(), String> {
    let conn = state.db.lock().unwrap();
    let root = db::get_setting(&conn, "library_path").ok_or("brak biblioteki")?;
    import::resolve_pending(&conn, std::path::Path::new(&root), &ids, &action)?;
    drop(conn);
    app.emit("library-changed", ()).ok();
    Ok(())
}

// ── Kosz i historia operacji ────────────────────────────────────────────

fn with_paths<T>(
    app: &tauri::AppHandle,
    state: &tauri::State<AppState>,
    f: impl FnOnce(&rusqlite::Connection, &std::path::Path, &std::path::Path) -> Result<T, String>,
) -> Result<T, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let conn = state.db.lock().unwrap();
    let root = db::get_setting(&conn, "library_path").ok_or("brak biblioteki")?;
    f(&conn, std::path::Path::new(&root), &data_dir)
}

#[tauri::command]
fn trash_files(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    ids: Vec<i64>,
) -> Result<u64, String> {
    let n = with_paths(&app, &state, |conn, root, data| {
        ops::trash_files(conn, root, data, &ids, "trash")
    })?;
    app.emit("library-changed", ()).ok();
    Ok(n)
}

#[tauri::command]
fn list_trash(state: tauri::State<AppState>) -> Vec<ops::TrashItem> {
    let conn = state.db.lock().unwrap();
    ops::list_trash(&conn)
}

#[tauri::command]
fn restore_trash(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    ids: Vec<i64>,
) -> Result<u64, String> {
    let n = with_paths(&app, &state, |conn, root, data| {
        ops::restore_trash(conn, root, data, &ids)
    })?;
    app.emit("library-changed", ()).ok();
    Ok(n)
}

#[tauri::command]
fn empty_trash(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    ids: Option<Vec<i64>>,
) -> Result<u64, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let conn = state.db.lock().unwrap();
    ops::empty_trash(&conn, &data_dir, ids.as_deref())
}

#[tauri::command]
fn list_operations(state: tauri::State<AppState>) -> Vec<ops::Operation> {
    let conn = state.db.lock().unwrap();
    ops::list_operations(&conn)
}

#[tauri::command]
fn undo_operation(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: i64,
) -> Result<String, String> {
    let msg = with_paths(&app, &state, |conn, root, data| {
        ops::undo_operation(conn, root, data, id)
    })?;
    app.emit("library-changed", ()).ok();
    Ok(msg)
}

/// Wykrywanie nowych nośników: nowa litera dysku ≈ podpięta karta/pendrive.
/// ponytail: heurystyka nowych liter zamiast GetDriveTypeW — zero zależności;
/// API Windows, jeśli heurystyka zacznie łapać dyski sieciowe.
fn drive_watch_loop(app: tauri::AppHandle) {
    let letters = || -> std::collections::HashSet<char> {
        ('D'..='Z')
            .filter(|c| std::path::Path::new(&format!("{c}:\\")).exists())
            .collect()
    };
    let mut known = letters();
    loop {
        std::thread::sleep(Duration::from_secs(3));
        let current = letters();
        for c in current.difference(&known) {
            app.emit("drive-added", format!("{c}:\\")).ok();
        }
        known = current;
    }
}

/// Obserwuje folder biblioteki; każda zmiana (z debounce 2 s) uruchamia skan
/// przyrostowy. ponytail: pełny rescan zamiast celowanych upsertów —
/// diff po HashMap i tak trwa sekundy; celowane upserty gdy skan zacznie ciążyć.
fn watch_loop(app: tauri::AppHandle) {
    use notify_debouncer_full::notify::RecursiveMode;

    let ctl = app.state::<Arc<IndexerCtl>>().inner().clone();
    let data_dir = app.path().app_data_dir().expect("app data dir");
    let conn = db::open(&data_dir).expect("watcher db");

    let ctl_events = ctl.clone();
    let mut debouncer = notify_debouncer_full::new_debouncer(
        Duration::from_secs(2),
        None,
        move |result: notify_debouncer_full::DebounceEventResult| {
            if result.is_ok() {
                ctl_events
                    .rescan
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
        },
    )
    .expect("debouncer");

    let mut watched: Option<PathBuf> = None;
    loop {
        let root = db::get_setting(&conn, "library_path").map(PathBuf::from);
        if root != watched {
            if let Some(old) = &watched {
                debouncer.unwatch(old).ok();
            }
            if let Some(new) = &root {
                debouncer.watch(new, RecursiveMode::Recursive).ok();
            }
            watched = root;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

const MEDIA_CHUNK: u64 = 8 * 1024 * 1024;

fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tiff" | "tif" => "image/tiff",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        "mts" => "video/mp2t",
        "3gp" => "video/3gpp",
        _ => "application/octet-stream",
    }
}

/// Serwuje oryginalne pliki przez media://<id> z obsługą Range —
/// wymagane do przewijania wideo; chunk ograniczony do 8 MB pamięci.
fn serve_media(
    app: &tauri::AppHandle,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};

    let not_found = || {
        tauri::http::Response::builder()
            .status(404)
            .body(Vec::new())
            .unwrap()
    };
    let url_path = request.uri().path().trim_start_matches('/').to_string();
    let mut protected_key: Option<crypto::Key> = None;
    let abs = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        // media://pending/<id> — podgląd pliku źródłowego z poczekalni importu
        if let Some(pid) = url_path.strip_prefix("pending/") {
            let Ok(pid) = pid.parse::<i64>() else {
                return not_found();
            };
            let Ok(src) = conn.query_row(
                "SELECT src FROM import_pending WHERE id = ?1",
                [pid],
                |r| r.get::<_, String>(0),
            ) else {
                return not_found();
            };
            PathBuf::from(src)
        } else {
            let Ok(id) = url_path.parse::<i64>() else {
                return not_found();
            };
            let Some(root) = db::get_setting(&conn, "library_path") else {
                return not_found();
            };
            let Ok((rel, album)) = conn.query_row(
                "SELECT path, protected_album FROM files WHERE id = ?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)),
            ) else {
                return not_found();
            };
            if let Some(album) = album {
                // plik chroniony: wymagany klucz sesyjny odblokowanego albumu
                match app.state::<protected::SessionKeys>().get(album) {
                    Some(key) => protected_key = Some(key),
                    None => {
                        return tauri::http::Response::builder()
                            .status(403)
                            .body(Vec::new())
                            .unwrap();
                    }
                }
            }
            PathBuf::from(root).join(rel)
        }
    };

    // odszyfrowywanie w locie z dostępem swobodnym (przewijanie wideo działa)
    if let Some(key) = protected_key {
        let Ok(mut enc) = crypto::EncryptedFile::open(&key, &abs) else {
            return not_found();
        };
        let len = enc.plaintext_len;
        let plain_name = abs
            .to_string_lossy()
            .trim_end_matches(".mnlock")
            .to_string();
        let mime = mime_for(&plain_name);
        let range = request
            .headers()
            .get("range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes="))
            .and_then(|v| {
                let (start, end) = v.split_once('-')?;
                let start: u64 = start.parse().ok()?;
                let end: u64 = end.parse().unwrap_or(len.saturating_sub(1));
                Some((start, end.min(len.saturating_sub(1))))
            });
        return match range {
            Some((start, end)) if start < len => {
                let end = end.min(start + MEDIA_CHUNK - 1);
                let Ok(buf) = enc.read_range(start, (end - start + 1) as usize) else {
                    return not_found();
                };
                tauri::http::Response::builder()
                    .status(206)
                    .header("Content-Type", mime)
                    .header("Accept-Ranges", "bytes")
                    .header("Cache-Control", "no-store")
                    .header("Content-Range", format!("bytes {start}-{end}/{len}"))
                    .body(buf)
                    .unwrap()
            }
            _ => {
                let Ok(buf) = enc.read_all() else {
                    return not_found();
                };
                tauri::http::Response::builder()
                    .header("Content-Type", mime)
                    .header("Accept-Ranges", "bytes")
                    .header("Cache-Control", "no-store")
                    .body(buf)
                    .unwrap()
            }
        };
    }
    let Ok(mut file) = std::fs::File::open(&abs) else {
        return not_found();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mime = mime_for(&abs.to_string_lossy());

    let range = request
        .headers()
        .get("range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes="))
        .and_then(|v| {
            let (start, end) = v.split_once('-')?;
            let start: u64 = start.parse().ok()?;
            let end: u64 = end.parse().unwrap_or(len.saturating_sub(1));
            Some((start, end.min(len.saturating_sub(1))))
        });

    match range {
        Some((start, end)) if start < len => {
            let end = end.min(start + MEDIA_CHUNK - 1);
            let mut buf = vec![0u8; (end - start + 1) as usize];
            if file.seek(SeekFrom::Start(start)).is_err() || file.read_exact(&mut buf).is_err() {
                return not_found();
            }
            tauri::http::Response::builder()
                .status(206)
                .header("Content-Type", mime)
                .header("Accept-Ranges", "bytes")
                .header("Content-Range", format!("bytes {start}-{end}/{len}"))
                .body(buf)
                .unwrap()
        }
        _ => {
            let mut buf = Vec::new();
            if file.read_to_end(&mut buf).is_err() {
                return not_found();
            }
            tauri::http::Response::builder()
                .header("Content-Type", mime)
                .header("Accept-Ranges", "bytes")
                .body(buf)
                .unwrap()
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .register_uri_scheme_protocol("thumb", |ctx, request| {
            let not_found = || {
                tauri::http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .unwrap()
            };
            let path = request.uri().path().trim_start_matches('/');
            let (hash, is_sprite) = match path.strip_suffix(".sprite") {
                Some(h) => (h, true),
                None => (path, false),
            };
            if hash.len() < 3 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return not_found();
            }
            let Ok(data_dir) = ctx.app_handle().path().app_data_dir() else {
                return not_found();
            };
            let thumbs = data_dir.join("thumbs");
            let file = if is_sprite {
                indexer::sprite_path(&thumbs, hash)
            } else {
                indexer::thumb_path(&thumbs, hash)
            };
            let ok = |bytes: Vec<u8>| {
                tauri::http::Response::builder()
                    .header("Content-Type", "image/webp")
                    .header("Cache-Control", "public, max-age=31536000, immutable")
                    .body(bytes)
                    .unwrap()
            };
            if let Ok(bytes) = std::fs::read(&file) {
                return ok(bytes);
            }
            // zaszyfrowana miniaturka pliku chronionego — tylko dla odblokowanego albumu
            let locked_file = file.with_extension("webp.mnlock");
            if !is_sprite && locked_file.exists() {
                let app = ctx.app_handle();
                let Ok(hash_blob) = (0..hash.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hash[i..i + 2], 16))
                    .collect::<Result<Vec<u8>, _>>()
                else {
                    return not_found();
                };
                let album: Option<i64> = {
                    let state = app.state::<AppState>();
                    let conn = state.db.lock().unwrap();
                    conn.query_row(
                        "SELECT protected_album FROM files WHERE hash = ?1 AND protected_album IS NOT NULL LIMIT 1",
                        rusqlite::params![hash_blob],
                        |r| r.get(0),
                    )
                    .ok()
                    .flatten()
                };
                if let Some(key) = album.and_then(|a| app.state::<protected::SessionKeys>().get(a)) {
                    if let Ok(mut enc) = crypto::EncryptedFile::open(&key, &locked_file) {
                        if let Ok(bytes) = enc.read_all() {
                            // Cache-Control: no-store — odszyfrowana treść nie może
                            // zostać w cache po zablokowaniu albumu
                            return tauri::http::Response::builder()
                                .header("Content-Type", "image/webp")
                                .header("Cache-Control", "no-store")
                                .body(bytes)
                                .unwrap();
                        }
                    }
                }
            }
            not_found()
        })
        .register_uri_scheme_protocol("media", |ctx, request| {
            serve_media(ctx.app_handle(), request)
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let conn = db::open(&data_dir)?;
            app.manage(AppState {
                db: Mutex::new(conn),
            });
            app.manage(Arc::new(IndexerCtl::default()));
            app.manage(protected::SessionKeys::default());

            std::thread::spawn(video::init);
            let handle = app.handle().clone();
            std::thread::spawn(move || indexer::run(handle));
            let handle = app.handle().clone();
            std::thread::spawn(move || watch_loop(handle));
            let handle = app.handle().clone();
            std::thread::spawn(move || drive_watch_loop(handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_library_path,
            count_files,
            list_files,
            get_file_info,
            list_folders,
            set_focus_folder,
            rescan,
            dedup_scan,
            set_rating,
            list_tags,
            get_file_tags,
            tag_file,
            untag_file,
            list_albums,
            create_album,
            delete_album,
            add_to_album,
            remove_from_album,
            timeline_months,
            timeline_histogram,
            library_stats,
            reconcile_missing,
            create_protected_album,
            unlock_album,
            lock_albums,
            unlocked_albums,
            unprotect_files,
            change_album_password,
            import_plan,
            import_run,
            list_import_pending,
            resolve_import_pending,
            trash_files,
            list_trash,
            restore_trash,
            empty_trash,
            list_operations,
            undo_operation
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
