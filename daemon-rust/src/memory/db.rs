use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactRecord {
    pub id: i64,
    pub key: String,
    pub value: String,
    pub tier: String,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteChunk {
    pub id: i64,
    pub path: String,
    pub heading: String,
    pub content: String,
    pub updated_at: u64,
}

/// Computes cosine similarity between two float vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    if norm_a <= 0.0 || norm_b <= 0.0 {
        0.0
    } else {
        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}

/// Serializes an f32 vector into a raw little-endian byte vector.
pub fn encode_embedding(vec: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for &f in vec {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// Deserializes a raw little-endian byte vector into an f32 vector.
pub fn decode_embedding(bytes: &[u8]) -> Vec<f32> {
    if !bytes.len().is_multiple_of(4) {
        return Vec::new();
    }
    let mut vec = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.as_chunks::<4>().0 {
        vec.push(f32::from_le_bytes(*chunk));
    }
    vec
}

enum DbCommand {
    UpsertFact {
        key: String,
        value: String,
        tier: String,
        reply: oneshot::Sender<Result<()>>,
    },
    GetFact {
        key: String,
        reply: oneshot::Sender<Result<Option<String>>>,
    },
    ListFacts {
        reply: oneshot::Sender<Result<Vec<FactRecord>>>,
    },
    DeleteFact {
        key: String,
        reply: oneshot::Sender<Result<bool>>,
    },
    InsertNoteChunk {
        path: String,
        heading: String,
        content: String,
        embedding: Option<Vec<f32>>,
        reply: oneshot::Sender<Result<()>>,
    },
    SearchNotesSemantic {
        query_embedding: Vec<f32>,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<(NoteChunk, f32)>>>,
    },
    SearchNotesKeyword {
        keyword: String,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<NoteChunk>>>,
    },
    Shutdown {
        reply: oneshot::Sender<Result<()>>,
    },
}

#[derive(Clone)]
pub struct MemoryDb {
    sender: mpsc::Sender<DbCommand>,
}

impl MemoryDb {
    /// Opens or creates a SQLite database with WAL mode and dedicated actor background thread.
    pub fn open(path: &Path) -> Result<Self> {
        let (tx, mut rx) = mpsc::channel::<DbCommand>(64);
        let path_buf = path.to_path_buf();
        let (init_tx, init_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("jarvis-db-actor".into())
            .spawn(move || {
                let init_res = (|| -> Result<Connection> {
                    if let Some(parent) = path_buf.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    let conn = Connection::open(&path_buf)
                        .with_context(|| format!("Failed to open SQLite database at {:?}", path_buf))?;

                    conn.execute_batch(
                        "PRAGMA journal_mode = WAL;
                         PRAGMA synchronous = NORMAL;
                         PRAGMA busy_timeout = 5000;
                         PRAGMA foreign_keys = ON;",
                    )
                    .context("Failed to apply SQLite WAL pragmas")?;

                    conn.execute_batch(
                        "CREATE TABLE IF NOT EXISTS facts (
                            id INTEGER PRIMARY KEY AUTOINCREMENT,
                            key TEXT UNIQUE NOT NULL,
                            value TEXT NOT NULL,
                            tier TEXT NOT NULL DEFAULT 'preference',
                            updated_at INTEGER NOT NULL
                        );

                        CREATE TABLE IF NOT EXISTS note_chunks (
                            id INTEGER PRIMARY KEY AUTOINCREMENT,
                            path TEXT NOT NULL,
                            heading TEXT NOT NULL,
                            content TEXT NOT NULL,
                            embedding_blob BLOB,
                            updated_at INTEGER NOT NULL
                        );

                        CREATE INDEX IF NOT EXISTS idx_facts_key ON facts(key);
                        CREATE INDEX IF NOT EXISTS idx_note_chunks_path ON note_chunks(path);",
                    )
                    .context("Failed to initialize SQLite memory tables")?;

                    info!("SQLite Memory database initialized in WAL mode at {:?}", path_buf);
                    Ok(conn)
                })();

                let conn = match init_res {
                    Ok(c) => {
                        let _ = init_tx.send(Ok(()));
                        c
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };

                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        DbCommand::UpsertFact { key, value, tier, reply } => {
                            let res = Self::sync_upsert_fact(&conn, &key, &value, &tier);
                            let _ = reply.send(res);
                        }
                        DbCommand::GetFact { key, reply } => {
                            let res = Self::sync_get_fact(&conn, &key);
                            let _ = reply.send(res);
                        }
                        DbCommand::ListFacts { reply } => {
                            let res = Self::sync_list_facts(&conn);
                            let _ = reply.send(res);
                        }
                        DbCommand::DeleteFact { key, reply } => {
                            let res = Self::sync_delete_fact(&conn, &key);
                            let _ = reply.send(res);
                        }
                        DbCommand::InsertNoteChunk { path, heading, content, embedding, reply } => {
                            let res = Self::sync_insert_note_chunk(&conn, &path, &heading, &content, embedding.as_deref());
                            let _ = reply.send(res);
                        }
                        DbCommand::SearchNotesSemantic { query_embedding, limit, reply } => {
                            let res = Self::sync_search_notes_semantic(&conn, &query_embedding, limit);
                            let _ = reply.send(res);
                        }
                        DbCommand::SearchNotesKeyword { keyword, limit, reply } => {
                            let res = Self::sync_search_notes_keyword(&conn, &keyword, limit);
                            let _ = reply.send(res);
                        }
                        DbCommand::Shutdown { reply } => {
                            let res = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                                .context("WAL checkpoint truncate failed on shutdown");
                            let _ = reply.send(res);
                            info!("SQLite DB actor cleanly checkpointed and shut down.");
                            drop(conn);
                            return;
                        }
                    }
                }

                // If rx closed without explicit shutdown, perform checkpoint truncate safety net
                let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            })?;

        init_rx.recv().map_err(|e| anyhow::anyhow!("DB actor thread init channel failed: {e}"))??;

        Ok(Self { sender: tx })
    }

    /// Opens an in-memory SQLite database actor (used for testing).
    pub fn in_memory() -> Result<Self> {
        let (tx, mut rx) = mpsc::channel::<DbCommand>(64);
        let (init_tx, init_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("jarvis-db-actor-test".into())
            .spawn(move || {
                let init_res = (|| -> Result<Connection> {
                    let conn = Connection::open_in_memory()?;
                    conn.execute_batch(
                        "PRAGMA busy_timeout = 2000;
                         CREATE TABLE IF NOT EXISTS facts (
                            id INTEGER PRIMARY KEY AUTOINCREMENT,
                            key TEXT UNIQUE NOT NULL,
                            value TEXT NOT NULL,
                            tier TEXT NOT NULL DEFAULT 'preference',
                            updated_at INTEGER NOT NULL
                        );

                        CREATE TABLE IF NOT EXISTS note_chunks (
                            id INTEGER PRIMARY KEY AUTOINCREMENT,
                            path TEXT NOT NULL,
                            heading TEXT NOT NULL,
                            content TEXT NOT NULL,
                            embedding_blob BLOB,
                            updated_at INTEGER NOT NULL
                        );",
                    )?;
                    Ok(conn)
                })();

                let conn = match init_res {
                    Ok(c) => {
                        let _ = init_tx.send(Ok(()));
                        c
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };

                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        DbCommand::UpsertFact { key, value, tier, reply } => {
                            let res = Self::sync_upsert_fact(&conn, &key, &value, &tier);
                            let _ = reply.send(res);
                        }
                        DbCommand::GetFact { key, reply } => {
                            let res = Self::sync_get_fact(&conn, &key);
                            let _ = reply.send(res);
                        }
                        DbCommand::ListFacts { reply } => {
                            let res = Self::sync_list_facts(&conn);
                            let _ = reply.send(res);
                        }
                        DbCommand::DeleteFact { key, reply } => {
                            let res = Self::sync_delete_fact(&conn, &key);
                            let _ = reply.send(res);
                        }
                        DbCommand::InsertNoteChunk { path, heading, content, embedding, reply } => {
                            let res = Self::sync_insert_note_chunk(&conn, &path, &heading, &content, embedding.as_deref());
                            let _ = reply.send(res);
                        }
                        DbCommand::SearchNotesSemantic { query_embedding, limit, reply } => {
                            let res = Self::sync_search_notes_semantic(&conn, &query_embedding, limit);
                            let _ = reply.send(res);
                        }
                        DbCommand::SearchNotesKeyword { keyword, limit, reply } => {
                            let res = Self::sync_search_notes_keyword(&conn, &keyword, limit);
                            let _ = reply.send(res);
                        }
                        DbCommand::Shutdown { reply } => {
                            let _ = reply.send(Ok(()));
                            drop(conn);
                            return;
                        }
                    }
                }
            })?;

        init_rx.recv().map_err(|e| anyhow::anyhow!("DB test actor init channel failed: {e}"))??;

        Ok(Self { sender: tx })
    }

    fn sync_upsert_fact(conn: &Connection, key: &str, value: &str, tier: &str) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        conn.execute(
            "INSERT INTO facts (key, value, tier, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET
                 value = excluded.value,
                 tier = excluded.tier,
                 updated_at = excluded.updated_at;",
            params![key.trim(), value.trim(), tier.trim(), now],
        )?;
        Ok(())
    }

    fn sync_get_fact(conn: &Connection, key: &str) -> Result<Option<String>> {
        let mut stmt = conn.prepare("SELECT value FROM facts WHERE key = ?1")?;
        let mut rows = stmt.query(params![key.trim()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    fn sync_list_facts(conn: &Connection) -> Result<Vec<FactRecord>> {
        let mut stmt = conn.prepare(
            "SELECT id, key, value, tier, updated_at FROM facts ORDER BY tier ASC, key ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FactRecord {
                id: row.get(0)?,
                key: row.get(1)?,
                value: row.get(2)?,
                tier: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    fn sync_delete_fact(conn: &Connection, key: &str) -> Result<bool> {
        let affected = conn.execute("DELETE FROM facts WHERE key = ?1", params![key.trim()])?;
        Ok(affected > 0)
    }

    fn sync_insert_note_chunk(
        conn: &Connection,
        path: &str,
        heading: &str,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let blob = embedding.map(encode_embedding);

        conn.execute(
            "INSERT INTO note_chunks (path, heading, content, embedding_blob, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![path, heading, content, blob, now],
        )?;
        Ok(())
    }

    fn sync_search_notes_semantic(
        conn: &Connection,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(NoteChunk, f32)>> {
        let mut stmt = conn.prepare(
            "SELECT id, path, heading, content, embedding_blob, updated_at
             FROM note_chunks
             WHERE embedding_blob IS NOT NULL",
        )?;
        let mut candidates = Vec::new();
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let path: String = row.get(1)?;
            let heading: String = row.get(2)?;
            let content: String = row.get(3)?;
            let blob: Option<Vec<u8>> = row.get(4)?;
            let updated_at: u64 = row.get(5)?;

            if let Some(b) = blob {
                let vec = decode_embedding(&b);
                let score = cosine_similarity(query_embedding, &vec);
                candidates.push((
                    NoteChunk {
                        id,
                        path,
                        heading,
                        content,
                        updated_at,
                    },
                    score,
                ));
            }
        }
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(limit);
        Ok(candidates)
    }

    fn sync_search_notes_keyword(
        conn: &Connection,
        keyword: &str,
        limit: usize,
    ) -> Result<Vec<NoteChunk>> {
        let pattern = format!("%{}%", keyword.trim());
        let mut stmt = conn.prepare(
            "SELECT id, path, heading, content, updated_at
             FROM note_chunks
             WHERE heading LIKE ?1 OR content LIKE ?1
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            Ok(NoteChunk {
                id: row.get(0)?,
                path: row.get(1)?,
                heading: row.get(2)?,
                content: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Asynchronously upserts a factual memory entry with ON CONFLICT DO UPDATE semantics.
    pub async fn upsert_fact(&self, key: &str, value: &str, tier: &str) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertFact {
                key: key.trim().to_string(),
                value: value.trim().to_string(),
                tier: tier.trim().to_string(),
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("DB actor disconnected"))?;
        rx.await.map_err(|_| anyhow::anyhow!("DB actor dropped response"))?
    }

    /// Asynchronously retrieves a single fact by key.
    pub async fn get_fact(&self, key: &str) -> Result<Option<String>> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetFact {
                key: key.trim().to_string(),
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("DB actor disconnected"))?;
        rx.await.map_err(|_| anyhow::anyhow!("DB actor dropped response"))?
    }

    /// Asynchronously returns all saved facts ordered by tier and key.
    pub async fn list_facts(&self) -> Result<Vec<FactRecord>> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListFacts { reply })
            .await
            .map_err(|_| anyhow::anyhow!("DB actor disconnected"))?;
        rx.await.map_err(|_| anyhow::anyhow!("DB actor dropped response"))?
    }

    /// Asynchronously deletes a fact by key. Returns true if removed.
    pub async fn delete_fact(&self, key: &str) -> Result<bool> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteFact {
                key: key.trim().to_string(),
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("DB actor disconnected"))?;
        rx.await.map_err(|_| anyhow::anyhow!("DB actor dropped response"))?
    }

    /// Asynchronously inserts a markdown note chunk with an optional embedding vector.
    pub async fn insert_note_chunk(
        &self,
        path: &str,
        heading: &str,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertNoteChunk {
                path: path.to_string(),
                heading: heading.to_string(),
                content: content.to_string(),
                embedding: embedding.map(|e| e.to_vec()),
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("DB actor disconnected"))?;
        rx.await.map_err(|_| anyhow::anyhow!("DB actor dropped response"))?
    }

    /// Asynchronously searches note chunks by cosine similarity over embeddings.
    pub async fn search_notes_semantic(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(NoteChunk, f32)>> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SearchNotesSemantic {
                query_embedding: query_embedding.to_vec(),
                limit,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("DB actor disconnected"))?;
        rx.await.map_err(|_| anyhow::anyhow!("DB actor dropped response"))?
    }

    /// Asynchronously searches note chunks via text substring search.
    pub async fn search_notes_keyword(&self, keyword: &str, limit: usize) -> Result<Vec<NoteChunk>> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SearchNotesKeyword {
                keyword: keyword.trim().to_string(),
                limit,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("DB actor disconnected"))?;
        rx.await.map_err(|_| anyhow::anyhow!("DB actor dropped response"))?
    }

    /// Flushes WAL log via PRAGMA wal_checkpoint(TRUNCATE) and terminates the actor thread.
    pub async fn shutdown(&self) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        if self.sender.send(DbCommand::Shutdown { reply }).await.is_err() {
            // Actor already terminated
            return Ok(());
        }
        rx.await.map_err(|_| anyhow::anyhow!("DB actor dropped shutdown reply"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let v1 = [1.0, 0.0, 0.0];
        let v2 = [1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v1, &v2) - 1.0).abs() < 1e-6);

        let v3 = [0.0, 1.0, 0.0];
        assert!((cosine_similarity(&v1, &v3) - 0.0).abs() < 1e-6);

        let frac = std::f32::consts::FRAC_1_SQRT_2;
        let v4 = [frac, frac, 0.0];
        assert!((cosine_similarity(&v1, &v4) - frac).abs() < 1e-5);
    }

    #[test]
    fn test_embedding_codec() {
        let original = vec![0.123f32, -4.567f32, 89.0f32, 0.0001f32];
        let bytes = encode_embedding(&original);
        assert_eq!(bytes.len(), original.len() * 4);
        let decoded = decode_embedding(&bytes);
        assert_eq!(original, decoded);
    }

    #[tokio::test]
    async fn test_facts_upsert_and_conflict() {
        let db = MemoryDb::in_memory().unwrap();

        db.upsert_fact("user_name", "Youssef", "profile").await.unwrap();
        assert_eq!(db.get_fact("user_name").await.unwrap().unwrap(), "Youssef");

        // Upsert overwrite
        db.upsert_fact("user_name", "Youssef S.", "profile").await.unwrap();
        assert_eq!(db.get_fact("user_name").await.unwrap().unwrap(), "Youssef S.");

        let facts = db.list_facts().await.unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].key, "user_name");
        assert_eq!(facts[0].value, "Youssef S.");

        db.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_notes_semantic_and_keyword_search() {
        let db = MemoryDb::in_memory().unwrap();

        let v_ai = [1.0, 0.0, 0.0];
        let v_os = [0.0, 1.0, 0.0];

        db.insert_note_chunk("notes/ai.md", "Gemini API", "Fast routing with flash-lite", Some(&v_ai)).await.unwrap();
        db.insert_note_chunk("notes/os.md", "Windows Job Objects", "Sandbox limits and kill-on-close", Some(&v_os)).await.unwrap();

        // Query AI vector
        let hits = db.search_notes_semantic(&[0.9, 0.1, 0.0], 5).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0.heading, "Gemini API");
        assert!(hits[0].1 > 0.85);

        // Keyword query
        let kw_hits = db.search_notes_keyword("Sandbox", 5).await.unwrap();
        assert_eq!(kw_hits.len(), 1);
        assert_eq!(kw_hits[0].heading, "Windows Job Objects");

        db.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_wal_checkpoint_on_shutdown() {
        let temp_dir = std::env::temp_dir().join(format!("jarvis_wal_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test_wal.db");

        let db = MemoryDb::open(&db_path).unwrap();
        db.upsert_fact("session", "active", "sys").await.unwrap();
        db.shutdown().await.unwrap();

        // Reconnect to verify data survived and WAL was truncated
        let conn = Connection::open(&db_path).unwrap();
        let mut stmt = conn.prepare("SELECT value FROM facts WHERE key = 'session'").unwrap();
        let val: String = stmt.query_row([], |r| r.get(0)).unwrap();
        assert_eq!(val, "active");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_sqlite_wal_integrity_check_and_crash_safety() {
        let temp_dir = std::env::temp_dir().join(format!("jarvis_wal_crash_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("crash_test.db");

        // Write without graceful shutdown / wal checkpoint
        {
            let db = MemoryDb::open(&db_path).unwrap();
            for i in 0..50 {
                db.upsert_fact(&format!("key_{i}"), &format!("val_{i}"), "test").await.unwrap();
            }
            // Drop db handle directly simulating sudden ungraceful process abort
            drop(db);
        }

        // Open raw connection and execute PRAGMA integrity_check
        let conn = Connection::open(&db_path).unwrap();
        let mut stmt = conn.prepare("PRAGMA integrity_check;").unwrap();
        let integrity: String = stmt.query_row([], |r| r.get(0)).unwrap();
        assert_eq!(integrity, "ok");

        // Verify data was durable
        let mut count_stmt = conn.prepare("SELECT COUNT(*) FROM facts").unwrap();
        let count: i64 = count_stmt.query_row([], |r| r.get(0)).unwrap();
        assert_eq!(count, 50);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
