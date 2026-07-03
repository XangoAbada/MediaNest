//! Zapytania katalogu: filtrowanie/wyszukiwanie plików, albumy, tagi, oceny.

use std::path::Path;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Wspólne filtry listy plików — jeden obiekt zamiast rosnącej listy parametrów.
#[derive(serde::Deserialize, Default, Clone)]
#[serde(default)]
pub struct ListQuery {
    pub parent: Option<String>,
    pub recursive: bool,
    pub query: Option<String>,   // FTS: nazwa/ścieżka/tagi
    pub kind: Option<i64>,       // 0 zdjęcia, 1 wideo
    pub rating_min: i64,
    pub tag_id: Option<i64>,
    pub album_id: Option<i64>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
    pub sort: String,
}

/// Zamienia frazę użytkownika na bezpieczne zapytanie FTS5 (prefiksowe).
fn fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .map(|t| {
            let clean: String = t.chars().filter(|c| c.is_alphanumeric()).collect();
            format!("\"{clean}\"*")
        })
        .filter(|t| t.len() > 3) // po odfiltrowaniu zostały znaki
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn build_where(q: &ListQuery) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let mut sql = String::from("f.status = 0");
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let arg = |params: &mut Vec<Box<dyn rusqlite::ToSql>>, v: Box<dyn rusqlite::ToSql>| {
        params.push(v);
        params.len()
    };

    if let Some(parent) = &q.parent {
        if q.recursive {
            if !parent.is_empty() {
                let i = arg(&mut params, Box::new(parent.clone()));
                // zakres zamiast LIKE — korzysta z indeksu ('0' = '/' + 1 w ASCII)
                sql += &format!(
                    " AND (f.parent = ?{i} OR (f.parent >= ?{i} || '/' AND f.parent < ?{i} || '0'))"
                );
            }
        } else {
            let i = arg(&mut params, Box::new(parent.clone()));
            sql += &format!(" AND f.parent = ?{i}");
        }
    }
    if let Some(kind) = q.kind {
        let i = arg(&mut params, Box::new(kind));
        sql += &format!(" AND f.kind = ?{i}");
    }
    if q.rating_min > 0 {
        let i = arg(&mut params, Box::new(q.rating_min));
        sql += &format!(" AND f.rating >= ?{i}");
    }
    if let Some(tag) = q.tag_id {
        let i = arg(&mut params, Box::new(tag));
        sql += &format!(" AND EXISTS (SELECT 1 FROM file_tags ft WHERE ft.file_id = f.id AND ft.tag_id = ?{i})");
    }
    if let Some(album) = q.album_id {
        let i = arg(&mut params, Box::new(album));
        sql += &format!(" AND EXISTS (SELECT 1 FROM album_files af WHERE af.file_id = f.id AND af.album_id = ?{i})");
    }
    if let Some(from) = q.date_from {
        let i = arg(&mut params, Box::new(from));
        sql += &format!(" AND f.taken_at >= ?{i}");
    }
    if let Some(to) = q.date_to {
        let i = arg(&mut params, Box::new(to));
        sql += &format!(" AND f.taken_at < ?{i}");
    }
    if let Some(query) = &q.query {
        let fts = fts_query(query);
        if !fts.is_empty() {
            let i = arg(&mut params, Box::new(fts));
            sql += &format!(
                " AND f.id IN (SELECT rowid FROM files_fts WHERE files_fts MATCH ?{i})"
            );
        }
    }
    (sql, params)
}

pub fn order_clause(sort: &str) -> &'static str {
    match sort {
        "date_asc" => "f.taken_at IS NULL, f.taken_at ASC, f.name COLLATE NOCASE",
        "name" => "f.name COLLATE NOCASE",
        "size_desc" => "f.size DESC",
        _ => "f.taken_at IS NULL, f.taken_at DESC, f.name COLLATE NOCASE",
    }
}

// ── oceny ───────────────────────────────────────────────────────────────

pub fn set_rating(conn: &Connection, id: i64, rating: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE files SET rating = ?2 WHERE id = ?1",
        rusqlite::params![id, rating.clamp(0, 5)],
    )?;
    Ok(())
}

// ── tagi ────────────────────────────────────────────────────────────────

