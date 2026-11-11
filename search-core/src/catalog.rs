//! Machine-level catalog (#240 P1-1): one small SQLite database that remembers which folders have a
//! fast index, where each index lives, and what files those indexes hold (path, size, mtime and,
//! when the semantic index has it, the content hash).
//!
//! The catalog is derived and additive. Per-folder indexes (ADR-011) stay the source of truth and
//! single-root search never touches the catalog. Deleting the catalog loses nothing: it is rebuilt
//! by registering the roots again. Cross-root search opens each registered root's own index and
//! unions the hits, so a cross-root query returns exactly what the per-root queries return.

use std::path::{Path, PathBuf};

use native_search::engine::{NativeSearchEngine, SearchHit};
use rusqlite::{params, Connection, OptionalExtension};

use crate::native_index;

/// A folder known to the catalog and the directory holding its fast index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRoot {
    pub path: String,
    pub index_dir: PathBuf,
}

/// One catalogued file: location, cheap freshness data and (optionally) its content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogFile {
    pub root: String,
    pub path: String,
    pub size: i64,
    pub modified_unix: i64,
    pub content_hash: Option<Vec<u8>>,
}

pub struct Catalog {
    pub(crate) conn: Connection,
}

impl Catalog {
    /// Opens (creating if needed) the catalog at `path`. A damaged database is recreated empty.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if path.exists() && !is_sound(path) {
            tracing::warn!("catalog is damaged; recreating it empty");
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let mut name = path.as_os_str().to_owned();
                name.push(suffix);
                let _ = std::fs::remove_file(PathBuf::from(name));
            }
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS roots(
                 id INTEGER PRIMARY KEY,
                 path TEXT NOT NULL UNIQUE,
                 index_dir TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS files(
                 root_id INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
                 path TEXT NOT NULL,
                 size INTEGER NOT NULL,
                 modified INTEGER NOT NULL,
                 content_hash BLOB,
                 PRIMARY KEY(root_id, path)
             );
             CREATE INDEX IF NOT EXISTS files_hash ON files(content_hash);
             CREATE TABLE IF NOT EXISTS entities(
                 root_id INTEGER NOT NULL,
                 path TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 value TEXT NOT NULL,
                 shown TEXT NOT NULL,
                 line INTEGER NOT NULL,
                 class TEXT NOT NULL,
                 extractor TEXT NOT NULL,
                 FOREIGN KEY(root_id,path) REFERENCES files(root_id,path) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS entities_value ON entities(kind,value);
             CREATE INDEX IF NOT EXISTS entities_file ON entities(root_id,path);",
        )?;
        let has_stamp = conn
            .prepare("SELECT 1 FROM pragma_table_info('files') WHERE name='entities_stamp'")?
            .exists([])?;
        if !has_stamp {
            conn.execute_batch("ALTER TABLE files ADD COLUMN entities_stamp TEXT;")?;
        }
        Ok(Self { conn })
    }

    /// Adds or updates a root. Returns its id.
    pub fn register_root(&self, path: &str, index_dir: &Path) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO roots(path,index_dir) VALUES(?1,?2)
             ON CONFLICT(path) DO UPDATE SET index_dir=excluded.index_dir",
            params![path, index_dir.to_string_lossy()],
        )?;
        self.conn
            .query_row("SELECT id FROM roots WHERE path=?1", [path], |r| r.get(0))
    }

    /// Forgets a root and its files. The root's own index on disk is left alone.
    pub fn remove_root(&self, path: &str) -> rusqlite::Result<bool> {
        Ok(self.conn.execute("DELETE FROM roots WHERE path=?1", [path])? > 0)
    }

    pub fn roots(&self) -> rusqlite::Result<Vec<CatalogRoot>> {
        let mut stmt = self.conn.prepare("SELECT path,index_dir FROM roots ORDER BY path")?;
        let rows = stmt.query_map([], |r| {
            Ok(CatalogRoot { path: r.get(0)?, index_dir: PathBuf::from(r.get::<_, String>(1)?) })
        })?;
        rows.collect()
    }

    /// Replaces the catalogued files of `root` with what its existing index holds now. Content
    /// hashes come from the root's `semantic.sqlite` when it exists and has them; otherwise `NULL`.
    /// Returns the number of files catalogued.
    pub fn sync_root(&mut self, root: &str) -> Result<usize, String> {
        let index_dir: String = self
            .conn
            .query_row("SELECT index_dir FROM roots WHERE path=?1", [root], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("root is not registered: {root}"))?;
        let index_dir = PathBuf::from(index_dir);
        let engine = native_index::open_or_create_with_rebuild(&index_dir).map_err(|e| e.to_string())?;
        let docs = engine.all_document_metadata().map_err(|e| e.to_string())?;
        let hashes = semantic_hashes(&index_dir.join("semantic.sqlite"));

        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        let root_id: i64 = tx
            .query_row("SELECT id FROM roots WHERE path=?1", [root], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        // Upsert so unchanged files keep their stored entities; a changed file loses them (stale evidence).
        tx.execute(
            "CREATE TEMP TABLE IF NOT EXISTS seen(path TEXT PRIMARY KEY)",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM seen", []).map_err(|e| e.to_string())?;
        {
            let mut mark = tx.prepare("INSERT INTO seen(path) VALUES(?1)").map_err(|e| e.to_string())?;
            let mut upsert = tx
                .prepare(
                    "INSERT INTO files(root_id,path,size,modified,content_hash) VALUES(?1,?2,?3,?4,?5)
                     ON CONFLICT(root_id,path) DO UPDATE SET
                         entities_stamp=CASE WHEN size=excluded.size AND modified=excluded.modified
                                             THEN entities_stamp ELSE NULL END,
                         size=excluded.size, modified=excluded.modified, content_hash=excluded.content_hash",
                )
                .map_err(|e| e.to_string())?;
            for (path, (modified, size)) in &docs {
                mark.execute([path]).map_err(|e| e.to_string())?;
                upsert
                    .execute(params![root_id, path, size, modified, hashes.get(path)])
                    .map_err(|e| e.to_string())?;
            }
        }
        tx.execute(
            "DELETE FROM files WHERE root_id=?1 AND path NOT IN (SELECT path FROM seen)",
            [root_id],
        )
        .map_err(|e| e.to_string())?;
        // A changed file (stamp cleared) must not keep entities from its old content.
        tx.execute(
            "DELETE FROM entities WHERE root_id=?1 AND path IN
               (SELECT path FROM files WHERE root_id=?1 AND entities_stamp IS NULL)",
            [root_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(docs.len())
    }

    /// Every catalogued file, ordered by root then path.
    pub fn files(&self) -> rusqlite::Result<Vec<CatalogFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.path,f.path,f.size,f.modified,f.content_hash
             FROM files f JOIN roots r ON r.id=f.root_id ORDER BY r.path,f.path",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(CatalogFile {
                root: r.get(0)?,
                path: r.get(1)?,
                size: r.get(2)?,
                modified_unix: r.get(3)?,
                content_hash: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Groups of files (across roots) with identical non-null content hash: copies and renames.
    pub fn duplicate_groups(&self) -> rusqlite::Result<Vec<Vec<CatalogFile>>> {
        let mut by_hash: std::collections::BTreeMap<Vec<u8>, Vec<CatalogFile>> = Default::default();
        for file in self.files()? {
            if let Some(hash) = file.content_hash.clone() {
                by_hash.entry(hash).or_default().push(file);
            }
        }
        Ok(by_hash.into_values().filter(|group| group.len() > 1).collect())
    }

    /// Searches the fast index of every registered root and unions the hits, best score first
    /// (ties by path). Each hit is exactly what `native_index::search` returns for that root; a root
    /// whose index cannot be opened is skipped and reported in the second element.
    pub fn search_all(&self, query: &str, limit_per_root: usize) -> rusqlite::Result<(Vec<SearchHit>, Vec<String>)> {
        let mut hits = Vec::new();
        let mut skipped = Vec::new();
        for root in self.roots()? {
            let result = NativeSearchEngine::open_or_create(&root.index_dir)
                .and_then(|engine| native_index::search(&engine, query, limit_per_root));
            match result {
                Ok(found) => hits.extend(found),
                Err(error) => skipped.push(format!("{}: {error}", root.path)),
            }
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
        Ok((hits, skipped))
    }
}

fn is_sound(path: &Path) -> bool {
    Connection::open(path)
        .and_then(|conn| conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0)))
        .map(|verdict| verdict == "ok")
        .unwrap_or(false)
}

/// `path -> content_hash` from a root's semantic index; empty when it is missing or unreadable.
fn semantic_hashes(path: &Path) -> std::collections::HashMap<String, Vec<u8>> {
    let mut out = std::collections::HashMap::new();
    if !path.exists() {
        return out;
    }
    let Ok(conn) = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return out;
    };
    let Ok(mut stmt) = conn.prepare("SELECT path,content_hash FROM semantic_documents WHERE content_hash IS NOT NULL")
    else {
        return out;
    };
    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))) {
        out.extend(rows.flatten());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use native_search::engine::DocumentInput;

    fn index(dir: &Path, docs: &[(&str, &str)]) {
        native_index::ensure_index_directory_exists(dir).unwrap();
        let engine = native_index::open_or_create_with_rebuild(dir).unwrap();
        for (path, body) in docs {
            engine
                .index_document(DocumentInput {
                    id: path,
                    path,
                    filename: Path::new(path).file_name().unwrap().to_str().unwrap(),
                    extension: ".txt",
                    title: "",
                    modified_unix: 7,
                    created_unix: 7,
                    size: body.len() as i64,
                    body,
                })
                .unwrap();
        }
        engine.commit().unwrap();
    }

    fn paths(hits: &[SearchHit]) -> Vec<String> {
        let mut p: Vec<String> = hits.iter().map(|h| h.path.clone()).collect();
        p.sort();
        p
    }

    #[test]
    fn cross_root_search_equals_union_of_per_root_results_and_sync_lists_files() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a-index"), tmp.path().join("b-index"));
        index(&a, &[("/a/one.txt", "torque wrench spec"), ("/a/two.txt", "unrelated text")]);
        index(&b, &[("/b/three.txt", "torque limit table"), ("/b/four.txt", "other words")]);

        let mut catalog = Catalog::open(&tmp.path().join("catalog.sqlite")).unwrap();
        catalog.register_root("/a", &a).unwrap();
        catalog.register_root("/b", &b).unwrap();

        let per_root: Vec<String> = [&a, &b]
            .iter()
            .flat_map(|dir| {
                let engine = NativeSearchEngine::open_or_create(dir).unwrap();
                native_index::search(&engine, "torque", 50).unwrap()
            })
            .map(|h| h.path)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let (all, skipped) = catalog.search_all("torque", 50).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(paths(&all), per_root);
        assert_eq!(per_root, vec!["/a/one.txt", "/b/three.txt"]);
        assert!(all.windows(2).all(|w| w[0].score >= w[1].score));

        assert_eq!(catalog.sync_root("/a").unwrap(), 2);
        assert_eq!(catalog.sync_root("/b").unwrap(), 2);
        let files = catalog.files().unwrap();
        assert_eq!(files.len(), 4);
        assert!(files.iter().any(|f| f.root == "/a" && f.path == "/a/one.txt" && f.size == 18 && f.modified_unix == 7));

        // Re-sync replaces, not appends; removing a root drops its files only.
        assert_eq!(catalog.sync_root("/a").unwrap(), 2);
        assert_eq!(catalog.files().unwrap().len(), 4);
        assert!(catalog.remove_root("/a").unwrap());
        assert_eq!(catalog.files().unwrap().len(), 2);
        assert!(catalog.sync_root("/a").is_err());
    }

    #[test]
    fn unopenable_root_is_reported_and_others_still_answer_and_damaged_catalog_is_recreated() {
        let tmp = tempfile::tempdir().unwrap();
        let good = tmp.path().join("good");
        index(&good, &[("/g/x.txt", "alpha beta")]);
        let blocked = tmp.path().join("blocked");
        std::fs::write(&blocked, b"a file, not a directory").unwrap();

        let catalog = Catalog::open_in_memory().unwrap();
        catalog.register_root("/g", &good).unwrap();
        catalog.register_root("/bad", &blocked).unwrap();
        let (hits, skipped) = catalog.search_all("alpha", 10).unwrap();
        assert_eq!(paths(&hits), vec!["/g/x.txt"]);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].starts_with("/bad"));

        let db = tmp.path().join("catalog.sqlite");
        std::fs::write(&db, b"not a database at all, just bytes").unwrap();
        let reopened = Catalog::open(&db).unwrap();
        assert!(reopened.roots().unwrap().is_empty());
    }

    #[test]
    fn duplicate_groups_pair_files_with_equal_hash_across_roots() {
        let catalog = Catalog::open_in_memory().unwrap();
        let r1 = catalog.register_root("/r1", Path::new("/i1")).unwrap();
        let r2 = catalog.register_root("/r2", Path::new("/i2")).unwrap();
        for (root, path, hash) in [(r1, "/r1/a", Some(vec![1u8])), (r2, "/r2/b", Some(vec![1u8])), (r2, "/r2/c", Some(vec![2u8])), (r1, "/r1/d", None)] {
            catalog
                .conn
                .execute("INSERT INTO files(root_id,path,size,modified,content_hash) VALUES(?1,?2,1,1,?3)", params![root, path, hash])
                .unwrap();
        }
        let groups = catalog.duplicate_groups().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec!["/r1/a", "/r2/b"]);
    }
}
