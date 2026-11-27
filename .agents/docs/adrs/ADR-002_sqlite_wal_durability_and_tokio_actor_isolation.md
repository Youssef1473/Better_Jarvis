# ADR-002: SQLite WAL Durability & Dedicated Actor Thread Isolation
**Status:** Accepted  
**Date:** 2026-11-20  
**Context:** TICKET-02 (Hardening & Production Execution Plan)  
**Governed Subsystems:** `daemon-rust/src/memory/db.rs`

---

## 1. Context & Problem Statement
Running synchronous SQLite (`rusqlite`) calls directly inside Tokio async worker threads caused worker starvation, latency spikes during voice turns, and risk of database corruption if `jarvis-host.exe` terminated the Job Object abruptly.

## 2. Decision
1. **Durability Pragmas:** Initialize every SQLite connection with:
   ```sql
   PRAGMA journal_mode = WAL;
   PRAGMA synchronous = NORMAL;
   PRAGMA foreign_keys = ON;
   ```
2. **Actor Thread Isolation:** Move all database access to a dedicated background OS thread (`jarvis-db-actor`). Tokio workers interact exclusively through bounded `tokio::sync::mpsc` channels with fixed capacity (32 messages).
3. **Graceful Checkpointing:** Implement an explicit WAL checkpoint (`PRAGMA wal_checkpoint(TRUNCATE)`) on daemon shutdown, ensuring zero database corruption when processes terminate.

## 3. Consequences
- **Positive:** Tokio workers are never blocked by disk I/O; SQLite WAL preserves consistency across hard process kills.
- **Trade-off:** Database queries are asynchronous and require message-passing hops.

## 4. Verification Evidence
- `daemon-rust` unit tests: `memory::db::tests::test_wal_checkpoint_on_shutdown` and `memory::db::tests::test_sqlite_wal_integrity_check_and_crash_safety` pass.
- `scripts/verify-beta.ps1` Stage 5 forced `taskkill /F` confirms intact database with zero corruption.
