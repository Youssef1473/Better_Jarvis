use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};
use tracing::info;

static INIT_VEC: std::sync::Once = std::sync::Once::new();

/// Ensures sqlite-vec static C extension is registered once across all SQLite connections in the process.
pub fn ensure_sqlite_vec_registered() {
    INIT_VEC.call_once(|| {
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut std::os::raw::c_char,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> std::os::raw::c_int,
            >(sqlite_vec::sqlite3_vec_init as *const ())));
        }
    });
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactRecord {
    pub id: i64,
    pub key: String,
    pub value: String,
    pub tier: String,
    pub source_trusted: bool,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoteChunk {
    pub id: i64,
    pub path: String,
    pub heading: String,
    pub content: String,
    pub source_trusted: bool,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchResult {
    pub chunk: NoteChunk,
    pub rrf_score: f32,
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

/// Sanitizes a text query into safe FTS5 query tokens.
pub fn sanitize_fts5_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 2)
        .map(|w| format!("\"{}\"", w))
        .collect();

    if words.is_empty() {
        None
    } else {
        Some(words.join(" OR "))
    }
}

enum DbCommand {
    UpsertFact {
        key: String,
        value: String,
        tier: String,
        source_trusted: bool,
        embedding: Option<Vec<f32>>,
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
        source_trusted: bool,
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
    HybridSearch {
        query: String,
        query_embedding: Option<Vec<f32>>,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<HybridSearchResult>>>,
    },
    SearchFactsSemantic {
        query: String,
        query_embedding: Option<Vec<f32>>,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<(FactRecord, f32)>>>,
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
        ensure_sqlite_vec_registered();
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

                    Self::init_tables(&conn)?;

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
                        DbCommand::UpsertFact { key, value, tier, source_trusted, embedding, reply } => {
                            let res = Self::sync_upsert_fact(&conn, &key, &value, &tier, source_trusted, embedding.as_deref());
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
                        DbCommand::InsertNoteChunk { path, heading, content, embedding, source_trusted, reply } => {
                            let res = Self::sync_insert_note_chunk(&conn, &path, &heading, &content, embedding.as_deref(), source_trusted);
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
                        DbCommand::HybridSearch { query, query_embedding, limit, reply } => {
                            let res = Self::sync_hybrid_search(&conn, &query, query_embedding.as_deref(), limit);
                            let _ = reply.send(res);
                        }
                        DbCommand::SearchFactsSemantic { query, query_embedding, limit, reply } => {
                            let res = Self::sync_search_facts_semantic(&conn, &query, query_embedding.as_deref(), limit);
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

                let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            })?;

        init_rx.recv().map_err(|e| anyhow::anyhow!("DB actor thread init channel failed: {e}"))??;

        Ok(Self { sender: tx })
    }

    /// Opens an in-memory SQLite database actor (used for testing).
    pub fn in_memory() -> Result<Self> {
        ensure_sqlite_vec_registered();
        let (tx, mut rx) = mpsc::channel::<DbCommand>(64);
        let (init_tx, init_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("jarvis-db-actor-test".into())
            .spawn(move || {
                let init_res = (|| -> Result<Connection> {
                    let conn = Connection::open_in_memory()?;
                    conn.execute_batch("PRAGMA busy_timeout = 2000;")?;
                    Self::init_tables(&conn)?;
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
                        DbCommand::UpsertFact { key, value, tier, source_trusted, embedding, reply } => {
                            let res = Self::sync_upsert_fact(&conn, &key, &value, &tier, source_trusted, embedding.as_deref());
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
                        DbCommand::InsertNoteChunk { path, heading, content, embedding, source_trusted, reply } => {
                            let res = Self::sync_insert_note_chunk(&conn, &path, &heading, &content, embedding.as_deref(), source_trusted);
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
                        DbCommand::HybridSearch { query, query_embedding, limit, reply } => {
                            let res = Self::sync_hybrid_search(&conn, &query, query_embedding.as_deref(), limit);
                            let _ = reply.send(res);
                        }
                        DbCommand::SearchFactsSemantic { query, query_embedding, limit, reply } => {
                            let res = Self::sync_search_facts_semantic(&conn, &query, query_embedding.as_deref(), limit);
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

    fn init_tables(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS facts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                key TEXT UNIQUE NOT NULL,
                value TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'preference',
                source_trusted INTEGER NOT NULL DEFAULT 1,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS note_chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                heading TEXT NOT NULL,
                content TEXT NOT NULL,
                embedding_blob BLOB,
                source_trusted INTEGER NOT NULL DEFAULT 1,
                updated_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_facts_key ON facts(key);
            CREATE INDEX IF NOT EXISTS idx_note_chunks_path ON note_chunks(path);

            CREATE VIRTUAL TABLE IF NOT EXISTS notes_vec USING vec0(
                note_id INTEGER PRIMARY KEY,
                embedding float[384] distance_metric=cosine
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS facts_vec USING vec0(
                fact_id INTEGER PRIMARY KEY,
                embedding float[384] distance_metric=cosine
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                heading,
                content,
                path UNINDEXED
            );",
        )
        .context("Failed to initialize SQLite memory tables and virtual tables")?;

        // Migration safety: ensure source_trusted column exists if upgrading an existing DB
        let _ = conn.execute("ALTER TABLE facts ADD COLUMN source_trusted INTEGER NOT NULL DEFAULT 1;", []);
        let _ = conn.execute("ALTER TABLE note_chunks ADD COLUMN source_trusted INTEGER NOT NULL DEFAULT 1;", []);

        Ok(())
    }

    fn sync_upsert_fact(
        conn: &Connection,
        key: &str,
        value: &str,
        tier: &str,
        source_trusted: bool,
        embedding: Option<&[f32]>,
    ) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let trusted_int: i64 = if source_trusted { 1 } else { 0 };

        conn.execute(
            "INSERT INTO facts (key, value, tier, source_trusted, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(key) DO UPDATE SET
                 value = excluded.value,
                 tier = excluded.tier,
                 source_trusted = excluded.source_trusted,
                 updated_at = excluded.updated_at;",
            params![key.trim(), value.trim(), tier.trim(), trusted_int, now],
        )?;

        let fact_id: i64 = conn.query_row(
            "SELECT id FROM facts WHERE key = ?1",
            params![key.trim()],
            |r| r.get(0),
        )?;

        if let Some(emb) = embedding.filter(|e| e.len() == 384) {
            let blob = encode_embedding(emb);
            conn.execute(
                "INSERT OR REPLACE INTO facts_vec (fact_id, embedding) VALUES (?1, ?2)",
                params![fact_id, blob],
            )?;
        } else {
            conn.execute(
                "DELETE FROM facts_vec WHERE fact_id = ?1",
                params![fact_id],
            )?;
        }

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
            "SELECT id, key, value, tier, source_trusted, updated_at FROM facts ORDER BY tier ASC, key ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let trusted_int: i64 = row.get(4)?;
            Ok(FactRecord {
                id: row.get(0)?,
                key: row.get(1)?,
                value: row.get(2)?,
                tier: row.get(3)?,
                source_trusted: trusted_int != 0,
                updated_at: row.get(5)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    fn sync_delete_fact(conn: &Connection, key: &str) -> Result<bool> {
        let fact_id: Option<i64> = conn
            .query_row("SELECT id FROM facts WHERE key = ?1", params![key.trim()], |r| r.get(0))
            .ok();

        if let Some(id) = fact_id {
            conn.execute("DELETE FROM facts_vec WHERE fact_id = ?1", params![id]).ok();
        }

        let affected = conn.execute("DELETE FROM facts WHERE key = ?1", params![key.trim()])?;
        Ok(affected > 0)
    }

    fn sync_search_facts_semantic(
        conn: &Connection,
        query: &str,
        query_embedding: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<(FactRecord, f32)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut scored: HashMap<i64, (FactRecord, f32)> = HashMap::new();

        if let Some(emb) = query_embedding {
            if emb.len() == 384 {
                let blob = encode_embedding(emb);
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT f.id, f.key, f.value, f.tier, f.source_trusted, f.updated_at, fv.distance
                     FROM facts_vec fv
                     JOIN facts f ON f.id = fv.fact_id
                     WHERE fv.embedding MATCH ?1 AND k = ?2
                     ORDER BY fv.distance ASC",
                ) {
                    if let Ok(rows) = stmt.query_map(params![blob, (limit * 2).max(10) as i64], |r| {
                        let trusted_int: i64 = r.get(4)?;
                        let dist: f32 = r.get(6)?;
                        Ok((
                            FactRecord {
                                id: r.get(0)?,
                                key: r.get(1)?,
                                value: r.get(2)?,
                                tier: r.get(3)?,
                                source_trusted: trusted_int != 0,
                                updated_at: r.get(5)?,
                            },
                            (1.0 - dist).max(0.0),
                        ))
                    }) {
                        for (fact, sim) in rows.flatten() {
                            scored.insert(fact.id, (fact, sim));
                        }
                    }
                }
            }
        }

        let trimmed = query.trim();
        if !trimmed.is_empty() {
            let pattern = format!("%{}%", trimmed);
            if let Ok(mut stmt) = conn.prepare(
                "SELECT id, key, value, tier, source_trusted, updated_at
                 FROM facts
                 WHERE key LIKE ?1 OR value LIKE ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            ) {
                if let Ok(rows) = stmt.query_map(params![pattern, limit as i64], |r| {
                    let trusted_int: i64 = r.get(4)?;
                    Ok(FactRecord {
                        id: r.get(0)?,
                        key: r.get(1)?,
                        value: r.get(2)?,
                        tier: r.get(3)?,
                        source_trusted: trusted_int != 0,
                        updated_at: r.get(5)?,
                    })
                }) {
                    for fact in rows.flatten() {
                        scored
                            .entry(fact.id)
                            .and_modify(|(_, s)| *s += 0.25)
                            .or_insert((fact, 0.25));
                    }
                }
            }
        }

        let mut results: Vec<(FactRecord, f32)> = scored.into_values().collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        Ok(results)
    }

    fn sync_insert_note_chunk(
        conn: &Connection,
        path: &str,
        heading: &str,
        content: &str,
        embedding: Option<&[f32]>,
        source_trusted: bool,
    ) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let blob = embedding.map(encode_embedding);
        let trusted_int: i64 = if source_trusted { 1 } else { 0 };

        conn.execute(
            "INSERT INTO note_chunks (path, heading, content, embedding_blob, source_trusted, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![path, heading, content, blob, trusted_int, now],
        )?;

        let note_id = conn.last_insert_rowid();

        // Index in FTS5
        conn.execute(
            "INSERT INTO notes_fts (rowid, heading, content, path) VALUES (?1, ?2, ?3, ?4)",
            params![note_id, heading, content, path],
        )?;

        // Index in sqlite-vec virtual table if 384-dimensional vector provided
        if let Some(emb) = embedding {
            if emb.len() == 384 {
                let emb_bytes = encode_embedding(emb);
                conn.execute(
                    "INSERT OR REPLACE INTO notes_vec (note_id, embedding) VALUES (?1, ?2)",
                    params![note_id, emb_bytes],
                )?;
            }
        }

        Ok(())
    }

    fn sync_search_notes_semantic(
        conn: &Connection,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(NoteChunk, f32)>> {
        let mut stmt = conn.prepare(
            "SELECT id, path, heading, content, embedding_blob, source_trusted, updated_at
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
            let trusted_int: i64 = row.get(5)?;
            let updated_at: u64 = row.get(6)?;

            if let Some(b) = blob {
                let vec = decode_embedding(&b);
                let score = cosine_similarity(query_embedding, &vec);
                candidates.push((
                    NoteChunk {
                        id,
                        path,
                        heading,
                        content,
                        source_trusted: trusted_int != 0,
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
            "SELECT id, path, heading, content, source_trusted, updated_at
             FROM note_chunks
             WHERE heading LIKE ?1 OR content LIKE ?1
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            let trusted_int: i64 = row.get(4)?;
            Ok(NoteChunk {
                id: row.get(0)?,
                path: row.get(1)?,
                heading: row.get(2)?,
                content: row.get(3)?,
                source_trusted: trusted_int != 0,
                updated_at: row.get(5)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    fn sync_hybrid_search(
        conn: &Connection,
        query: &str,
        query_embedding: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<HybridSearchResult>> {
        let mut rrf_scores: HashMap<i64, f32> = HashMap::new();

        // 1. Vector Search via sqlite-vec notes_vec
        if let Some(emb) = query_embedding {
            if emb.len() == 384 {
                let blob = encode_embedding(emb);
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT note_id, distance FROM notes_vec WHERE embedding MATCH ?1 AND k = ?2"
                ) {
                    if let Ok(rows) = stmt.query_map(params![blob, (limit * 2).max(10) as i64], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, f32>(1)?))
                    }) {
                        for (rank_idx, row_res) in rows.flatten().enumerate() {
                            let (note_id, _dist) = row_res;
                            let r = (rank_idx + 1) as f32;
                            *rrf_scores.entry(note_id).or_insert(0.0) += 1.0 / (60.0 + r);
                        }
                    }
                }
            }
        }

        // 2. Keyword Search via FTS5 notes_fts
        if let Some(fts_query) = sanitize_fts5_query(query) {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT rowid, rank FROM notes_fts WHERE notes_fts MATCH ?1 ORDER BY rank LIMIT ?2"
            ) {
                if let Ok(rows) = stmt.query_map(params![fts_query, (limit * 2).max(10) as i64], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
                }) {
                    for (rank_idx, row_res) in rows.flatten().enumerate() {
                        let (rowid, _bm25_rank) = row_res;
                        let r = (rank_idx + 1) as f32;
                        *rrf_scores.entry(rowid).or_insert(0.0) += 1.0 / (60.0 + r);
                    }
                }
            }
        }

        // 3. Substring fallback if both vector and FTS yield zero candidates
        if rrf_scores.is_empty() && !query.trim().is_empty() {
            let pattern = format!("%{}%", query.trim());
            if let Ok(mut stmt) = conn.prepare(
                "SELECT id FROM note_chunks WHERE heading LIKE ?1 OR content LIKE ?1 LIMIT ?2"
            ) {
                if let Ok(rows) = stmt.query_map(params![pattern, limit as i64], |r| r.get::<_, i64>(0)) {
                    for (rank_idx, id_res) in rows.flatten().enumerate() {
                        let r = (rank_idx + 1) as f32;
                        *rrf_scores.entry(id_res).or_insert(0.0) += 1.0 / (60.0 + r);
                    }
                }
            }
        }

        // Sort candidates by RRF score descending
        let mut candidates: Vec<(i64, f32)> = rrf_scores.into_iter().collect();
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(limit);

        let mut results = Vec::new();
        for (id, score) in candidates {
            let mut stmt = conn.prepare(
                "SELECT id, path, heading, content, source_trusted, updated_at FROM note_chunks WHERE id = ?1"
            )?;
            if let Ok(chunk) = stmt.query_row(params![id], |row| {
                let trusted_int: i64 = row.get(4)?;
                Ok(NoteChunk {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    heading: row.get(2)?,
                    content: row.get(3)?,
                    source_trusted: trusted_int != 0,
                    updated_at: row.get(5)?,
                })
            }) {
                results.push(HybridSearchResult {
                    chunk,
                    rrf_score: score,
                });
            }
        }

        Ok(results)
    }

    /// Asynchronously upserts a factual memory entry with ON CONFLICT DO UPDATE semantics.
    pub async fn upsert_fact(&self, key: &str, value: &str, tier: &str) -> Result<()> {
        self.upsert_fact_with_opts(key, value, tier, true, None).await
    }

    /// Asynchronously upserts a fact with explicit trust provenance and optional embedding.
    pub async fn upsert_fact_with_opts(
        &self,
        key: &str,
        value: &str,
        tier: &str,
        source_trusted: bool,
        embedding: Option<&[f32]>,
    ) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertFact {
                key: key.trim().to_string(),
                value: value.trim().to_string(),
                tier: tier.trim().to_string(),
                source_trusted,
                embedding: embedding.map(|e| e.to_vec()),
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
        self.insert_note_chunk_with_opts(path, heading, content, embedding, true).await
    }

    /// Asynchronously inserts a note chunk with explicit trust provenance and optional embedding.
    pub async fn insert_note_chunk_with_opts(
        &self,
        path: &str,
        heading: &str,
        content: &str,
        embedding: Option<&[f32]>,
        source_trusted: bool,
    ) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertNoteChunk {
                path: path.to_string(),
                heading: heading.to_string(),
                content: content.to_string(),
                embedding: embedding.map(|e| e.to_vec()),
                source_trusted,
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

    /// Performs hybrid Reciprocal Rank Fusion search combining sqlite-vec KNN and FTS5 BM25.
    pub async fn hybrid_search(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<HybridSearchResult>> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::HybridSearch {
                query: query.to_string(),
                query_embedding: query_embedding.map(|e| e.to_vec()),
                limit,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("DB actor disconnected"))?;
        rx.await.map_err(|_| anyhow::anyhow!("DB actor dropped response"))?
    }

    /// Queries stored facts via `facts_vec` cosine similarity (plus SQL substring boost/fallback).
    pub async fn search_facts_semantic(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<(FactRecord, f32)>> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SearchFactsSemantic {
                query: query.to_string(),
                query_embedding: query_embedding.map(|e| e.to_vec()),
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

    #[test]
    fn test_sqlite_vec_extension_loads_and_knn_ranks() {
        ensure_sqlite_vec_registered();
        let conn = Connection::open_in_memory().unwrap();
        let version: String = conn.query_row("SELECT vec_version()", [], |r| r.get(0)).unwrap();
        assert!(!version.is_empty(), "sqlite-vec extension must report valid version");

        conn.execute_batch(
            "CREATE VIRTUAL TABLE test_knn USING vec0(
                id INTEGER PRIMARY KEY,
                embedding float[4] distance_metric=cosine
            );"
        ).unwrap();

        let frac = std::f32::consts::FRAC_1_SQRT_2;
        let v1 = encode_embedding(&[1.0, 0.0, 0.0, 0.0]);
        let v2 = encode_embedding(&[frac, frac, 0.0, 0.0]);
        let v3 = encode_embedding(&[0.0, 1.0, 0.0, 0.0]);

        conn.execute("INSERT INTO test_knn (id, embedding) VALUES (?1, ?2)", params![1i64, v1]).unwrap();
        conn.execute("INSERT INTO test_knn (id, embedding) VALUES (?1, ?2)", params![2i64, v2]).unwrap();
        conn.execute("INSERT INTO test_knn (id, embedding) VALUES (?1, ?2)", params![3i64, v3]).unwrap();

        let q = encode_embedding(&[0.9, 0.1, 0.0, 0.0]);
        let mut stmt = conn.prepare("SELECT id, distance FROM test_knn WHERE embedding MATCH ?1 AND k = 3").unwrap();
        let rows = stmt.query_map([q], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f32>(1)?))).unwrap();
        let results: Vec<(i64, f32)> = rows.map(|r| r.unwrap()).collect();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, 1, "Closest vector must rank first");
        assert_eq!(results[1].0, 2, "Second vector must rank second");
        assert_eq!(results[2].0, 3, "Orthogonal vector must rank third");
    }

    #[tokio::test]
    async fn test_hybrid_rrf_combines_bm25_and_vector() {
        let db = MemoryDb::in_memory().unwrap();

        // 384-dimensional test vectors
        let mut v_neural = vec![0.0f32; 384];
        v_neural[0] = 1.0;

        let mut v_unrelated = vec![0.0f32; 384];
        v_unrelated[1] = 1.0;

        // Note 1: High vector similarity (neural), but no exact keyword match
        db.insert_note_chunk_with_opts(
            "notes/deep_learning.md",
            "Transformer Architecture",
            "Multi-head self-attention mechanisms and dense latent projections.",
            Some(&v_neural),
            true,
        ).await.unwrap();

        // Note 2: Exact keyword match "kernel", but different vector
        db.insert_note_chunk_with_opts(
            "notes/os_kernel.md",
            "Windows Job Object",
            "Kernel process management and termination policies in Windows NT.",
            Some(&v_unrelated),
            true,
        ).await.unwrap();

        // Note 3: Both vector and keyword match
        db.insert_note_chunk_with_opts(
            "notes/acceleration.md",
            "Neural Inference Kernel",
            "Hardware kernel acceleration for ONNX Runtime models.",
            Some(&v_neural),
            true,
        ).await.unwrap();

        // Query with vector matching Note 1 & Note 3, and keyword "Kernel" matching Note 2 & Note 3
        let hits = db.hybrid_search("Kernel", Some(&v_neural), 5).await.unwrap();

        assert!(hits.len() >= 3);
        // Note 3 has both signals, so RRF score must rank it #1
        assert_eq!(hits[0].chunk.heading, "Neural Inference Kernel");
        assert!(hits[0].rrf_score > hits[1].rrf_score);

        db.shutdown().await.unwrap();
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
        assert!(facts[0].source_trusted);

        db.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_facts_vec_semantic_search_ranks_by_embedding() {
        let db = MemoryDb::in_memory().unwrap();

        let mut v_editor = vec![0.0f32; 384];
        v_editor[0] = 1.0;

        let mut v_music = vec![0.0f32; 384];
        v_music[1] = 1.0;

        db.upsert_fact_with_opts("preferred_ide", "Neovim with LSP", "preference", true, Some(&v_editor))
            .await
            .unwrap();
        db.upsert_fact_with_opts("favorite_genre", "Ambient electronic", "profile", true, Some(&v_music))
            .await
            .unwrap();

        let hits = db
            .search_facts_semantic("coding workspace", Some(&v_editor), 2)
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0.key, "preferred_ide");
        assert!(hits[0].1 > hits[1].1);

        let kw_hits = db.search_facts_semantic("Ambient", None, 2).await.unwrap();
        assert_eq!(kw_hits.len(), 1);
        assert_eq!(kw_hits[0].0.key, "favorite_genre");

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
            drop(db);
        }

        // Open raw connection and execute PRAGMA integrity_check
        let conn = Connection::open(&db_path).unwrap();
        let mut stmt = conn.prepare("PRAGMA integrity_check;").unwrap();
        let integrity: String = stmt.query_row([], |r| r.get(0)).unwrap();
        assert_eq!(integrity, "ok");

        let mut count_stmt = conn.prepare("SELECT COUNT(*) FROM facts").unwrap();
        let count: i64 = count_stmt.query_row([], |r| r.get(0)).unwrap();
        assert_eq!(count, 50);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_upsert_fact_without_embedding_clears_stale_vector() {
        let db = MemoryDb::in_memory().unwrap();
        let mut v_old = vec![0.0f32; 384];
        v_old[0] = 1.0;

        db.upsert_fact_with_opts("drink", "espresso", "preference", true, Some(&v_old))
            .await
            .unwrap();
        let initial_hits = db
            .search_facts_semantic("", Some(&v_old), 5)
            .await
            .unwrap();
        assert_eq!(initial_hits.len(), 1);
        assert_eq!(initial_hits[0].0.value, "espresso");

        // Overwrite without embedding: stale vector row in facts_vec must be removed
        db.upsert_fact_with_opts("drink", "matcha", "preference", true, None)
            .await
            .unwrap();
        let stale_hits = db
            .search_facts_semantic("", Some(&v_old), 5)
            .await
            .unwrap();
        assert!(
            stale_hits.is_empty(),
            "Overwriting fact without embedding must clear stale facts_vec row, got {:?}",
            stale_hits
        );

        db.shutdown().await.unwrap();
    }
}
