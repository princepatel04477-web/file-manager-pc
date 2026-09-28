use crate::error::AppError;
use crate::ops::display_path;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
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

/// A screenshot candidate plus whether it came from the Screenshots folder itself.
#[derive(Clone, Debug)]
pub struct ScreenshotRow {
    pub entry: IndexedEntry,
    pub inside_folder: bool,
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

/// A pinned file or folder.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteItem {
    pub path: String,
    pub name: String,
    pub is_directory: bool,
    pub added_at_unix: i64,
}

/// A file the user opened recently.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecentItem {
    pub path: String,
    pub name: String,
    pub opened_at_unix: i64,
}

// Deserialize as well as Serialize: this one crosses the IPC boundary inbound as a
// command argument, which is what `CommandArg` requires.
#[derive(Clone, Debug, Deserialize, Serialize, Type)]
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

/// How many recent files are kept.
pub const MAX_RECENTS: u32 = 40;

/// Bumped whenever a change alters what belongs in the index, so an existing
/// index built under the old rules is rebuilt rather than reconciled.
/// 2: AppData and the tool caches are no longer indexed.
const INDEX_VERSION: &str = "2";

pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(crate::ops::io_path(parent))?;
        }
        let connection = Connection::open(crate::ops::io_path(path))?;
        // `PRAGMA journal_mode` answers with the mode it settled on, and rusqlite's
        // `execute` path rejects a statement that returns rows, so read the answer.
        let _: String =
            connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "temp_store", "MEMORY")?;

        // The settings table has to exist before the index version can be read;
        // the batch below recreates it harmlessly either way.
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        let stored: Option<String> = connection
            .query_row("SELECT value FROM settings WHERE key = 'index_version'", [], |row| {
                row.get(0)
            })
            .optional()?;
        let stale = stored.as_deref() != Some(INDEX_VERSION);
        if stale {
            // What counts as worth indexing changed, so every row written under the
            // old rules is suspect. Walking an old index row by row to reconcile it
            // takes far longer than building a fresh one — on a profile that had
            // been indexed a few times it meant reading 1.8 million stale rows out
            // of a 1.3 GB file before the app was useful — so start clean instead.
            connection.execute_batch(
                "DROP TRIGGER IF EXISTS files_ai;
                 DROP TRIGGER IF EXISTS files_ad;
                 DROP TRIGGER IF EXISTS files_au;
                 DROP TABLE IF EXISTS files_fts;
                 DROP TABLE IF EXISTS files;",
            )?;
        }

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
             CREATE INDEX IF NOT EXISTS idx_files_size ON files(is_directory, size);
             CREATE INDEX IF NOT EXISTS idx_files_drive ON files(drive);
             CREATE TABLE IF NOT EXISTS favorites (
                 id INTEGER PRIMARY KEY,
                 path TEXT NOT NULL UNIQUE COLLATE NOCASE,
                 name TEXT NOT NULL,
                 is_directory INTEGER NOT NULL DEFAULT 0,
                 added_at INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS recents (
                 id INTEGER PRIMARY KEY,
                 path TEXT NOT NULL UNIQUE COLLATE NOCASE,
                 name TEXT NOT NULL,
                 opened_at INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_recents_opened ON recents(opened_at DESC);
             CREATE TABLE IF NOT EXISTS settings (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS exclusions (
                 path TEXT PRIMARY KEY COLLATE NOCASE,
                 label TEXT NOT NULL,
                 added_at INTEGER NOT NULL DEFAULT 0
             );
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

        if stale {
            connection.execute(
                "INSERT INTO settings (key, value) VALUES ('index_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [INDEX_VERSION],
            )?;
            // Deliberately no VACUUM here. `open` runs inside Tauri's setup hook on
            // the main thread, and rewriting a 1.3 GB file there held the window
            // for fifteen seconds — the very symptom this change exists to remove.
            // Dropping the tables leaves the pages on the free list, and the vacuum
            // at the end of the first scan reclaims them from the indexer thread.
        }
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
        let connection = self.lock()?;
        connection.execute("DELETE FROM files WHERE last_seen <> ?1", [epoch])?;
        // Every full rescan deletes the rows it did not see again, and SQLite keeps
        // those pages on its free list instead of returning them to the filesystem.
        // Left alone the file only ever grows: a profile indexed a handful of times
        // reached 1.37 GB for an index that needs tens of megabytes. Reclaim only
        // when a quarter of the file is waste, so a normal launch never pays for it.
        let pages: i64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let free: i64 = connection.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
        if pages > 0 && free.saturating_mul(4) > pages {
            // VACUUM rewrites the database, so it cannot run inside a transaction.
            connection.execute_batch("VACUUM")?;
        }
        Ok(())
    }

    pub fn remove_path(&self, path: &Path) -> Result<(), AppError> {
        let path = display_path(path);
        let child_prefix = descendant_prefix(&path);
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

    /// Pin a file or folder. Pinning the same path again just refreshes the name.
    pub fn add_favorite(&self, path: &str, name: &str, is_directory: bool, added_at: i64) -> Result<(), AppError> {
        self.lock()?.execute(
            "INSERT INTO favorites(path, name, is_directory, added_at) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET name=excluded.name, is_directory=excluded.is_directory",
            params![path, name, is_directory, added_at],
        )?;
        Ok(())
    }

    pub fn remove_favorite(&self, path: &str) -> Result<(), AppError> {
        self.lock()?.execute("DELETE FROM favorites WHERE path = ?1", [path])?;
        Ok(())
    }

    pub fn is_favorite(&self, path: &str) -> Result<bool, AppError> {
        let connection = self.lock()?;
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM favorites WHERE path = ?1",
            [path],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn favorites(&self) -> Result<Vec<FavoriteItem>, AppError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT path, name, is_directory, added_at FROM favorites ORDER BY added_at DESC, name COLLATE NOCASE ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(FavoriteItem {
                path: row.get(0)?,
                name: row.get(1)?,
                is_directory: row.get::<_, bool>(2)?,
                added_at_unix: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    /// Record an open, newest first, keeping at most [`MAX_RECENTS`] entries.
    pub fn push_recent(&self, path: &str, name: &str, opened_at: i64) -> Result<(), AppError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO recents(path, name, opened_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET name=excluded.name, opened_at=excluded.opened_at",
            params![path, name, opened_at],
        )?;
        transaction.execute(
            "DELETE FROM recents WHERE id NOT IN (SELECT id FROM recents ORDER BY opened_at DESC, id DESC LIMIT ?1)",
            [MAX_RECENTS],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn recents(&self, limit: u32) -> Result<Vec<RecentItem>, AppError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT path, name, opened_at FROM recents ORDER BY opened_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit.clamp(1, MAX_RECENTS)], |row| {
            Ok(RecentItem { path: row.get(0)?, name: row.get(1)?, opened_at_unix: row.get(2)? })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn clear_recents(&self) -> Result<(), AppError> {
        self.lock()?.execute("DELETE FROM recents", [])?;
        Ok(())
    }

    /// One persisted preference, or `None` when it has never been set. A missing
    /// value always falls back to the built-in default, never to a guess.
    pub fn setting(&self, key: &str) -> Result<Option<String>, AppError> {
        let connection = self.lock()?;
        connection
            .query_row::<String, _, _>("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(AppError::from)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), AppError> {
        self.lock()?.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Folders the user has excluded from the index, newest first.
    pub fn exclusions(&self) -> Result<Vec<(String, String, i64)>, AppError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT path, label, added_at FROM exclusions ORDER BY added_at DESC, path COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    /// Excluding a folder twice only refreshes its label.
    pub fn add_exclusion(&self, path: &str, label: &str, added_at: i64) -> Result<(), AppError> {
        self.lock()?.execute(
            "INSERT INTO exclusions (path, label, added_at) VALUES (?1, ?2, ?3) ON CONFLICT(path) DO UPDATE SET label=excluded.label",
            params![path, label, added_at],
        )?;
        Ok(())
    }

    pub fn remove_exclusion(&self, path: &str) -> Result<(), AppError> {
        self.lock()?
            .execute("DELETE FROM exclusions WHERE path = ?1", [path])?;
        Ok(())
    }

    /// Drop favourites and recents for paths that no longer exist, so both lists stay
    /// honest after a delete, move, or rename.
    pub fn forget_paths(&self, paths: &[String]) -> Result<(), AppError> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        {
            let mut favorites = transaction.prepare("DELETE FROM favorites WHERE path = ?1")?;
            let mut recents = transaction.prepare("DELETE FROM recents WHERE path = ?1")?;
            for path in paths {
                favorites.execute([path])?;
                recents.execute([path])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Everything at least `min_size` bytes, largest first. Directories are excluded.
    pub fn large_files(&self, min_size: u64, limit: u32) -> Result<Vec<IndexedEntry>, AppError> {
        let min_size = min_size.min(i64::MAX as u64) as i64;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, path, parent_path, name, ext, category, size, mtime, ctime, is_hidden, is_cloud, is_directory, drive \
             FROM files WHERE is_directory = 0 AND size >= ?1 \
             ORDER BY size DESC, name COLLATE NOCASE ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![min_size, limit], map_entry)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    /// How many files and how many bytes sit above `min_size`, ignoring the list cap.
    pub fn large_file_totals(&self, min_size: u64) -> Result<(u64, u64), AppError> {
        let min_size = min_size.min(i64::MAX as u64) as i64;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files WHERE is_directory = 0 AND size >= ?1",
                params![min_size],
                |row| Ok((row.get::<_, i64>(0)?.max(0) as u64, row.get::<_, i64>(1)?.max(0) as u64)),
            )
            .map_err(AppError::from)
    }

    /// The same totals for one folder tree, without the row cap.
    pub fn older_than_under_totals(&self, parent: &Path, cutoff_unix: i64) -> Result<(u64, u64), AppError> {
        let prefix = descendant_prefix(&display_path(parent));
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files WHERE is_directory = 0 \
                 AND substr(path, 1, length(?1)) = ?1 AND mtime IS NOT NULL AND mtime < ?2",
                params![prefix, cutoff_unix],
                |row| Ok((row.get::<_, i64>(0)?.max(0) as u64, row.get::<_, i64>(1)?.max(0) as u64)),
            )
            .map_err(AppError::from)
    }

    /// Non-directory entries anywhere under `parent` whose modification time is older
    /// than `cutoff_unix`, oldest first. Entries with no recorded time are ignored.
    pub fn files_older_than_under(
        &self,
        parent: &Path,
        cutoff_unix: i64,
        limit: u32,
    ) -> Result<Vec<IndexedEntry>, AppError> {
        let prefix = descendant_prefix(&display_path(parent));
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, path, parent_path, name, ext, category, size, mtime, ctime, is_hidden, is_cloud, is_directory, drive \
             FROM files WHERE is_directory = 0 AND substr(path, 1, length(?1)) = ?1 \
               AND mtime IS NOT NULL AND mtime < ?2 \
             ORDER BY mtime ASC, size DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![prefix, cutoff_unix, limit], map_entry)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    /// Files that look like captures. The name patterns here are deliberately broader
    /// than `clean::rules::is_screenshot_name`, which has the final say on every row;
    /// `inside_folder` tells that rule whether the row came from the Screenshots folder.
    pub fn screenshot_candidates(
        &self,
        folder_prefix: &str,
        limit: u32,
    ) -> Result<Vec<ScreenshotRow>, AppError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, path, parent_path, name, ext, category, size, mtime, ctime, is_hidden, is_cloud, is_directory, drive, \
                    substr(path, 1, length(?1)) = ?1 \
             FROM files WHERE is_directory = 0 \
               AND (substr(path, 1, length(?1)) = ?1 \
                    OR lower(name) LIKE '%screenshot%' OR lower(name) LIKE '%screen shot%' \
                    OR lower(name) LIKE '%screen-shot%' OR lower(name) LIKE '%screen_shot%' \
                    OR lower(name) LIKE '%snipping%' OR lower(name) LIKE '%snip %' \
                    OR lower(name) LIKE '%capture%') \
             ORDER BY size DESC, mtime ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![folder_prefix, limit], |row| {
            Ok(ScreenshotRow { entry: map_entry(row)?, inside_folder: row.get::<_, i64>(13)? != 0 })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    /// Sizes that more than one non-cloud file shares, biggest total first. These are
    /// the only sizes worth hashing.
    pub fn duplicate_size_groups(&self, min_size: u64, limit: u32) -> Result<Vec<u64>, AppError> {
        let min_size = min_size.min(i64::MAX as u64) as i64;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT size FROM files WHERE is_directory = 0 AND is_cloud = 0 AND size >= ?1 \
             GROUP BY size HAVING COUNT(*) > 1 ORDER BY size * COUNT(*) DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![min_size, limit], |row| {
            Ok(row.get::<_, i64>(0)?.max(0) as u64)
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    /// The members of those size groups — the input to the duplicate hash pipeline.
    pub fn duplicate_candidates(&self, sizes: &[u64]) -> Result<Vec<IndexedEntry>, AppError> {
        if sizes.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = sizes.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id, path, parent_path, name, ext, category, size, mtime, ctime, is_hidden, is_cloud, is_directory, drive \
             FROM files WHERE is_directory = 0 AND is_cloud = 0 AND size IN ({placeholders}) \
             ORDER BY size DESC, path ASC"
        );
        let connection = self.lock()?;
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(sizes.iter().map(|size| (*size).min(i64::MAX as u64) as i64)),
            map_entry,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
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

/// `C:\Users\me\Downloads` → `C:\Users\me\Downloads\` so a prefix match cannot
/// pick up a sibling like `Downloads 2`.
fn descendant_prefix(path: &str) -> String {
    let separator = if cfg!(windows) { '\\' } else { '/' };
    if path.ends_with('\\') || path.ends_with('/') {
        path.to_owned()
    } else {
        format!("{path}{separator}")
    }
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

    fn temp_database(tag: &str) -> (Database, PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sift-db-{tag}-{unique}")).join("index.sqlite");
        let database = Database::open(&path).expect("database opens");
        (database, path)
    }

    #[test]
    fn favourites_round_trip_and_are_case_insensitive() {
        let (database, path) = temp_database("favourites");
        database.add_favorite("C:\\Users\\u\\Documents\\Report.pdf", "Report.pdf", false, 1_700_000_010).expect("add");
        database.add_favorite("C:\\Users\\u\\Pictures", "Pictures", true, 1_700_000_020).expect("add");
        // Pinning the same path with different casing must not create a second row.
        database.add_favorite("c:\\users\\u\\documents\\report.pdf", "Report.pdf", false, 1_700_000_010).expect("re-add");

        let favourites = database.favorites().expect("list");
        assert_eq!(favourites.len(), 2);
        assert_eq!(favourites[0].name, "Pictures", "newest first");
        assert!(favourites[0].is_directory);
        assert!(database.is_favorite("C:\\USERS\\u\\Documents\\REPORT.pdf").expect("lookup"));
        assert!(!database.is_favorite("C:\\Users\\u\\Documents\\Other.pdf").expect("lookup"));

        database.remove_favorite("C:\\Users\\u\\Pictures").expect("remove");
        assert_eq!(database.favorites().expect("list").len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn recents_keep_the_newest_entries_and_deduplicate_paths() {
        let (database, path) = temp_database("recents");
        for index in 0..(MAX_RECENTS + 10) {
            database.push_recent(&format!("C:\\Users\\u\\file{index}.txt"), &format!("file{index}.txt"), 1_700_000_000 + index as i64).expect("push");
        }
        let recents = database.recents(MAX_RECENTS).expect("list");
        assert_eq!(recents.len(), MAX_RECENTS as usize, "older entries are pruned");
        assert_eq!(recents[0].opened_at_unix, 1_700_000_000 + MAX_RECENTS as i64 + 9, "newest first");

        // Re-opening a file moves it to the top instead of duplicating it.
        database.push_recent("C:\\Users\\u\\file0.txt", "file0.txt", 1_800_000_000).expect("push");
        let recents = database.recents(MAX_RECENTS).expect("list");
        assert_eq!(recents.len(), MAX_RECENTS as usize);
        assert_eq!(recents[0].path, "C:\\Users\\u\\file0.txt");

        assert!(database.recents(5).expect("limit").len() == 5);
        database.clear_recents().expect("clear");
        assert!(database.recents(MAX_RECENTS).expect("list").is_empty());
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn forgetting_paths_clears_both_lists() {
        let (database, path) = temp_database("forget");
        database.add_favorite("C:\\Users\\u\\gone.pdf", "gone.pdf", false, 1).expect("add");
        database.push_recent("C:\\Users\\u\\gone.pdf", "gone.pdf", 2).expect("push");
        database.push_recent("C:\\Users\\u\\keep.pdf", "keep.pdf", 3).expect("push");

        database.forget_paths(&["C:\\Users\\u\\gone.pdf".to_owned()]).expect("forget");
        assert!(database.favorites().expect("list").is_empty());
        let recents = database.recents(MAX_RECENTS).expect("list");
        assert_eq!(recents.len(), 1);
        assert_eq!(recents[0].name, "keep.pdf");
        database.forget_paths(&[]).expect("empty is a no-op");
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }
}
