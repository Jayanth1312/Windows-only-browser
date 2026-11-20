use rusqlite::{params, Connection};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub visited_at: i64,
}

#[derive(Debug, Clone)]
pub struct SavedTab {
    pub id: String,
    pub title: String,
    pub url: String,
    pub position: i32,
    pub is_pinned: bool,
    pub custom_title: bool,
}

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    fn init_schema(&self) -> Result<(), StorageError> {
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA temp_store = MEMORY;
            PRAGMA cache_size = -16000;

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tabs (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                url TEXT NOT NULL,
                position INTEGER NOT NULL,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                custom_title INTEGER NOT NULL DEFAULT 0,
                space_id TEXT NOT NULL DEFAULT 'default',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL,
                title TEXT NOT NULL,
                visited_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS favicons (
                domain TEXT PRIMARY KEY,
                png_bytes BLOB NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_history_url ON history(url);
            CREATE INDEX IF NOT EXISTS idx_history_visited ON history(visited_at DESC);
            "#,
        )?;
        let _ = self.conn.execute("ALTER TABLE tabs ADD COLUMN custom_title INTEGER NOT NULL DEFAULT 0", []);
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        let mut stmt = self.conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn add_history(&self, url: &str, title: &str) -> Result<(), StorageError> {
        let url = url.trim();
        if url.is_empty() || url == "about:blank" {
            return Ok(());
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        self.conn.execute(
            "INSERT INTO history (url, title, visited_at) VALUES (?1, ?2, ?3)",
            params![url, title, now],
        )?;
        Ok(())
    }

    pub fn query_history(&self, query: &str, limit: usize) -> Result<Vec<HistoryEntry>, StorageError> {
        let q = query.trim();
        let pattern = format!("%{}%", q);
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, url, title, visited_at
            FROM history
            WHERE url LIKE ?1 OR title LIKE ?1
            GROUP BY url
            ORDER BY visited_at DESC
            LIMIT ?2
            "#,
        )?;

        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                visited_at: row.get(3)?,
            })
        })?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    pub fn recent_history(&self, limit: usize) -> Result<Vec<HistoryEntry>, StorageError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, url, title, MAX(visited_at) as latest_visited
            FROM history
            GROUP BY url
            ORDER BY latest_visited DESC
            LIMIT ?1
            "#,
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                visited_at: row.get(3)?,
            })
        })?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    pub fn save_favicon(&self, domain: &str, png_bytes: &[u8]) -> Result<(), StorageError> {
        let domain = domain.trim().to_lowercase();
        if domain.is_empty() || png_bytes.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO favicons (domain, png_bytes) VALUES (?1, ?2) ON CONFLICT(domain) DO UPDATE SET png_bytes = ?2",
            params![domain, png_bytes],
        )?;
        Ok(())
    }

    pub fn get_favicon(&self, domain: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let domain = domain.trim().to_lowercase();
        let mut stmt = self.conn.prepare("SELECT png_bytes FROM favicons WHERE domain = ?1")?;
        let mut rows = stmt.query(params![domain])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn save_tabs_state(&self, tabs: &[SavedTab], active_idx: usize) -> Result<(), StorageError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM tabs WHERE space_id = 'default'", [])?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        {
            let mut stmt = tx.prepare(
                "INSERT INTO tabs (id, title, url, position, is_pinned, custom_title, space_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'default', ?7)"
            )?;
            for t in tabs {
                stmt.execute(params![
                    t.id,
                    t.title,
                    t.url,
                    t.position,
                    if t.is_pinned { 1 } else { 0 },
                    if t.custom_title { 1 } else { 0 },
                    now
                ])?;
            }
        }
        tx.execute(
            "INSERT INTO settings (key, value) VALUES ('active_tab_idx', ?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
            params![active_idx.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_saved_tabs(&self) -> Result<Vec<SavedTab>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, url, position, is_pinned, custom_title FROM tabs WHERE space_id = 'default' ORDER BY position ASC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SavedTab {
                id: row.get(0)?,
                title: row.get(1)?,
                url: row.get(2)?,
                position: row.get(3)?,
                is_pinned: row.get::<_, i32>(4)? != 0,
                custom_title: row.get::<_, i32>(5).unwrap_or(0) != 0,
            })
        })?;
        let mut res = Vec::new();
        for r in rows {
            res.push(r?);
        }
        Ok(res)
    }

    pub fn get_saved_active_tab_idx(&self) -> Result<Option<usize>, StorageError> {
        if let Some(val) = self.get_setting("active_tab_idx")? {
            if let Ok(idx) = val.parse::<usize>() {
                return Ok(Some(idx));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tabs_state_persistence() {
        let storage = Storage::open_in_memory().expect("open in memory");
        let tabs = vec![
            SavedTab {
                id: "pin-gmail".into(),
                title: "Gmail".into(),
                url: "https://mail.google.com".into(),
                position: 0,
                is_pinned: true,
                custom_title: false,
            },
            SavedTab {
                id: "pin-custom".into(),
                title: "My Custom Pin".into(),
                url: "https://news.ycombinator.com".into(),
                position: 1,
                is_pinned: true,
                custom_title: true,
            },
            SavedTab {
                id: "tab-1".into(),
                title: "Wikipedia".into(),
                url: "https://en.wikipedia.org".into(),
                position: 2,
                is_pinned: false,
                custom_title: false,
            },
        ];

        storage.save_tabs_state(&tabs, 2).expect("save tabs state");

        let loaded = storage.get_saved_tabs().expect("get saved tabs");
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].id, "pin-gmail");
        assert!(loaded[0].is_pinned);
        assert_eq!(loaded[1].title, "My Custom Pin");
        assert!(loaded[1].custom_title);
        assert_eq!(loaded[2].url, "https://en.wikipedia.org");
        assert!(!loaded[2].is_pinned);

        let active_idx = storage.get_saved_active_tab_idx().expect("get active tab idx");
        assert_eq!(active_idx, Some(2));
    }
}

