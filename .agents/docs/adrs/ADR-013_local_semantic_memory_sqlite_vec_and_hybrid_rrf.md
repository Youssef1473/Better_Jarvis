# ADR-013: Local Semantic Memory via SQLite-Vec & Hybrid RRF
**Status:** Accepted  
**Date:** 2026-11-22  
**Context:** TICKET-13 (Persistent Long-Term Memory & Dual-Engine Retrieval)  
**Governed Subsystems:** `daemon-rust/src/memory/db.rs`, `daemon-rust/src/memory/embedder.rs`

---

## 1. Context & Problem Statement
A personal desktop assistant requires persistent, durable memory of user preferences, instructions, and past interactions. Pure full-text search (BM25) fails when the user asks conceptual or paraphrased questions without exact keyword overlap. Conversely, pure dense vector search frequently suffers from semantic drift, hallucinating conceptually similar but factually irrelevant documents when exact identifier matches (such as file paths, error codes, or function names) are queried.

## 2. Decision Drivers
- High precision on exact keywords, identifiers, and error strings.
- High recall on conceptual paraphrases and semantic intent.
- Zero external vector database daemon (no Qdrant, Pinecone, or Chroma processes).
- Local sub-15ms embedding generation and search latency.

## 3. Decision
1. **In-Process SQLite Vector Engine:** Utilize `sqlite-vec` integrated directly into the primary SQLite database under WAL mode (`PRAGMA journal_mode=WAL; synchronous=NORMAL;`).
2. **Actor Thread Isolation:** All embedding generation and SQLite read/write operations execute on a dedicated single-writer actor thread (`jarvis-db-actor`) communicating over bounded `tokio::sync::mpsc` channels, preventing Tokio worker starvation.
3. **Local ONNX Embedder:** Embed text via `all-MiniLM-L6-v2-q8.onnx` producing 384-dimensional dense vectors. Warm latency is bounded to < 12 ms through pre-warmed memory arenas and intra-op thread bounding. All vectors are L2 unit-normalized.
4. **Hybrid Reciprocal Rank Fusion (RRF):** Combine keyword search and vector similarity using reciprocal rank fusion with constant $k = 60$:
   $$\text{Score}(d) = \frac{1}{60 + \text{Rank}_{\text{BM25}}(d)} + \frac{1}{60 + \text{Rank}_{\text{Vec}}(d)}$$

## 4. Consequences
- **Positive:** Unifies structured storage, FTS5 full-text indexing, and dense vector KNN inside a single `.db` file with atomic transactions.
- **Trade-off:** Ingesting new memories requires running local ONNX inference, adding ~10ms per memory record.

## 5. Verification Evidence
- `daemon-rust/src/memory/embedder.rs` test `test_local_embedder_real_onnx_semantics_and_latency` confirms cosine similarity of synonymous texts > 0.75, unrelated < 0.20, and warm latency < 12 ms.
- `daemon-rust/src/memory/db.rs` test `test_hybrid_rrf_ranking` proves RRF correctly promotes records matching both vector and keyword criteria.
