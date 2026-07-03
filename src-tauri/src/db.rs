use rusqlite::Connection;
use std::path::Path;

// Migracje wersjonowane przez PRAGMA user_version — indeks w tablicy = wersja docelowa - 1.
// Nigdy nie edytować istniejących wpisów, tylko dopisywać nowe.
const MIGRATIONS: &[&str] = &[
    // v1 — ustawienia aplikacji
    "CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );",
    // v2 — indeks plików. Tabela files pełni też rolę trwałej kolejki zadań:
    // wiersz z (hash IS NULL OR thumb=0) czeka na przetworzenie przez indekser.
    // thumb: 0=oczekuje, 1=gotowa, 2=brak/nieudana (nie ponawiać)
    // status: 0=ok, 1=brakujący na dysku, 2=w koszu
    "CREATE TABLE files (
        id       INTEGER PRIMARY KEY,
        path     TEXT NOT NULL UNIQUE,
        parent   TEXT NOT NULL,
        name     TEXT NOT NULL,
        kind     INTEGER NOT NULL,
        size     INTEGER NOT NULL,
        mtime    INTEGER NOT NULL,
        hash     BLOB,
        width    INTEGER,
        height   INTEGER,
        duration REAL,
        taken_at INTEGER,
        blurhash TEXT,
        thumb    INTEGER NOT NULL DEFAULT 0,
        status   INTEGER NOT NULL DEFAULT 0,
        error    TEXT
    );
    CREATE INDEX idx_files_parent ON files(parent);
    CREATE INDEX idx_files_taken ON files(taken_at);
    CREATE INDEX idx_files_hash ON files(hash);
    CREATE INDEX idx_files_pending ON files(status) WHERE hash IS NULL OR thumb = 0;",
    // v3 — ponowne kolejkowanie wideo: Faza 2 dodała miniaturki przez ffmpeg
    "UPDATE files SET thumb = 0 WHERE kind = 1;",
    // v4 — import: log operacji (undo), poczekalnia duplikatów, kosz
    "CREATE TABLE operations (
        id         INTEGER PRIMARY KEY,
        kind       TEXT NOT NULL,
        label      TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        undone_at  INTEGER
    );
    CREATE TABLE operation_items (
        id     INTEGER PRIMARY KEY,
        op_id  INTEGER NOT NULL REFERENCES operations(id) ON DELETE CASCADE,
        src    TEXT NOT NULL,
        dst    TEXT
    );
    CREATE INDEX idx_op_items_op ON operation_items(op_id);
    CREATE TABLE import_pending (
        id          INTEGER PRIMARY KEY,
        src         TEXT NOT NULL UNIQUE,
        dst         TEXT NOT NULL,
        hash        BLOB NOT NULL,
        dup_file_id INTEGER NOT NULL,
        size        INTEGER NOT NULL,
        created_at  INTEGER NOT NULL
    );
    CREATE TABLE trash (
        id         INTEGER PRIMARY KEY,
        orig_path  TEXT NOT NULL,
        trash_name TEXT NOT NULL,
        kind       INTEGER NOT NULL,
        size       INTEGER NOT NULL,
        hash       BLOB,
        blurhash   TEXT,
        thumb      INTEGER NOT NULL DEFAULT 0,
        deleted_at INTEGER NOT NULL
    );",
    // v5 — hashe percepcyjne do wykrywania zdjęć podobnych (Faza 4);
    // istniejące zdjęcia dostaną phash przy kolejnym przebiegu indeksera
    "ALTER TABLE files ADD COLUMN phash INTEGER;
     ALTER TABLE files ADD COLUMN dhash INTEGER;",
    // v6 — flaga prawdziwej daty EXIF: serie (burst) nie mogą opierać się na
    // mtime, bo po kopiowaniu tysiące plików dzielą tę samą datę modyfikacji.
    // Requeue zdjęć uzupełnia flagę przy kolejnym przebiegu indeksera.
    "ALTER TABLE files ADD COLUMN exif_date INTEGER NOT NULL DEFAULT 0;
     UPDATE files SET thumb = 0 WHERE kind = 0 AND thumb = 1;",
    // v7 — albumy, tagi, oceny, wyszukiwarka FTS5 (Faza 5)
    "ALTER TABLE files ADD COLUMN rating INTEGER NOT NULL DEFAULT 0;
    CREATE TABLE albums (
        id          INTEGER PRIMARY KEY,
        name        TEXT NOT NULL,
        type        TEXT NOT NULL DEFAULT 'virtual',
        folder_path TEXT,
        created_at  INTEGER NOT NULL
    );
    CREATE TABLE album_files (
        album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
        file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
        added_at INTEGER NOT NULL,
        PRIMARY KEY (album_id, file_id)
    );
    CREATE TABLE tags (
        id   INTEGER PRIMARY KEY,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE
    );
    CREATE TABLE file_tags (
        file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
        tag_id  INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
        PRIMARY KEY (file_id, tag_id)
    );
    CREATE VIRTUAL TABLE files_fts USING fts5(name, path, tags, tokenize='unicode61');
    CREATE TRIGGER files_fts_ai AFTER INSERT ON files BEGIN
        INSERT INTO files_fts(rowid, name, path, tags) VALUES (new.id, new.name, new.path, '');
    END;
    CREATE TRIGGER files_fts_ad AFTER DELETE ON files BEGIN
        DELETE FROM files_fts WHERE rowid = old.id;
    END;
    CREATE TRIGGER files_fts_au AFTER UPDATE OF name, path ON files BEGIN
        UPDATE files_fts SET name = new.name, path = new.path WHERE rowid = new.id;
    END;
    INSERT INTO files_fts(rowid, name, path, tags) SELECT id, name, path, '' FROM files;",
    // v8 — albumy chronione: sól + weryfikator klucza (Argon2id), oznaczenie
    // zaszyfrowanych plików (Faza 6)
    "ALTER TABLE albums ADD COLUMN key_salt BLOB;
     ALTER TABLE albums ADD COLUMN key_verifier BLOB;
     ALTER TABLE files ADD COLUMN protected_album INTEGER;",
    // v9 — poczekalnia obejmuje też pliki bez duplikatu: import skanuje i stage'uje
    // wszystko, nic nie kopiuje; użytkownik wybiera zaznaczeniem, co trafi do
    // biblioteki. dup_file_id może być NULL (nowy plik), kind przechowywany lokalnie
    // (brak wiersza files dla nowych). Tabela przejściowa → bezpieczny DROP+CREATE.
    "DROP TABLE import_pending;
    CREATE TABLE import_pending (
        id          INTEGER PRIMARY KEY,
        src         TEXT NOT NULL UNIQUE,
        dst         TEXT NOT NULL,
        hash        BLOB NOT NULL,
        dup_file_id INTEGER,
        kind        INTEGER NOT NULL DEFAULT 0,
        size        INTEGER NOT NULL,
        created_at  INTEGER NOT NULL
    );",
];

