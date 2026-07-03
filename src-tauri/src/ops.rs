//! Kosz aplikacji i cofanie operacji (undo).

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::Connection;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

pub fn trash_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("trash")
}

/// Przenosi plik między lokalizacjami. Zwykły rename na Windows zawodzi
/// przy przenoszeniu MIĘDZY wolumenami (biblioteka na D:\, kosz na C:\ w
/// %APPDATA%) — wtedy fallback na kopię + usunięcie oryginału.
fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(src, dst)?;
            std::fs::remove_file(src)?;
            Ok(())
        }
    }
}

/// Przenosi pliki biblioteki do kosza aplikacji (odwracalne).
pub fn trash_files(
    conn: &Connection,
    root: &Path,
    data_dir: &Path,
    ids: &[i64],
    op_kind: &str,
) -> Result<u64, String> {
    let tdir = trash_dir(data_dir);
    std::fs::create_dir_all(&tdir).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO operations (kind, label, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![op_kind, format!("{} plików", ids.len()), now()],
    )
    .map_err(|e| e.to_string())?;
    let op_id = conn.last_insert_rowid();

    let mut moved = 0u64;
    for id in ids {
        let row: Option<(String, String, i64, i64, Option<Vec<u8>>, Option<String>, i64)> = conn
            .query_row(
                "SELECT path, name, kind, size, hash, blurhash, thumb FROM files WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .ok();
        let Some((rel, name, kind, size, hash, blurhash, thumb)) = row else {
            continue;
        };
        let trash_name = format!("{id}_{name}");
        if let Err(e) = move_file(&root.join(&rel), &tdir.join(&trash_name)) {
            // plik zniknął / zablokowany przez inną aplikację — pomijamy
            eprintln!("kosz: nie przeniesiono {rel}: {e}");
            continue;
        }
        conn.execute(
            "INSERT INTO trash (orig_path, trash_name, kind, size, hash, blurhash, thumb, deleted_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![rel, trash_name, kind, size, hash, blurhash, thumb, now()],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM files WHERE id = ?1", [id])
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO operation_items (op_id, src, dst) VALUES (?1, ?2, ?3)",
            rusqlite::params![op_id, rel, trash_name],
        )
        .map_err(|e| e.to_string())?;
        moved += 1;
    }
    Ok(moved)
}

#[derive(serde::Serialize)]
pub struct TrashItem {
    pub id: i64,
    pub orig_path: String,
    pub kind: i64,
    pub size: i64,
    pub thumb: Option<String>,
    pub blurhash: Option<String>,
    pub deleted_at: i64,
}

