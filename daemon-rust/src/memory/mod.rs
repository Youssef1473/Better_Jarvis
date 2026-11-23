pub mod db;
pub mod profile_sync;

pub use db::{encode_embedding, decode_embedding, cosine_similarity, FactRecord, MemoryDb, NoteChunk};
pub use profile_sync::ProfileSyncManager;

use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct MemoryEngine {
    db: MemoryDb,
    sync: ProfileSyncManager,
}

impl MemoryEngine {
    /// Opens the memory engine with persistent storage under the specified database path.
    pub async fn open(db_path: &Path, profile_path: Option<PathBuf>) -> Result<Self> {
        let db = MemoryDb::open(db_path)?;
        let sync = ProfileSyncManager::new(db.clone(), profile_path);

        // If profile.md already exists, seed the database
        if sync.profile_path().exists() {
            let _ = sync.import_profile_if_external().await;
        } else {
            // Otherwise export current DB to create profile.md
            let _ = sync.export_profile_atomic().await;
        }

        Ok(Self { db, sync })
    }

    /// Creates an in-memory memory engine for testing.
    pub async fn in_memory() -> Result<Self> {
        let db = MemoryDb::in_memory()?;
        let sync = ProfileSyncManager::new(db.clone(), None);
        Ok(Self { db, sync })
    }

    pub fn db(&self) -> &MemoryDb {
        &self.db
    }

    pub fn sync(&self) -> &ProfileSyncManager {
        &self.sync
    }

    /// Formats all stored profile facts as a context block for the Gemini agent.
    pub async fn format_context_for_prompt(&self) -> String {
        let facts = match self.db.list_facts().await {
            Ok(f) => f,
            Err(_) => return String::new(),
        };

        if facts.is_empty() {
            return String::new();
        }

        let mut out = String::from("User Profile & System Preferences:\n");
        for fact in facts {
            out.push_str(&format!("- {}: {}\n", fact.key, fact.value));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_format_context_for_prompt() {
        let engine = MemoryEngine::in_memory().await.unwrap();
        engine.db().upsert_fact("name", "Youssef", "profile").await.unwrap();
        engine.db().upsert_fact("shell", "powershell", "env").await.unwrap();

        let ctx = engine.format_context_for_prompt().await;
        assert!(ctx.contains("- name: Youssef"));
        assert!(ctx.contains("- shell: powershell"));

        engine.db().shutdown().await.unwrap();
    }
}
