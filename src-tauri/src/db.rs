use crate::error::AppError;
use crate::ops::display_path;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct FileRecord {
    pub path: String,
    pub parent_path: String,
    pub name: String,
    pub ext: String,
    pub category: String,
    pub size: u64,
    pub mtime: Option<i64>,
    pub ctime: Option<i64>,
    pub is_hidden: bool,
    pub is_cloud: bool,
    pub is_directory: bool,
    pub drive: String,
    pub last_seen: i64,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IndexedEntry {
    pub id: i64,
    pub path: String,
    pub parent_path: String,
    pub name: String,
    pub ext: String,
    pub category: String,
    pub size: u64,
    pub mtime: Option<i64>,
    pub ctime: Option<i64>,
    pub is_hidden: bool,
    pub is_cloud: bool,
    pub is_directory: bool,
    pub drive: String,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CategorySummary {
    pub category: String,
    pub count: u64,
    pub total_size: u64,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IndexCounts {
    pub indexed_files: u64,
    pub indexed_directories: u64,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilter {
    pub query: String,
    pub category: Option<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub modified_after: Option<i64>,
    pub modified_before: Option<i64>,
    pub limit: Option<u32>,
}

pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(crate::ops::io_path(parent))?;
        }
        let connection = Connection::open(crate::ops::io_path(path))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "temp_store", "MEMORY")?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS files (
                 id INTEGER PRIMARY KEY,
                 path TEXT NOT NULL UNIQUE,
                 parent_path TEXT NOT NULL,
                 name TEXT NOT NULL,
                 ext TEXT NOT NULL,
                 category TEXT NOT NULL,
                 size INTEGER NOT NULL,
                 mtime INTEGER,
                 ctime INTEGER,
                 is_hidden INTEGER NOT NULL,
                 is_cloud INTEGER NOT NULL,
                 is_directory INTEGER NOT NULL DEFAULT 0,
                 drive TEXT NOT NULL,
                 last_seen INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_files_parent_name ON files(parent_path, name COLLATE NOCASE);
             CREATE INDEX IF NOT EXISTS idx_files_category_size ON files(category, size);
             CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime);
             CREATE INDEX IF NOT EXISTS idx_files_drive ON files(drive);
             CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(name, content='files', content_rowid='id', tokenize='unicode61');
             CREATE TRIGGER IF NOT EXISTS files_ai AFTER INSERT ON files BEGIN
                 INSERT INTO files_fts(rowid, name) VALUES (new.id, new.name);
             END;
             CREATE TRIGGER IF NOT EXISTS files_ad AFTER DELETE ON files BEGIN
                 INSERT INTO files_fts(files_fts, rowid, name) VALUES ('delete', old.id, old.name);
             END;
             CREATE TRIGGER IF NOT EXISTS files_au AFTER UPDATE OF name ON files BEGIN
                 INSERT INTO files_fts(files_fts, rowid, name) VALUES ('delete', old.id, old.name);
                 INSERT INTO files_fts(rowid, name) VALUES (new.id, new.name);
             END;",
        )?;
        Ok(Self { connection: Mutex::new(connection) })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, AppError> {
        self.connection.lock().map_err(|_| AppError::Unavailable)
    }

    pub fn insert_batch(&self, records: &[FileRecord]) -> Result<(), AppError> {
        if records.is_empty() { return Ok(()); }
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        insert_records(&transaction, records)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert(&self, record: &FileRecord) -> Result<(), AppError> {
        self.insert_batch(std::slice::from_ref(record))
    }

    pub fn complete_scan(&self, epoch: i64) -> Result<(), AppError> {
        self.lock()?.execute("DELETE FROM files WHERE last_seen <> ?1", [epoch])?;
        Ok(())
    }

    pub fn remove_path(&self, path: &Path) -> Result<(), AppError> {
        let path = display_path(path);
        let separator = if cfg!(windows) { '\\' } else { '/' };
        let child_prefix = if path.ends_with('\\') || path.ends_with('/') { path.clone() } else { format!("{path}{separator}") };
        self.lock()?.execute(
            "DELETE FROM files WHERE path = ?1 OR substr(path, 1, length(?2)) = ?2",
            params![path, child_prefix],
        )?;
        Ok(())
    }

    pub fn children(&self, path: &Path, sort: &str, descending: bool, limit: u32, offset: u32) -> Result<Vec<IndexedEntry>, AppError> {
        let order_column = match sort {
            "date" => "mtime",
            "size" => "size",
            "type" => "category",
            _ => "name COLLATE NOCASE",
        };
        let order_direction = if descending { "DESC" } else { "ASC" };
        let sql = format!(
            "SELECT id, path, parent_path, name, ext, category, size, mtime, ctime, is_hidden, is_cloud, is_directory, drive \
             FROM files WHERE parent_path = ?1 ORDER BY is_directory DESC, {order_column} {order_direction}, name COLLATE NOCASE ASC LIMIT ?2 OFFSET ?3"
        );
        let parent = display_path(path);
        let connection = self.lock()?;
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params![parent, limit, offset], map_entry)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn search(&self, filter: &SearchFilter) -> Result<Vec<IndexedEntry>, AppError> {
        let limit = filter.limit.unwrap_or(100).clamp(1, 250);
        let min_size = filter.min_size.map(|value| value.min(i64::MAX as u64) as i64);
        let max_size = filter.max_size.map(|value| value.min(i64::MAX as u64) as i64);
        let connection = self.lock()?;
        if filter.query.trim().is_empty() {
            if filter.category.is_none() && min_size.is_none() && max_size.is_none()
                && filter.modified_after.is_none() && filter.modified_before.is_none()
            {
                return Err(AppError::InvalidRequest);
            }
            let mut statement = connection.prepare(
                "SELECT id, path, parent_path, name, ext, category, size, mtime, ctime, is_hidden, is_cloud, is_directory, drive
                 FROM files WHERE is_directory=0
                   AND (?1 IS NULL OR category=?1)
                   AND (?2 IS NULL OR size>=?2)
                   AND (?3 IS NULL OR size<=?3)
                   AND (?4 IS NULL OR mtime>=?4)
                   AND (?5 IS NULL OR mtime<=?5)
                 ORDER BY mtime DESC LIMIT ?6",
            )?;
            let rows = statement.query_map(
                params![filter.category.as_deref(), min_size, max_size, filter.modified_after, filter.modified_before, limit],
                map_entry,
            )?;
            return rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from);
        }

        let query = fts_prefix_query(&filter.query)?;
        let mut statement = connection.prepare(
            "SELECT f.id, f.path, f.parent_path, f.name, f.ext, f.category, f.size, f.mtime, f.ctime, f.is_hidden, f.is_cloud, f.is_directory, f.drive
             FROM files_fts JOIN files AS f ON f.id = files_fts.rowid
             WHERE files_fts MATCH ?1
               AND f.is_directory = 0
               AND (?2 IS NULL OR f.category = ?2)
               AND (?3 IS NULL OR f.size >= ?3)
               AND (?4 IS NULL OR f.size <= ?4)
               AND (?5 IS NULL OR f.mtime >= ?5)
               AND (?6 IS NULL OR f.mtime <= ?6)
             LIMIT ?7",
        )?;
        let rows = statement.query_map(
            params![query, filter.category.as_deref(), min_size, max_size, filter.modified_after, filter.modified_before, limit],
            map_entry,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn categories(&self) -> Result<Vec<CategorySummary>, AppError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT category, COUNT(*), COALESCE(SUM(size), 0) FROM files WHERE is_directory = 0 GROUP BY category ORDER BY category",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(CategorySummary {
                category: row.get(0)?,
                count: row.get::<_, i64>(1)?.max(0) as u64,
                total_size: row.get::<_, i64>(2)?.max(0) as u64,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn drive_sizes(&self) -> Result<Vec<(String, u64)>, AppError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare("SELECT drive, COALESCE(SUM(size),0) FROM files WHERE is_directory=0 GROUP BY drive")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?.max(0) as u64))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn counts(&self) -> Result<IndexCounts, AppError> {
        let connection = self.lock()?;
        let (files, directories): (i64, i64) = connection.query_row(
            "SELECT COALESCE(SUM(CASE WHEN is_directory=0 THEN 1 ELSE 0 END),0), COALESCE(SUM(CASE WHEN is_directory=1 THEN 1 ELSE 0 END),0) FROM files",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(IndexCounts { indexed_files: files.max(0) as u64, indexed_directories: directories.max(0) as u64 })
    }

    pub fn path_entry(&self, path: &str) -> Result<Option<IndexedEntry>, AppError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT id, path, parent_path, name, ext, category, size, mtime, ctime, is_hidden, is_cloud, is_directory, drive FROM files WHERE path=?1",
                [path],
                map_entry,
            )
            .optional()
            .map_err(AppError::from)
    }
}