pub fn open(dir: &Path) -> anyhow::Result<Connection> {
    std::fs::create_dir_all(dir)?;
    let conn = Connection::open(dir.join("medianest.db"))?;
    // indekser i komendy UI pracują na osobnych połączeniach; bez tego równoległy
    // zapis (np. zmiana biblioteki w trakcie skanu) padał na SQLITE_BUSY
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "cache_size", -64000)?; // 64 MB cache stron
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        conn.execute_batch(sql)?;
        conn.pragma_update(None, "user_version", (i + 1) as i64)?;
    }
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_and_settings_roundtrip() {
        let dir = std::env::temp_dir().join("medianest-test-db");
        let _ = std::fs::remove_dir_all(&dir);
        let conn = open(&dir).unwrap();
        assert_eq!(get_setting(&conn, "library_path"), None);
        set_setting(&conn, "library_path", "D:\\Zdjecia").unwrap();
        set_setting(&conn, "library_path", "E:\\Foto").unwrap();
        assert_eq!(get_setting(&conn, "library_path").as_deref(), Some("E:\\Foto"));
        // ponowne otwarcie nie uruchamia migracji drugi raz
        drop(conn);
        let conn = open(&dir).unwrap();
        assert_eq!(get_setting(&conn, "library_path").as_deref(), Some("E:\\Foto"));
    }
}
