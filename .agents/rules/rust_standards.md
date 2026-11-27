# Rust Engineering Rules & Verification Standards
**Scope:** Applied to all files under `daemon-rust/`

## 1. Ownership & Lifetime Discipline
- Avoid unnecessary `.clone()` calls; use borrowing (`&T`, `&mut T`) and reference lifetimes where practical.
- Keep struct fields private by default; expose accessors only when needed.
- Enforce clean error handling via `Result<T, E>` and custom error types implementing `std::error::Error`. Never use bare `.unwrap()` or `.expect()` in non-test production code.

## 2. Async Tokio Runtime Constraints
- **ZERO Blocking I/O on Workers:** Never execute synchronous file I/O, `rusqlite` transactions, or Win32 COM invocations directly on Tokio worker threads.
- Isolate SQLite operations behind dedicated actor threads (`jarvis-db-actor`) with bounded `mpsc` channels (`capacity <= 32`).
- Never hold `std::sync::Mutex` across `.await` points; use `tokio::sync::Mutex` or restructure to release the lock before awaiting.

## 3. SQLite Durability
- Always execute `PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;` on initialization.
- Perform clean checkpoints (`PRAGMA wal_checkpoint(TRUNCATE)`) before process termination.

## 4. Verification Gates
Before proposing code completion, the following commands must execute cleanly:
1. `cargo check -p jarvis-daemon` (Exit code 0)
2. `cargo clippy -p jarvis-daemon --all-targets -- -D warnings` (Zero warnings)
3. `cargo test -p jarvis-daemon` (100% tests passing)