pub fn list_trash(conn: &Connection) -> Vec<TrashItem> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT id, orig_path, kind, size,
                    CASE WHEN thumb = 1 THEN lower(hex(hash)) END, blurhash, deleted_at
             FROM trash ORDER BY deleted_at DESC",
        )
        .expect("stmt");
    stmt.query_map([], |r| {
        Ok(TrashItem {
            id: r.get(0)?,
            orig_path: r.get(1)?,
            kind: r.get(2)?,
            size: r.get(3)?,
            thumb: r.get(4)?,
            blurhash: r.get(5)?,
            deleted_at: r.get(6)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// Przywraca pliki z kosza na oryginalne miejsce (kolizja → sufiks).
pub fn restore_trash(
    conn: &Connection,
    root: &Path,
    data_dir: &Path,
    ids: &[i64],
) -> Result<u64, String> {
    let tdir = trash_dir(data_dir);
    let mut restored = 0u64;
    for id in ids {
        let row: Option<(String, String, i64)> = conn
            .query_row(
                "SELECT orig_path, trash_name, kind FROM trash WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        let Some((orig, trash_name, kind)) = row else {
            continue;
        };
        let (dir, name) = orig.rsplit_once('/').unwrap_or(("", orig.as_str()));
        let (dst_abs, dst_rel) = crate::import::unique_dst(root, dir, name);
        if let Some(p) = dst_abs.parent() {
            std::fs::create_dir_all(p).ok();
        }
        if let Err(e) = move_file(&tdir.join(&trash_name), &dst_abs) {
            eprintln!("przywracanie: nie przeniesiono {trash_name}: {e}");
            continue;
        }
        let meta = std::fs::metadata(&dst_abs).map_err(|e| e.to_string())?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let parent = dst_rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let fname = dst_rel.rsplit('/').next().unwrap_or(&dst_rel);
        // hash NULL → indekser przeliczy metadane; miniaturka i tak jest w cache
        conn.execute(
            "INSERT INTO files (path, parent, name, kind, size, mtime)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(path) DO UPDATE SET status=0, hash=NULL, thumb=0",
            rusqlite::params![dst_rel, parent, fname, kind, meta.len() as i64, mtime],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM trash WHERE id = ?1", [id])
            .map_err(|e| e.to_string())?;
        restored += 1;
    }
    Ok(restored)
}

/// Trwale usuwa zawartość kosza (wszystko albo wskazane pozycje).
pub fn empty_trash(conn: &Connection, data_dir: &Path, ids: Option<&[i64]>) -> Result<u64, String> {
    let tdir = trash_dir(data_dir);
    let items: Vec<(i64, String)> = {
        let (sql, use_ids) = match ids {
            Some(_) => ("SELECT id, trash_name FROM trash WHERE id = ?1", true),
            None => ("SELECT id, trash_name FROM trash", false),
        };
        if use_ids {
            let mut out = Vec::new();
            let mut stmt = conn.prepare_cached(sql).map_err(|e| e.to_string())?;
            for id in ids.unwrap() {
                if let Ok(row) = stmt.query_row([id], |r| Ok((r.get(0)?, r.get(1)?))) {
                    out.push(row);
                }
            }
            out
        } else {
            let mut stmt = conn.prepare_cached(sql).map_err(|e| e.to_string())?;
            let rows: Vec<(i64, String)> = match stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
                Ok(mapped) => mapped.filter_map(Result::ok).collect(),
                Err(e) => return Err(e.to_string()),
            };
            rows
        }
    };
    let mut removed = 0u64;
    for (id, trash_name) in items {
        std::fs::remove_file(tdir.join(&trash_name)).ok();
        conn.execute("DELETE FROM trash WHERE id = ?1", [id])
            .map_err(|e| e.to_string())?;
        removed += 1;
    }
    Ok(removed)
}

#[derive(serde::Serialize)]
pub struct Operation {
    pub id: i64,
    pub kind: String,
    pub label: String,
    pub created_at: i64,
    pub undone_at: Option<i64>,
    pub items: i64,
}

pub fn list_operations(conn: &Connection) -> Vec<Operation> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT o.id, o.kind, o.label, o.created_at, o.undone_at,
                    (SELECT count(*) FROM operation_items i WHERE i.op_id = o.id)
             FROM operations o ORDER BY o.id DESC LIMIT 50",
        )
        .expect("stmt");
    stmt.query_map([], |r| {
        Ok(Operation {
            id: r.get(0)?,
            kind: r.get(1)?,
            label: r.get(2)?,
            created_at: r.get(3)?,
            undone_at: r.get(4)?,
            items: r.get(5)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// Cofa operację. Import → zaimportowane pliki lądują w koszu aplikacji.
/// Kosz → pliki wracają na miejsce.
pub fn undo_operation(
    conn: &Connection,
    root: &Path,
    data_dir: &Path,
    op_id: i64,
) -> Result<String, String> {
    let (kind, undone): (String, Option<i64>) = conn
        .query_row(
            "SELECT kind, undone_at FROM operations WHERE id = ?1",
            [op_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if undone.is_some() {
        return Err("Operacja została już cofnięta".into());
    }

    let items: Vec<(String, Option<String>)> = {
        let mut stmt = conn
            .prepare_cached("SELECT src, dst FROM operation_items WHERE op_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows: Vec<(String, Option<String>)> =
            match stmt.query_map([op_id], |r| Ok((r.get(0)?, r.get(1)?))) {
                Ok(mapped) => mapped.filter_map(Result::ok).collect(),
                Err(e) => return Err(e.to_string()),
            };
        rows
    };

    let message = match kind.as_str() {
        "import" => {
            // zaimportowane pliki → kosz aplikacji
            let ids: Vec<i64> = items
                .iter()
                .filter_map(|(_, dst)| dst.as_ref())
                .filter_map(|dst| {
                    conn.query_row(
                        "SELECT id FROM files WHERE path = ?1",
                        [dst],
                        |r| r.get(0),
                    )
                    .ok()
                })
                .collect();
            let moved = trash_files(conn, root, data_dir, &ids, "undo-import")?;
            format!("Cofnięto import: {moved} plików przeniesiono do kosza")
        }
        "trash" | "undo-import" => {
            // przywróć z kosza po trash_name (dst)
            let ids: Vec<i64> = items
                .iter()
                .filter_map(|(_, dst)| dst.as_ref())
                .filter_map(|dst| {
                    conn.query_row(
                        "SELECT id FROM trash WHERE trash_name = ?1",
                        [dst],
                        |r| r.get(0),
                    )
                    .ok()
                })
                .collect();
            let restored = restore_trash(conn, root, data_dir, &ids)?;
            format!("Przywrócono {restored} plików z kosza")
        }
        other => return Err(format!("Operacji '{other}' nie można cofnąć")),
    };
    conn.execute(
        "UPDATE operations SET undone_at = ?2 WHERE id = ?1",
        rusqlite::params![op_id, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn setup(name: &str) -> (Connection, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("medianest-test-ops-{name}"));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("lib");
        let data = base.join("data");
        std::fs::create_dir_all(&root).unwrap();
        let conn = db::open(&data).unwrap();
        (conn, root, data)
    }

    fn add_file(conn: &Connection, root: &Path, rel: &str) -> i64 {
        std::fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
        std::fs::write(root.join(rel), rel.as_bytes()).unwrap();
        let parent = rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let name = rel.rsplit('/').next().unwrap();
        conn.execute(
            "INSERT INTO files (path,parent,name,kind,size,mtime,hash,thumb)
             VALUES (?1,?2,?3,0,1,1,x'ab',1)",
            rusqlite::params![rel, parent, name],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn trash_restore_roundtrip() {
        let (conn, root, data) = setup("roundtrip");
        let id = add_file(&conn, &root, "2023/a.jpg");

        let moved = trash_files(&conn, &root, &data, &[id], "trash").unwrap();
        assert_eq!(moved, 1);
        assert!(!root.join("2023/a.jpg").exists());
        assert_eq!(list_trash(&conn).len(), 1);
        let fcount: i64 = conn
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fcount, 0);

        let tid = list_trash(&conn)[0].id;
        let restored = restore_trash(&conn, &root, &data, &[tid]).unwrap();
        assert_eq!(restored, 1);
        assert!(root.join("2023/a.jpg").exists());
        assert!(list_trash(&conn).is_empty());
        let fcount: i64 = conn
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fcount, 1);
    }

    #[test]
    fn undo_trash_restores_files() {
        let (conn, root, data) = setup("undo");
        let id = add_file(&conn, &root, "x/b.jpg");
        trash_files(&conn, &root, &data, &[id], "trash").unwrap();
        let op_id: i64 = conn
            .query_row("SELECT max(id) FROM operations", [], |r| r.get(0))
            .unwrap();
        let msg = undo_operation(&conn, &root, &data, op_id).unwrap();
        assert!(msg.contains("Przywrócono 1"));
        assert!(root.join("x/b.jpg").exists());
        // drugi raz się nie da
        assert!(undo_operation(&conn, &root, &data, op_id).is_err());
    }

    #[test]
    fn empty_trash_is_permanent() {
        let (conn, root, data) = setup("empty");
        let id = add_file(&conn, &root, "c.jpg");
        trash_files(&conn, &root, &data, &[id], "trash").unwrap();
        let removed = empty_trash(&conn, &data, None).unwrap();
        assert_eq!(removed, 1);
        assert!(list_trash(&conn).is_empty());
        assert!(std::fs::read_dir(trash_dir(&data)).unwrap().next().is_none());
    }
}