fn sync_fts_tags(conn: &Connection, file_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE files_fts SET tags = COALESCE((
            SELECT group_concat(t.name, ' ') FROM file_tags ft
            JOIN tags t ON t.id = ft.tag_id WHERE ft.file_id = ?1
         ), '') WHERE rowid = ?1",
        [file_id],
    )?;
    Ok(())
}

pub fn tag_file(conn: &Connection, file_id: i64, name: &str) -> rusqlite::Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(());
    }
    conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [name])?;
    let tag_id: i64 = conn.query_row("SELECT id FROM tags WHERE name = ?1", [name], |r| r.get(0))?;
    conn.execute(
        "INSERT OR IGNORE INTO file_tags (file_id, tag_id) VALUES (?1, ?2)",
        [file_id, tag_id],
    )?;
    sync_fts_tags(conn, file_id)
}

pub fn untag_file(conn: &Connection, file_id: i64, tag_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM file_tags WHERE file_id = ?1 AND tag_id = ?2",
        [file_id, tag_id],
    )?;
    // osierocone tagi znikają z listy
    conn.execute(
        "DELETE FROM tags WHERE id = ?1
         AND NOT EXISTS (SELECT 1 FROM file_tags WHERE tag_id = ?1)",
        [tag_id],
    )?;
    sync_fts_tags(conn, file_id)
}

pub fn list_tags(conn: &Connection) -> Vec<(i64, String, i64)> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT t.id, t.name, count(ft.file_id) FROM tags t
             LEFT JOIN file_tags ft ON ft.tag_id = t.id
             GROUP BY t.id ORDER BY t.name COLLATE NOCASE",
        )
        .expect("stmt");
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

