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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenElement {
    pub id: i64,
    pub host: String,
    pub selector: String,
    pub label: String,
    pub note: Option<String>,
    pub created_at: i64,
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
    pub folder_id: Option<String>,
    pub custom_letter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SavedFolder {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub position: i32,
    pub is_open: bool,
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
                created_at INTEGER NOT NULL,
                folder_id TEXT
            );

            CREATE TABLE IF NOT EXISTS folders (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                icon TEXT NOT NULL,
                position INTEGER NOT NULL,
                is_open INTEGER NOT NULL DEFAULT 1
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

            CREATE TABLE IF NOT EXISTS hidden_elements (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                host TEXT NOT NULL,
                selector TEXT NOT NULL,
                label TEXT NOT NULL,
                note TEXT,
                created_at INTEGER NOT NULL
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_hidden_host_sel ON hidden_elements(host, selector);
            CREATE INDEX IF NOT EXISTS idx_hidden_host ON hidden_elements(host);

            CREATE INDEX IF NOT EXISTS idx_history_url ON history(url);
            CREATE INDEX IF NOT EXISTS idx_history_visited ON history(visited_at DESC);

            CREATE TABLE IF NOT EXISTS shield_paused (
                host TEXT PRIMARY KEY,
                paused_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS site_permissions (
                host TEXT NOT NULL,
                permission TEXT NOT NULL,
                state INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY(host, permission)
            );

            CREATE TABLE IF NOT EXISTS site_settings (
                host TEXT PRIMARY KEY,
                zoom REAL DEFAULT 1.0,
                keep_awake INTEGER NOT NULL DEFAULT 0,
                autoplay INTEGER NOT NULL DEFAULT 1,
                updated_at INTEGER NOT NULL
            );
            "#,
        )?;
        let _ = self.conn.execute("ALTER TABLE tabs ADD COLUMN custom_title INTEGER NOT NULL DEFAULT 0", []);
        let _ = self.conn.execute("ALTER TABLE tabs ADD COLUMN folder_id TEXT", []);
        let _ = self.conn.execute("ALTER TABLE tabs ADD COLUMN custom_letter TEXT", []);
        let _ = self.conn.execute("ALTER TABLE folders ADD COLUMN is_open INTEGER NOT NULL DEFAULT 1", []);
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

    pub fn save_folders_state(&self, folders: &[SavedFolder]) -> Result<(), StorageError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM folders", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO folders (id, name, icon, position, is_open) VALUES (?1, ?2, ?3, ?4, ?5)"
            )?;
            for f in folders {
                stmt.execute(params![
                    f.id,
                    f.name,
                    f.icon,
                    f.position,
                    if f.is_open { 1 } else { 0 },
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_saved_folders(&self) -> Result<Vec<SavedFolder>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, icon, position, is_open FROM folders ORDER BY position ASC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SavedFolder {
                id: row.get(0)?,
                name: row.get(1)?,
                icon: row.get(2)?,
                position: row.get(3)?,
                is_open: row.get::<_, i32>(4).map(|v| v != 0).unwrap_or(true),
            })
        })?;
        let mut res = Vec::new();
        for r in rows {
            res.push(r?);
        }
        Ok(res)
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
                "INSERT INTO tabs (id, title, url, position, is_pinned, custom_title, space_id, created_at, folder_id, custom_letter) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'default', ?7, ?8, ?9)"
            )?;
            for t in tabs {
                stmt.execute(params![
                    t.id,
                    t.title,
                    t.url,
                    t.position,
                    if t.is_pinned { 1 } else { 0 },
                    if t.custom_title { 1 } else { 0 },
                    now,
                    t.folder_id,
                    t.custom_letter,
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
            "SELECT id, title, url, position, is_pinned, custom_title, folder_id, custom_letter FROM tabs WHERE space_id = 'default' ORDER BY position ASC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SavedTab {
                id: row.get(0)?,
                title: row.get(1)?,
                url: row.get(2)?,
                position: row.get(3)?,
                is_pinned: row.get::<_, i32>(4)? != 0,
                custom_title: row.get::<_, i32>(5).unwrap_or(0) != 0,
                folder_id: row.get(6).ok(),
                custom_letter: row.get(7).ok(),
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

    pub fn normalize_host(host: &str) -> String {
        let trimmed = host.trim().to_ascii_lowercase();
        let domain_only = if let Some(stripped) = trimmed.strip_prefix("http://") {
            stripped
        } else if let Some(stripped) = trimmed.strip_prefix("https://") {
            stripped
        } else {
            &trimmed
        };
        let host_part = domain_only.split(['/', ':', '?', '#']).next().unwrap_or(domain_only);
        if let Some(stripped) = host_part.strip_prefix("www.") {
            stripped.to_string()
        } else {
            host_part.to_string()
        }
    }

    pub fn add_hidden_element(
        &self,
        host: &str,
        selector: &str,
        label: &str,
        note: Option<&str>,
    ) -> Result<i64, StorageError> {
        let norm_host = Self::normalize_host(host);
        let sel = selector.trim();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        self.conn.execute(
            r#"
            INSERT INTO hidden_elements (host, selector, label, note, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(host, selector) DO UPDATE SET
                label = excluded.label,
                note = excluded.note,
                created_at = excluded.created_at
            "#,
            params![norm_host, sel, label.trim(), note.map(|n| n.trim()), now],
        )?;

        let id = self.conn.last_insert_rowid();
        Ok(id)
    }

    pub fn get_hidden_elements(&self, host: &str) -> Result<Vec<HiddenElement>, StorageError> {
        let norm_host = Self::normalize_host(host);
        let mut stmt = self.conn.prepare(
            "SELECT id, host, selector, label, note, created_at FROM hidden_elements WHERE host = ?1 ORDER BY created_at ASC"
        )?;
        let rows = stmt.query_map(params![norm_host], |row| {
            Ok(HiddenElement {
                id: row.get(0)?,
                host: row.get(1)?,
                selector: row.get(2)?,
                label: row.get(3)?,
                note: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;

        let mut res = Vec::new();
        for r in rows {
            res.push(r?);
        }
        Ok(res)
    }

    pub fn get_hidden_elements_css(
        &self,
        host: &str,
        without_selector: Option<&str>,
    ) -> Result<String, StorageError> {
        let elements = self.get_hidden_elements(host)?;
        let mut css = String::new();
        for elem in elements {
            if let Some(without) = without_selector {
                if elem.selector == without {
                    continue;
                }
            }
            css.push_str(&format!("{} {{ display: none !important; }}\n", elem.selector));
        }
        Ok(css)
    }

    pub fn remove_hidden_element(&self, id: i64) -> Result<(), StorageError> {
        self.conn.execute(
            "DELETE FROM hidden_elements WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn remove_hidden_element_by_selector(&self, host: &str, selector: &str) -> Result<(), StorageError> {
        let norm_host = Self::normalize_host(host);
        self.conn.execute(
            "DELETE FROM hidden_elements WHERE host = ?1 AND selector = ?2",
            params![norm_host, selector.trim()],
        )?;
        Ok(())
    }

    pub fn pop_last_hidden_element(&self, host: &str) -> Result<Option<HiddenElement>, StorageError> {
        let norm_host = Self::normalize_host(host);
        let last = self.conn.query_row(
            "SELECT id, host, selector, label, note, created_at FROM hidden_elements WHERE host = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
            params![norm_host],
            |row| {
                Ok(HiddenElement {
                    id: row.get(0)?,
                    host: row.get(1)?,
                    selector: row.get(2)?,
                    label: row.get(3)?,
                    note: row.get(4)?,
                    created_at: row.get(5)?,
                })
            },
        );

        match last {
            Ok(elem) => {
                self.remove_hidden_element(elem.id)?;
                Ok(Some(elem))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    pub fn clear_hidden_elements(&self, host: &str) -> Result<usize, StorageError> {
        let norm_host = Self::normalize_host(host);
        let rows = self.conn.execute(
            "DELETE FROM hidden_elements WHERE host = ?1",
            params![norm_host],
        )?;
        Ok(rows)
    }

    pub fn is_shield_paused(&self, host: &str) -> Result<bool, StorageError> {
        let norm = Self::normalize_host(host);
        let mut stmt = self.conn.prepare("SELECT 1 FROM shield_paused WHERE host = ?1")?;
        let exists = stmt.exists(params![norm])?;
        Ok(exists)
    }

    pub fn set_shield_paused(&self, host: &str, paused: bool) -> Result<(), StorageError> {
        let norm = Self::normalize_host(host);
        if norm.is_empty() {
            return Ok(());
        }
        if paused {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            self.conn.execute(
                "INSERT INTO shield_paused (host, paused_at) VALUES (?1, ?2) ON CONFLICT(host) DO NOTHING",
                params![norm, now],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM shield_paused WHERE host = ?1",
                params![norm],
            )?;
        }
        Ok(())
    }

    pub fn get_all_shield_paused(&self) -> Result<Vec<String>, StorageError> {
        let mut stmt = self.conn.prepare("SELECT host FROM shield_paused ORDER BY host ASC")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn get_shield_enabled(&self) -> Result<bool, StorageError> {
        match self.get_setting("shield_enabled")? {
            Some(v) => Ok(v != "false" && v != "0"),
            None => Ok(true),
        }
    }

    pub fn set_shield_enabled(&self, enabled: bool) -> Result<(), StorageError> {
        self.set_setting("shield_enabled", if enabled { "true" } else { "false" })
    }

    pub fn get_site_permission(&self, host: &str, perm: &str) -> Result<u8, StorageError> {
        let norm = Self::normalize_host(host);
        let mut stmt = self.conn.prepare("SELECT state FROM site_permissions WHERE host = ?1 AND permission = ?2")?;
        let mut rows = stmt.query(params![norm, perm])?;
        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Ok(0) // Default: 0 = Ask
        }
    }

    pub fn set_site_permission(&self, host: &str, perm: &str, state: u8) -> Result<(), StorageError> {
        let norm = Self::normalize_host(host);
        if norm.is_empty() {
            return Ok(());
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.conn.execute(
            r#"
            INSERT INTO site_permissions (host, permission, state, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(host, permission) DO UPDATE SET
                state = excluded.state,
                updated_at = excluded.updated_at
            "#,
            params![norm, perm, state as i32, now],
        )?;
        Ok(())
    }

    pub fn get_all_site_permissions(&self, host: &str) -> Result<std::collections::HashMap<String, u8>, StorageError> {
        let norm = Self::normalize_host(host);
        let mut stmt = self.conn.prepare("SELECT permission, state FROM site_permissions WHERE host = ?1")?;
        let rows = stmt.query_map(params![norm], |row| {
            let perm: String = row.get(0)?;
            let state: i32 = row.get(1)?;
            Ok((perm, state as u8))
        })?;
        let mut map = std::collections::HashMap::new();
        for r in rows {
            let (k, v) = r?;
            map.insert(k, v);
        }
        Ok(map)
    }

    pub fn get_site_keep_awake(&self, host: &str) -> Result<bool, StorageError> {
        let norm = Self::normalize_host(host);
        let mut stmt = self.conn.prepare("SELECT keep_awake FROM site_settings WHERE host = ?1")?;
        let mut rows = stmt.query(params![norm])?;
        if let Some(row) = rows.next()? {
            let val: i32 = row.get(0)?;
            Ok(val != 0)
        } else {
            Ok(false)
        }
    }

    pub fn set_site_keep_awake(&self, host: &str, keep_awake: bool) -> Result<(), StorageError> {
        let norm = Self::normalize_host(host);
        if norm.is_empty() {
            return Ok(());
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.conn.execute(
            r#"
            INSERT INTO site_settings (host, keep_awake, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(host) DO UPDATE SET
                keep_awake = excluded.keep_awake,
                updated_at = excluded.updated_at
            "#,
            params![norm, if keep_awake { 1 } else { 0 }, now],
        )?;
        Ok(())
    }

    pub fn get_site_autoplay(&self, host: &str) -> Result<bool, StorageError> {
        let norm = Self::normalize_host(host);
        let mut stmt = self.conn.prepare("SELECT autoplay FROM site_settings WHERE host = ?1")?;
        let mut rows = stmt.query(params![norm])?;
        if let Some(row) = rows.next()? {
            let val: i32 = row.get(0)?;
            Ok(val != 0)
        } else {
            Ok(true) // Autoplay allowed by default
        }
    }

    pub fn set_site_autoplay(&self, host: &str, allow: bool) -> Result<(), StorageError> {
        let norm = Self::normalize_host(host);
        if norm.is_empty() {
            return Ok(());
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.conn.execute(
            r#"
            INSERT INTO site_settings (host, autoplay, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(host) DO UPDATE SET
                autoplay = excluded.autoplay,
                updated_at = excluded.updated_at
            "#,
            params![norm, if allow { 1 } else { 0 }, now],
        )?;
        Ok(())
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
                folder_id: None,
                custom_letter: Some("M".into()),
            },
            SavedTab {
                id: "pin-custom".into(),
                title: "My Custom Pin".into(),
                url: "https://news.ycombinator.com".into(),
                position: 1,
                is_pinned: true,
                custom_title: true,
                folder_id: Some("f-work".into()),
                custom_letter: None,
            },
            SavedTab {
                id: "tab-1".into(),
                title: "Wikipedia".into(),
                url: "https://en.wikipedia.org".into(),
                position: 2,
                is_pinned: false,
                custom_title: false,
                folder_id: None,
                custom_letter: None,
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
        assert_eq!(loaded[1].folder_id.as_deref(), Some("f-work"));

        let folders = vec![
            SavedFolder {
                id: "f-work".into(),
                name: "Work".into(),
                icon: "<>".into(),
                position: 0,
                is_open: true,
            },
        ];
        storage.save_folders_state(&folders).expect("save folders");
        let loaded_folders = storage.get_saved_folders().expect("get folders");
        assert_eq!(loaded_folders.len(), 1);
        assert_eq!(loaded_folders[0].name, "Work");
        assert_eq!(loaded_folders[0].icon, "<>");
        assert!(loaded_folders[0].is_open);

        let active_idx = storage.get_saved_active_tab_idx().expect("get active tab idx");
        assert_eq!(active_idx, Some(2));
    }

    #[test]
    fn test_hidden_elements() {
        let storage = Storage::open_in_memory().expect("open in memory");

        // Normalize host testing
        assert_eq!(Storage::normalize_host("https://www.youtube.com/watch?v=123"), "youtube.com");
        assert_eq!(Storage::normalize_host("http://reddit.com/r/rust"), "reddit.com");
        assert_eq!(Storage::normalize_host("WWW.GitHub.com"), "github.com");

        // Add hidden elements
        let id1 = storage
            .add_hidden_element("www.youtube.com", "#masthead", "YouTube Header", Some("1200x56 top center"))
            .expect("add id1");
        assert!(id1 > 0);

        let id2 = storage
            .add_hidden_element("youtube.com", "#related", "Sidebar Recommendations", Some("400x800 middle right"))
            .expect("add id2");
        assert!(id2 > 0);

        // Fetch elements for youtube.com
        let list = storage.get_hidden_elements("https://youtube.com").expect("get elements");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].selector, "#masthead");
        assert_eq!(list[0].label, "YouTube Header");
        assert_eq!(list[1].selector, "#related");

        // Check CSS generation
        let css = storage.get_hidden_elements_css("youtube.com", None).expect("get css");
        assert!(css.contains("#masthead { display: none !important; }"));
        assert!(css.contains("#related { display: none !important; }"));

        // CSS with spared selector (peeking)
        let css_peeking = storage.get_hidden_elements_css("youtube.com", Some("#related")).expect("peek css");
        assert!(css_peeking.contains("#masthead { display: none !important; }"));
        assert!(!css_peeking.contains("#related { display: none !important; }"));

        // Pop last element (Undo)
        let undone = storage.pop_last_hidden_element("youtube.com").expect("undo");
        assert!(undone.is_some());
        assert_eq!(undone.unwrap().selector, "#related");

        let remaining = storage.get_hidden_elements("youtube.com").expect("remaining");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].selector, "#masthead");

        // Clear all
        let cleared = storage.clear_hidden_elements("youtube.com").expect("clear");
        assert_eq!(cleared, 1);
        assert!(storage.get_hidden_elements("youtube.com").unwrap().is_empty());
    }

    #[test]
    fn test_shield_persistence() {
        let storage = Storage::open_in_memory().expect("open in memory");

        // Enabled by default
        assert!(storage.get_shield_enabled().unwrap());

        // Toggle global shield
        storage.set_shield_enabled(false).unwrap();
        assert!(!storage.get_shield_enabled().unwrap());
        storage.set_shield_enabled(true).unwrap();
        assert!(storage.get_shield_enabled().unwrap());

        // Per-site pause
        assert!(!storage.is_shield_paused("nytimes.com").unwrap());
        assert!(!storage.is_shield_paused("https://www.nytimes.com/section/world").unwrap());

        storage.set_shield_paused("https://www.nytimes.com", true).unwrap();
        assert!(storage.is_shield_paused("nytimes.com").unwrap());
        assert!(storage.is_shield_paused("www.nytimes.com").unwrap());

        let paused = storage.get_all_shield_paused().unwrap();
        assert_eq!(paused, vec!["nytimes.com"]);

        // Resume / unpause
        storage.set_shield_paused("nytimes.com", false).unwrap();
        assert!(!storage.is_shield_paused("nytimes.com").unwrap());
        assert!(storage.get_all_shield_paused().unwrap().is_empty());
    }

    #[test]
    fn test_site_permissions_and_settings() {
        let storage = Storage::open_in_memory().expect("open in memory");

        // Default permission is 0 (Ask)
        assert_eq!(storage.get_site_permission("kissasian.cam", "camera").unwrap(), 0);
        assert_eq!(storage.get_site_permission("kissasian.cam", "notifications").unwrap(), 0);

        // Set permission (1 = Allow, 2 = Block)
        storage.set_site_permission("kissasian.cam", "camera", 1).unwrap();
        storage.set_site_permission("https://kissasian.cam/watch", "microphone", 2).unwrap();

        assert_eq!(storage.get_site_permission("kissasian.cam", "camera").unwrap(), 1);
        assert_eq!(storage.get_site_permission("kissasian.cam", "microphone").unwrap(), 2);

        let perms = storage.get_all_site_permissions("kissasian.cam").unwrap();
        assert_eq!(perms.get("camera"), Some(&1));
        assert_eq!(perms.get("microphone"), Some(&2));

        // Keep awake default is false
        assert!(!storage.get_site_keep_awake("kissasian.cam").unwrap());
        storage.set_site_keep_awake("kissasian.cam", true).unwrap();
        assert!(storage.get_site_keep_awake("kissasian.cam").unwrap());

        // Autoplay default is true
        assert!(storage.get_site_autoplay("kissasian.cam").unwrap());
        storage.set_site_autoplay("kissasian.cam", false).unwrap();
        assert!(!storage.get_site_autoplay("kissasian.cam").unwrap());
    }
}

