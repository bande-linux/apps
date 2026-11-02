use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use web_host::Url;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS pages (
        url TEXT PRIMARY KEY,
        title TEXT NOT NULL DEFAULT '',
        visits INTEGER NOT NULL DEFAULT 0,
        last INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS visits (url TEXT NOT NULL, time INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS visits_by_time ON visits (time);
    CREATE TABLE IF NOT EXISTS bookmarks (
        url TEXT PRIMARY KEY,
        title TEXT NOT NULL DEFAULT '',
        favourite INTEGER NOT NULL DEFAULT 1,
        added INTEGER NOT NULL
    );
";
const MAX_TEXT: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    pub(crate) url: Url,
    pub(crate) title: String,
    pub(crate) time: u64,
    pub(crate) bookmarked: bool,
}

pub(crate) struct Places {
    db: Connection,
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn bounded(text: &str) -> &str {
    let mut end = text.len().min(MAX_TEXT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn recorded(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

impl Places {
    pub(crate) fn open(file: &Path) -> rusqlite::Result<Self> {
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let db = Connection::open(file)?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db })
    }

    pub(crate) fn in_memory() -> rusqlite::Result<Self> {
        let db = Connection::open_in_memory()?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db })
    }

    pub(crate) fn visit(&self, url: &Url, title: &str) -> rusqlite::Result<()> {
        if !recorded(url) {
            return Ok(());
        }
        let (url, title) = (bounded(url.as_str()), bounded(title));
        let time = i64::try_from(now()).unwrap_or(i64::MAX);
        self.db.execute(
            "INSERT INTO pages (url, title, visits, last) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT (url) DO UPDATE SET visits = visits + 1, last = ?3,
             title = CASE WHEN ?2 = '' THEN title ELSE ?2 END",
            params![url, title, time],
        )?;
        self.db.execute(
            "INSERT INTO visits (url, time) VALUES (?1, ?2)",
            params![url, time],
        )?;
        Ok(())
    }

    fn places(&self, sql: &str, parameters: impl rusqlite::Params) -> Vec<Place> {
        let Ok(mut statement) = self.db.prepare_cached(sql) else {
            return Vec::new();
        };
        let rows = statement.query_map(parameters, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, bool>(3)?,
            ))
        });
        rows.map(|rows| {
            rows.filter_map(Result::ok)
                .filter_map(|(url, title, time, bookmarked)| {
                    Some(Place {
                        url: Url::parse(&url).ok()?,
                        title,
                        time: u64::try_from(time).unwrap_or(0),
                        bookmarked,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
    }

    pub(crate) fn suggest(&self, text: &str, limit: usize) -> Vec<Place> {
        let text = bounded(text.trim());
        if text.is_empty() {
            return Vec::new();
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.places(
            "SELECT url, title, time, bookmarked FROM (
                 SELECT b.url, b.title, 0 AS time, 1 AS bookmarked, 1000000 AS rank FROM bookmarks b
                 WHERE instr(lower(b.url), lower(?1)) > 0 OR instr(lower(b.title), lower(?1)) > 0
                 UNION ALL
                 SELECT p.url, p.title, p.last, 0, p.visits * 100 + p.last / 86400 FROM pages p
                 WHERE (instr(lower(p.url), lower(?1)) > 0 OR instr(lower(p.title), lower(?1)) > 0)
                   AND p.url NOT IN (SELECT url FROM bookmarks)
             ) ORDER BY bookmarked DESC, rank DESC LIMIT ?2",
            params![text, limit],
        )
    }

    pub(crate) fn history(&self, limit: usize) -> Vec<Place> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.places(
            "SELECT v.url, coalesce(p.title, ''), v.time, 0 FROM visits v
             LEFT JOIN pages p ON p.url = v.url ORDER BY v.time DESC, v.rowid DESC LIMIT ?1",
            params![limit],
        )
    }

    pub(crate) fn recent(&self, limit: usize) -> Vec<Place> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.places(
            "SELECT url, title, last, 0 FROM pages ORDER BY last DESC LIMIT ?1",
            params![limit],
        )
    }

    pub(crate) fn bookmarks(&self) -> Vec<Place> {
        self.places(
            "SELECT url, title, added, 1 FROM bookmarks ORDER BY added",
            [],
        )
    }

    pub(crate) fn is_bookmarked(&self, url: &Url) -> bool {
        self.db
            .query_row(
                "SELECT 1 FROM bookmarks WHERE url = ?1",
                params![url.as_str()],
                |_| Ok(()),
            )
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    pub(crate) fn toggle_bookmark(&self, url: &Url, title: &str) -> rusqlite::Result<bool> {
        if self.is_bookmarked(url) {
            self.db.execute(
                "DELETE FROM bookmarks WHERE url = ?1",
                params![url.as_str()],
            )?;
            return Ok(false);
        }
        let added = i64::try_from(now()).unwrap_or(i64::MAX);
        self.db.execute(
            "INSERT INTO bookmarks (url, title, added) VALUES (?1, ?2, ?3)",
            params![bounded(url.as_str()), bounded(title), added],
        )?;
        Ok(true)
    }
}