fn insert_records(transaction: &Transaction<'_>, records: &[FileRecord]) -> Result<(), AppError> {
    let mut statement = transaction.prepare_cached(
        "INSERT INTO files(path,parent_path,name,ext,category,size,mtime,ctime,is_hidden,is_cloud,is_directory,drive,last_seen)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
         ON CONFLICT(path) DO UPDATE SET parent_path=excluded.parent_path,name=excluded.name,ext=excluded.ext,
         category=excluded.category,size=excluded.size,mtime=excluded.mtime,ctime=excluded.ctime,
         is_hidden=excluded.is_hidden,is_cloud=excluded.is_cloud,is_directory=excluded.is_directory,drive=excluded.drive,last_seen=excluded.last_seen",
    )?;
    for record in records {
        statement.execute(params![
            record.path,
            record.parent_path,
            record.name,
            record.ext,
            record.category,
            record.size.min(i64::MAX as u64) as i64,
            record.mtime,
            record.ctime,
            record.is_hidden,
            record.is_cloud,
            record.is_directory,
            record.drive,
            record.last_seen,
        ])?;
    }
    Ok(())
}

fn map_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<IndexedEntry> {
    Ok(IndexedEntry {
        id: row.get(0)?,
        path: row.get(1)?,
        parent_path: row.get(2)?,
        name: row.get(3)?,
        ext: row.get(4)?,
        category: row.get(5)?,
        size: row.get::<_, i64>(6)?.max(0) as u64,
        mtime: row.get(7)?,
        ctime: row.get(8)?,
        is_hidden: row.get(9)?,
        is_cloud: row.get(10)?,
        is_directory: row.get(11)?,
        drive: row.get(12)?,
    })
}

fn fts_prefix_query(input: &str) -> Result<String, AppError> {
    let terms = input.split_whitespace().filter(|term| !term.is_empty()).collect::<Vec<_>>();
    if terms.is_empty() { return Err(AppError::InvalidRequest); }
    Ok(terms
        .into_iter()
        .map(|term| format!("name : \"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND "))
}

pub fn database_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("index.sqlite")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_query_is_a_quoted_prefix_query() {
        assert_eq!(fts_prefix_query("cat doc").expect("query should be valid"), "name : \"cat\"* AND name : \"doc\"*");
    }

    #[test]
    fn fts_query_rejects_blank_input() {
        assert!(fts_prefix_query("  \n ").is_err());
    }
}