pub fn file_tags(conn: &Connection, file_id: i64) -> Vec<(i64, String)> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT t.id, t.name FROM file_tags ft JOIN tags t ON t.id = ft.tag_id
             WHERE ft.file_id = ?1 ORDER BY t.name COLLATE NOCASE",
        )
        .expect("stmt");
    stmt.query_map([file_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

// ── albumy ──────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct Album {
    pub id: i64,
    pub name: String,
    pub r#type: String,
    pub folder_path: Option<String>,
    pub count: i64,
    pub cover: Option<String>, // hash miniaturki najnowszego pliku
}

pub fn list_albums(conn: &Connection) -> Vec<Album> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT a.id, a.name, a.type, a.folder_path,
                (SELECT count(*) FROM album_files af JOIN files f ON f.id = af.file_id
                 WHERE af.album_id = a.id AND f.status = 0),
                (SELECT CASE WHEN f.thumb = 1 THEN lower(hex(f.hash)) END
                 FROM album_files af JOIN files f ON f.id = af.file_id
                 WHERE af.album_id = a.id AND f.status = 0
                 ORDER BY af.added_at DESC LIMIT 1)
             FROM albums a ORDER BY a.name COLLATE NOCASE",
        )
        .expect("stmt");
    stmt.query_map([], |r| {
        Ok(Album {
            id: r.get(0)?,
            name: r.get(1)?,
            r#type: r.get(2)?,
            folder_path: r.get(3)?,
            count: r.get(4)?,
            cover: r.get(5)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// Zamienia znaki niedozwolone w nazwie katalogu na „_".
pub fn sanitize_component(name: &str) -> String {
    name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
}

pub fn create_album(
    conn: &Connection,
    root: &Path,
    name: &str,
    album_type: &str,
) -> Result<i64, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Podaj nazwę albumu".into());
    }
    let folder_path = if album_type == "folder" {
        // fizyczny folder Albumy/<nazwa> w bibliotece
        let rel = format!("Albumy/{}", sanitize_component(name));
        std::fs::create_dir_all(root.join(&rel)).map_err(|e| e.to_string())?;
        Some(rel)
    } else {
        None
    };
    conn.execute(
        "INSERT INTO albums (name, type, folder_path, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![name, album_type, folder_path, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

pub fn delete_album(conn: &Connection, id: i64) -> Result<(), String> {
    // folderowy: pliki zostają na dysku, znika tylko album
    conn.execute("DELETE FROM albums WHERE id = ?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Fizycznie przenosi pliki do folderu (ścieżka względna do `root`; `""` = korzeń
/// biblioteki). Zwraca id plików faktycznie znajdujących się w folderze (przeniesione
/// lub już tam będące). Operacja jest logowana (undo przez move); pliki, których nie
/// udało się przenieść (brak wpisu / błąd rename), są pomijane.
pub fn move_files_to_folder(
    conn: &Connection,
    root: &Path,
    folder: &str,
    file_ids: &[i64],
    op_label: &str,
) -> Result<Vec<i64>, String> {
    if !folder.is_empty() {
        std::fs::create_dir_all(root.join(folder)).map_err(|e| e.to_string())?;
    }
    conn.execute(
        "INSERT INTO operations (kind, label, created_at) VALUES ('move', ?1, ?2)",
        rusqlite::params![op_label, now()],
    )
    .map_err(|e| e.to_string())?;
    let op_id = conn.last_insert_rowid();
    let mut moved = Vec::new();
    for id in file_ids {
        let Ok(rel) = conn.query_row("SELECT path FROM files WHERE id = ?1", [id], |r| {
            r.get::<_, String>(0)
        }) else {
            continue;
        };
        let already = rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("") == folder;
        if already {
            moved.push(*id);
            continue; // już w docelowym folderze
        }
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        let (dst_abs, dst_rel) = crate::import::unique_dst(root, folder, name);
        if std::fs::rename(root.join(&rel), &dst_abs).is_err() {
            continue;
        }
        let parent = dst_rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let fname = dst_rel.rsplit('/').next().unwrap_or(&dst_rel);
        conn.execute(
            "UPDATE files SET path = ?2, parent = ?3, name = ?4 WHERE id = ?1",
            rusqlite::params![id, dst_rel, parent, fname],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO operation_items (op_id, src, dst) VALUES (?1, ?2, ?3)",
            rusqlite::params![op_id, rel, dst_rel],
        )
        .map_err(|e| e.to_string())?;
        moved.push(*id);
    }
    Ok(moved)
}

/// Dodaje pliki do albumu. Wirtualny: tylko referencje. Folderowy: fizyczne
/// przeniesienie do folderu albumu, zapisane w logu operacji (undo przez move).
pub fn add_to_album(
    conn: &Connection,
    root: &Path,
    album_id: i64,
    file_ids: &[i64],
) -> Result<u64, String> {
    let (album_type, folder_path): (String, Option<String>) = conn
        .query_row(
            "SELECT type, folder_path FROM albums WHERE id = ?1",
            [album_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;

    let mut added = 0u64;
    if album_type == "folder" {
        let folder = folder_path.ok_or("album folderowy bez ścieżki")?;
        let moved = move_files_to_folder(
            conn,
            root,
            &folder,
            file_ids,
            &format!("do albumu: {folder}"),
        )?;
        for id in moved {
            added += move_noop_link(conn, album_id, id)?;
        }
    } else {
        for id in file_ids {
            added += move_noop_link(conn, album_id, *id)?;
        }
    }
    Ok(added)
}

fn move_noop_link(conn: &Connection, album_id: i64, file_id: i64) -> Result<u64, String> {
    let n = conn
        .execute(
            "INSERT OR IGNORE INTO album_files (album_id, file_id, added_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![album_id, file_id, now()],
        )
        .map_err(|e| e.to_string())?;
    Ok(n as u64)
}

pub fn remove_from_album(conn: &Connection, album_id: i64, file_ids: &[i64]) -> Result<(), String> {
    for id in file_ids {
        conn.execute(
            "DELETE FROM album_files WHERE album_id = ?1 AND file_id = ?2",
            [album_id, *id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── oś czasu ────────────────────────────────────────────────────────────

/// Miesiące z liczbą plików (na podstawie taken_at), najnowsze najpierw.
pub fn timeline_months(conn: &Connection) -> Vec<(String, i64)> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT strftime('%Y-%m', taken_at, 'unixepoch') ym, count(*)
             FROM files WHERE status = 0 AND taken_at IS NOT NULL
             GROUP BY ym ORDER BY ym DESC",
        )
        .expect("stmt");
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// Histogram miesięcy dla aktualnego zapytania (te same filtry co lista),
/// uporządkowany zgodnie z kierunkiem sortowania — zasila scrubber daty.
/// Kolejność miesięcy odpowiada kolejności plików w siatce, więc suma
/// narastająca daje mapowanie: indeks pliku ↔ data.
pub fn timeline_for_query(conn: &Connection, q: &ListQuery) -> Vec<(String, i64)> {
    let (where_sql, params) = build_where(q);
    // scrubber ma sens tylko dla sortowania po dacie; dla date_asc rosnąco
    let dir = if q.sort == "date_asc" { "ASC" } else { "DESC" };
    let sql = format!(
        "SELECT strftime('%Y-%m', f.taken_at, 'unixepoch') ym, count(*)
         FROM files f WHERE {where_sql} AND f.taken_at IS NOT NULL
         GROUP BY ym ORDER BY ym {dir}"
    );
    let Ok(mut stmt) = conn.prepare_cached(&sql) else {
        return Vec::new();
    };
    stmt.query_map(
        rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

// ── statystyki i zdrowie biblioteki ─────────────────────────────────────

#[derive(serde::Serialize)]
pub struct LibraryStats {
    pub files: i64,
    pub photos: i64,
    pub videos: i64,
    pub total_size: i64,
    pub missing: i64,
    pub protected: i64,
    pub by_year: Vec<(String, i64, i64)>, // rok, pliki, bajty
}

pub fn library_stats(conn: &Connection) -> LibraryStats {
    let (files, photos, videos, total_size, protected): (i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT count(*), sum(kind=0), sum(kind=1), COALESCE(sum(size),0),
                    count(protected_album) FROM files WHERE status = 0",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap_or((0, 0, 0, 0, 0));
    let missing: i64 = conn
        .query_row("SELECT count(*) FROM files WHERE status = 1", [], |r| r.get(0))
        .unwrap_or(0);
    let by_year = {
        let mut stmt = conn
            .prepare_cached(
                "SELECT COALESCE(strftime('%Y', taken_at, 'unixepoch'), '?'), count(*), sum(size)
                 FROM files WHERE status = 0 GROUP BY 1 ORDER BY 1 DESC",
            )
            .expect("stmt");
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    };
    LibraryStats {
        files,
        photos,
        videos,
        total_size,
        missing,
        protected,
        by_year,
    }
}

/// Rekoncyliacja brakujących plików: jeśli treść (hash) istnieje pod inną
/// ścieżką, przenosi metadane (ocena, tagi, albumy) i usuwa martwy wpis.
/// Wpisy bez pary są usuwane (plik realnie zniknął z dysku).
pub fn reconcile_missing(conn: &Connection) -> Result<(u64, u64), String> {
    let missing: Vec<(i64, Option<Vec<u8>>, i64)> = {
        let mut stmt = conn
            .prepare_cached("SELECT id, hash, rating FROM files WHERE status = 1")
            .map_err(|e| e.to_string())?;
        let rows: Vec<(i64, Option<Vec<u8>>, i64)> =
            match stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))) {
                Ok(mapped) => mapped.filter_map(Result::ok).collect(),
                Err(e) => return Err(e.to_string()),
            };
        rows
    };
    let (mut merged, mut removed) = (0u64, 0u64);
    for (old_id, hash, rating) in missing {
        let target: Option<i64> = hash.as_ref().and_then(|h| {
            conn.query_row(
                "SELECT id FROM files WHERE hash = ?1 AND status = 0 LIMIT 1",
                rusqlite::params![h],
                |r| r.get(0),
            )
            .ok()
        });
        if let Some(new_id) = target {
            conn.execute(
                "UPDATE OR IGNORE file_tags SET file_id = ?2 WHERE file_id = ?1",
                [old_id, new_id],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "UPDATE OR IGNORE album_files SET file_id = ?2 WHERE file_id = ?1",
                [old_id, new_id],
            )
            .map_err(|e| e.to_string())?;
            if rating > 0 {
                conn.execute(
                    "UPDATE files SET rating = max(rating, ?2) WHERE id = ?1",
                    [new_id, rating],
                )
                .map_err(|e| e.to_string())?;
            }
            merged += 1;
        } else {
            removed += 1;
        }
        conn.execute("DELETE FROM files WHERE id = ?1", [old_id])
            .map_err(|e| e.to_string())?;
    }
    Ok((merged, removed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn setup(name: &str) -> (Connection, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("medianest-test-catalog-{name}"));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("lib");
        std::fs::create_dir_all(&root).unwrap();
        (db::open(&base.join("data")).unwrap(), root)
    }

    fn add(conn: &Connection, path: &str, name: &str) -> i64 {
        let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        conn.execute(
            "INSERT INTO files (path,parent,name,kind,size,mtime) VALUES (?1,?2,?3,0,1,1)",
            rusqlite::params![path, parent, name],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn fts_search_finds_by_name_and_tag() {
        let (conn, _) = setup("fts");
        let id1 = add(&conn, "2020/wakacje_gdansk.jpg", "wakacje_gdansk.jpg");
        add(&conn, "2020/praca.jpg", "praca.jpg");
        tag_file(&conn, id1, "morze").unwrap();

        let count = |q: &str| -> i64 {
            let lq = ListQuery {
                query: Some(q.into()),
                ..Default::default()
            };
            let (where_sql, params) = build_where(&lq);
            conn.query_row(
                &format!("SELECT count(*) FROM files f WHERE {where_sql}"),
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(count("gdansk"), 1);
        assert_eq!(count("wakac"), 1); // prefiks
        assert_eq!(count("morze"), 1); // tag
        assert_eq!(count("gory"), 0);

        // po zdjęciu taga znika z wyszukiwania
        let tag_id: i64 = conn.query_row("SELECT id FROM tags WHERE name='morze'", [], |r| r.get(0)).unwrap();
        untag_file(&conn, id1, tag_id).unwrap();
        assert_eq!(count("morze"), 0);
        assert!(list_tags(&conn).is_empty()); // osierocony tag usunięty
    }

    #[test]
    fn filters_compose() {
        let (conn, _) = setup("filters");
        let id1 = add(&conn, "a/x.jpg", "x.jpg");
        let id2 = add(&conn, "a/y.jpg", "y.jpg");
        add(&conn, "b/z.jpg", "z.jpg");
        set_rating(&conn, id1, 5).unwrap();
        set_rating(&conn, id2, 2).unwrap();

        let q = ListQuery {
            parent: Some("a".into()),
            recursive: true,
            rating_min: 3,
            ..Default::default()
        };
        let (where_sql, params) = build_where(&q);
        let count: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM files f WHERE {where_sql}"),
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn virtual_and_folder_albums() {
        let (conn, root) = setup("albums");
        std::fs::create_dir_all(root.join("2020")).unwrap();
        std::fs::write(root.join("2020/a.jpg"), b"a").unwrap();
        let id = add(&conn, "2020/a.jpg", "a.jpg");

        // wirtualny: referencja, plik zostaje na miejscu
        let virt = create_album(&conn, &root, "Ulubione", "virtual").unwrap();
        add_to_album(&conn, &root, virt, &[id]).unwrap();
        assert!(root.join("2020/a.jpg").exists());
        assert_eq!(list_albums(&conn)[0].count, 1);

        // folderowy: fizyczne przeniesienie
        let fold = create_album(&conn, &root, "Rodzina", "folder").unwrap();
        add_to_album(&conn, &root, fold, &[id]).unwrap();
        assert!(!root.join("2020/a.jpg").exists());
        assert!(root.join("Albumy/Rodzina/a.jpg").exists());
        let path: String = conn
            .query_row("SELECT path FROM files WHERE id=?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(path, "Albumy/Rodzina/a.jpg");
        // operacja move w logu z możliwością cofnięcia
        let op: String = conn
            .query_row("SELECT kind FROM operations ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(op, "move");

        // usunięcie albumu nie usuwa plików
        delete_album(&conn, fold).unwrap();
        assert!(root.join("Albumy/Rodzina/a.jpg").exists());
    }

    #[test]
    fn move_to_folder_moves_and_logs() {
        let (conn, root) = setup("movefolder");
        std::fs::create_dir_all(root.join("2020")).unwrap();
        std::fs::write(root.join("2020/a.jpg"), b"a").unwrap();
        let id = add(&conn, "2020/a.jpg", "a.jpg");

        let moved =
            move_files_to_folder(&conn, &root, "Foto/2021", &[id], "do folderu: Foto/2021").unwrap();
        assert_eq!(moved, vec![id]);
        assert!(!root.join("2020/a.jpg").exists());
        assert!(root.join("Foto/2021/a.jpg").exists());
        let (path, parent): (String, String) = conn
            .query_row("SELECT path, parent FROM files WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(path, "Foto/2021/a.jpg");
        assert_eq!(parent, "Foto/2021");
        // wpis move w logu (możliwość cofnięcia)
        let op: String = conn
            .query_row("SELECT kind FROM operations ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(op, "move");

        // ponowne przeniesienie do tego samego folderu = no-op (już tam jest)
        let again = move_files_to_folder(&conn, &root, "Foto/2021", &[id], "x").unwrap();
        assert_eq!(again, vec![id]);
        assert!(root.join("Foto/2021/a.jpg").exists());
    }

    #[test]
    fn reconcile_moves_metadata_to_surviving_copy() {
        let (conn, _) = setup("reconcile");
        // stary wpis (brakujący) i nowa lokalizacja tej samej treści
        conn.execute(
            "INSERT INTO files (path,parent,name,kind,size,mtime,hash,rating,status)
             VALUES ('old/a.jpg','old','a.jpg',0,1,1,x'aa',4,1)",
            [],
        )
        .unwrap();
        let old_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO files (path,parent,name,kind,size,mtime,hash,status)
             VALUES ('new/a.jpg','new','a.jpg',0,1,1,x'aa',0)",
            [],
        )
        .unwrap();
        let new_id = conn.last_insert_rowid();
        tag_file(&conn, old_id, "wakacje").unwrap();
        // wpis bez pary — plik przepadł
        conn.execute(
            "INSERT INTO files (path,parent,name,kind,size,mtime,hash,status)
             VALUES ('gone.jpg','','gone.jpg',0,1,1,x'bb',1)",
            [],
        )
        .unwrap();

        let (merged, removed) = reconcile_missing(&conn).unwrap();
        assert_eq!((merged, removed), (1, 1));
        let rating: i64 = conn
            .query_row("SELECT rating FROM files WHERE id=?1", [new_id], |r| r.get(0))
            .unwrap();
        assert_eq!(rating, 4);
        assert_eq!(file_tags(&conn, new_id).len(), 1);
        let missing: i64 = conn
            .query_row("SELECT count(*) FROM files WHERE status=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(missing, 0);
    }

    #[test]
    fn timeline_histogram_respects_filter_and_sort() {
        let (conn, _) = setup("histogram");
        let a = add(&conn, "a.jpg", "a.jpg");
        let b = add(&conn, "b.jpg", "b.jpg");
        let c = add(&conn, "c.jpg", "c.jpg");
        let d = add(&conn, "d.jpg", "d.jpg"); // bez daty — poza histogramem
        // 2023-07 ×2, 2024-01 ×1
        conn.execute("UPDATE files SET taken_at=1689422400 WHERE id IN (?1,?2)", [a, b]).unwrap();
        conn.execute("UPDATE files SET taken_at=1704067200 WHERE id=?1", [c]).unwrap();
        set_rating(&conn, a, 5).unwrap();

        // domyślnie (date_desc): najnowsze najpierw, d bez daty pominięte
        let h = timeline_for_query(&conn, &ListQuery::default());
        assert_eq!(h, vec![("2024-01".into(), 1), ("2023-07".into(), 2)]);
        let dated: i64 = h.iter().map(|(_, n)| n).sum();
        assert_eq!(dated, 3, "d.jpg bez daty nie wchodzi do histogramu");
        let _ = d;

        // date_asc odwraca kolejność
        let asc = timeline_for_query(
            &conn,
            &ListQuery { sort: "date_asc".into(), ..Default::default() },
        );
        assert_eq!(asc, vec![("2023-07".into(), 2), ("2024-01".into(), 1)]);

        // filtr oceny zawęża histogram do pasujących plików
        let filtered = timeline_for_query(
            &conn,
            &ListQuery { rating_min: 5, ..Default::default() },
        );
        assert_eq!(filtered, vec![("2023-07".into(), 1)]);
    }

    #[test]
    fn timeline_groups_by_month() {
        let (conn, _) = setup("timeline");
        let id1 = add(&conn, "a.jpg", "a.jpg");
        let id2 = add(&conn, "b.jpg", "b.jpg");
        let id3 = add(&conn, "c.jpg", "c.jpg");
        // 2023-07 ×2, 2024-01 ×1
        conn.execute("UPDATE files SET taken_at=1689422400 WHERE id IN (?1,?2)", [id1, id2]).unwrap();
        conn.execute("UPDATE files SET taken_at=1704067200 WHERE id=?1", [id3]).unwrap();
        let months = timeline_months(&conn);
        assert_eq!(months, vec![("2024-01".into(), 1), ("2023-07".into(), 2)]);
    }
}
