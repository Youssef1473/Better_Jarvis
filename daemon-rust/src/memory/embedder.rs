use anyhow::{Context, Result};
use ort::{session::Session, value::Tensor};
use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};
use tracing::info;

/// Bounded channel capacity for embedder requests.
pub const EMBEDDER_CHANNEL_CAPACITY: usize = 32;

/// Resolves the absolute path to onnxruntime.dll avoiding System32 collisions.
pub fn resolve_ort_dylib_path() -> Result<PathBuf> {
    // 1. Directory of current_exe()
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let candidate = parent.join("onnxruntime.dll");
            if candidate.exists() && !is_system32(&candidate) {
                return Ok(candidate);
            }
        }
    }

    // 2. JARVIS_ORT_DYLIB_PATH environment variable
    if let Ok(env_path) = std::env::var("JARVIS_ORT_DYLIB_PATH") {
        let candidate = PathBuf::from(env_path);
        if candidate.exists() && !is_system32(&candidate) {
            return Ok(candidate);
        }
    }

    // 3. build/host-cpp/Release/onnxruntime.dll or build/host-cpp/onnxruntime.dll relative to workspace or current dir
    for rel in &[
        "build/host-cpp/Release/onnxruntime.dll",
        "build/host-cpp/onnxruntime.dll",
        "../build/host-cpp/Release/onnxruntime.dll",
        "../build/host-cpp/onnxruntime.dll",
    ] {
        let candidate = PathBuf::from(rel);
        if candidate.exists() && !is_system32(&candidate) {
            return Ok(candidate.canonicalize().unwrap_or(candidate));
        }
    }

    // 4. MSYS64 UCRT64 distribution
    let ucrt_candidate = PathBuf::from(r"C:\msys64\ucrt64\bin\onnxruntime.dll");
    if ucrt_candidate.exists() && !is_system32(&ucrt_candidate) {
        return Ok(ucrt_candidate);
    }

    anyhow::bail!("Failed to locate valid onnxruntime.dll (System32 rejected to avoid API collision)")
}

fn is_system32(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_lowercase();
    lower.contains("system32") || lower.contains("syswow64")
}

/// Discovers the path to `all-MiniLM-L6-v2-q8.onnx` and `all-MiniLM-L6-v2-tokenizer.json`.
pub fn resolve_model_paths(data_dir: Option<&Path>) -> Result<(PathBuf, PathBuf)> {
    let mut candidate_dirs = Vec::new();

    if let Some(d) = data_dir {
        candidate_dirs.push(d.join("models"));
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidate_dirs.push(exe_dir.join("models"));
        }
    }

    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        candidate_dirs.push(PathBuf::from(local_app_data).join("Better_Jarvis").join("models"));
    }

    candidate_dirs.push(PathBuf::from("models"));
    candidate_dirs.push(PathBuf::from("../models"));

    for dir in candidate_dirs {
        let model = dir.join("all-MiniLM-L6-v2-q8.onnx");
        let tokenizer = dir.join("all-MiniLM-L6-v2-tokenizer.json");
        if model.exists() && tokenizer.exists() {
            let model_canon = model.canonicalize().unwrap_or(model);
            let tokenizer_canon = tokenizer.canonicalize().unwrap_or(tokenizer);
            return Ok((model_canon, tokenizer_canon));
        }
    }

    anyhow::bail!("Failed to locate all-MiniLM-L6-v2 model and tokenizer in any search path")
}

enum EmbedderCommand {
    Embed {
        text: String,
        reply: oneshot::Sender<Result<Vec<f32>>>,
    },
    EmbedBatch {
        texts: Vec<String>,
        reply: oneshot::Sender<Result<Vec<Vec<f32>>>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone)]
pub struct LocalEmbedder {
    sender: mpsc::Sender<EmbedderCommand>,
}

impl LocalEmbedder {
    /// Starts the dedicated background embedder actor thread.
    pub fn start(
        model_path: &Path,
        tokenizer_path: &Path,
        ort_dylib_path: Option<&Path>,
    ) -> Result<Self> {
        let dylib = match ort_dylib_path {
            Some(p) => p.to_path_buf(),
            None => resolve_ort_dylib_path()?,
        };

        let env = ort::init_from(dylib.to_str().context("Invalid ORT dylib path")?)
            .context("Failed to load dynamic ONNX Runtime dylib")?
            .build()
            .context("Failed to build ONNX Runtime environment")?;

        let model_path = model_path.to_path_buf();
        let tokenizer_path = tokenizer_path.to_path_buf();

        let (tx, mut rx) = mpsc::channel::<EmbedderCommand>(EMBEDDER_CHANNEL_CAPACITY);
        let (init_tx, init_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("jarvis-embed-actor".into())
            .spawn(move || {
                let init_res = (|| -> Result<(Session, tokenizers::Tokenizer)> {
                    let mut session = Session::builder(&env)?
                        .with_intra_threads(2)
                        .map_err(|e| anyhow::anyhow!("Failed to set intra threads: {e}"))?
                        .with_inter_threads(1)
                        .map_err(|e| anyhow::anyhow!("Failed to set inter threads: {e}"))?
                        .commit_from_file(&model_path)
                        .with_context(|| format!("Failed to load ONNX model at {:?}", model_path))?;
                    let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
                        .map_err(|e| anyhow::anyhow!("Failed to load tokenizer at {:?}: {e}", tokenizer_path))?;

                    // Warmup inference to eliminate cold arena allocation on first turn
                    let _ = Self::sync_embed(&mut session, &tokenizer, "warmup query");

                    Ok((session, tokenizer))
                })();

                let (mut session, tokenizer) = match init_res {
                    Ok(pair) => {
                        let _ = init_tx.send(Ok(()));
                        pair
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };

                info!("LocalEmbedder initialized on dedicated thread jarvis-embed-actor");

                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        EmbedderCommand::Embed { text, reply } => {
                            let res = Self::sync_embed(&mut session, &tokenizer, &text);
                            let _ = reply.send(res);
                        }
                        EmbedderCommand::EmbedBatch { texts, reply } => {
                            let mut results = Vec::with_capacity(texts.len());
                            let mut err = None;
                            for t in texts {
                                match Self::sync_embed(&mut session, &tokenizer, &t) {
                                    Ok(v) => results.push(v),
                                    Err(e) => {
                                        err = Some(e);
                                        break;
                                    }
                                }
                            }
                            if let Some(e) = err {
                                let _ = reply.send(Err(e));
                            } else {
                                let _ = reply.send(Ok(results));
                            }
                        }
                        EmbedderCommand::Shutdown { reply } => {
                            let _ = reply.send(());
                            info!("LocalEmbedder actor cleanly stopped.");
                            return;
                        }
                    }
                }
            })
            .context("Failed to spawn jarvis-embed-actor thread")?;

        init_rx
            .recv()
            .map_err(|e| anyhow::anyhow!("Embedder actor init channel failed: {e}"))??;

        Ok(Self { sender: tx })
    }

    fn sync_embed(
        session: &mut Session,
        tokenizer: &tokenizers::Tokenizer,
        text: &str,
    ) -> Result<Vec<f32>> {
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {e}"))?;

        let ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
        let mask: Vec<i64> = encoding.get_attention_mask().iter().map(|&x| x as i64).collect();
        let type_ids: Vec<i64> = encoding.get_type_ids().iter().map(|&x| x as i64).collect();
        let seq_len = ids.len();

        if seq_len == 0 {
            return Ok(vec![0.0f32; 384]);
        }

        let in_ids = Tensor::<i64>::from_array(([1, seq_len], ids))?;
        let in_mask = Tensor::<i64>::from_array(([1, seq_len], mask.clone()))?;
        let in_types = Tensor::<i64>::from_array(([1, seq_len], type_ids))?;

        let outputs = session.run(ort::inputs![
            "input_ids" => in_ids,
            "attention_mask" => in_mask,
            "token_type_ids" => in_types
        ])?;

        let (_shape, data) = outputs[0].try_extract_tensor::<f32>()?;

        // Mean pooling over tokens where attention_mask == 1
        let mut pooled = vec![0.0f32; 384];
        let mut active_tokens = 0.0f32;

        for (i, &m) in mask.iter().enumerate().take(seq_len) {
            if m == 1 {
                active_tokens += 1.0;
                let offset = i * 384;
                for (j, item) in pooled.iter_mut().enumerate() {
                    *item += data[offset + j];
                }
            }
        }

        if active_tokens > 0.0 {
            for item in &mut pooled {
                *item /= active_tokens;
            }
        }

        // L2 Normalization to unit length
        let mut norm_sq = 0.0f32;
        for &v in &pooled {
            norm_sq += v * v;
        }
        let norm = norm_sq.sqrt();
        if norm > 1e-12 {
            for v in &mut pooled {
                *v /= norm;
            }
        }

        Ok(pooled)
    }

    /// Asynchronously computes a 384-dimensional unit vector embedding for the input text.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(EmbedderCommand::Embed {
                text: text.to_string(),
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Embedder actor disconnected"))?;
        rx.await.map_err(|_| anyhow::anyhow!("Embedder actor dropped response"))?
    }

    /// Asynchronously computes embeddings for a batch of input texts.
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(EmbedderCommand::EmbedBatch {
                texts: texts.to_vec(),
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Embedder actor disconnected"))?;
        rx.await.map_err(|_| anyhow::anyhow!("Embedder actor dropped response"))?
    }

    /// Stops the dedicated embedder actor thread.
    pub async fn shutdown(&self) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        if self.sender.send(EmbedderCommand::Shutdown { reply }).await.is_err() {
            return Ok(());
        }
        rx.await.map_err(|_| anyhow::anyhow!("Embedder actor dropped shutdown response"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ort_resolve_and_system32_rejection() {
        let path = resolve_ort_dylib_path().unwrap();
        assert!(!is_system32(&path));
        assert!(path.exists());
    }

    #[tokio::test]
    async fn test_local_embedder_real_onnx_semantics_and_latency() {
        let dylib = resolve_ort_dylib_path().unwrap();
        let (model_path, tokenizer_path) = match resolve_model_paths(None) {
            Ok(p) => p,
            Err(_) => return, // Skip test if models not provisioned
        };

        let embedder = LocalEmbedder::start(&model_path, &tokenizer_path, Some(&dylib)).unwrap();

        // Measure 5 consecutive warm calls for average latency
        let mut total_warm_ms = 0.0;
        let warm_samples = 5;
        let mut v_ai = Vec::new();
        for _ in 0..warm_samples {
            let start = std::time::Instant::now();
            v_ai = embedder.embed("Artificial intelligence and neural network embeddings").await.unwrap();
            total_warm_ms += start.elapsed().as_secs_f64() * 1000.0;
        }
        let avg_warm_ms = total_warm_ms / (warm_samples as f64);
        println!("Warm ONNX embedding latency (average of 5 calls): {:.2} ms", avg_warm_ms);

        // Verify L2 norm is ~1.0
        let norm: f32 = v_ai.iter().map(|&x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "Embedding must be unit normalized");

        // Semantic test: "machine learning" should be closer to "artificial intelligence" than "banana recipe"
        let v_ml = embedder.embed("Deep learning models and machine learning").await.unwrap();
        let v_fruit = embedder.embed("Delicious recipe for baking banana bread in an oven").await.unwrap();

        let dot_ai_ml: f32 = v_ai.iter().zip(&v_ml).map(|(&a, &b)| a * b).sum();
        let dot_ai_fruit: f32 = v_ai.iter().zip(&v_fruit).map(|(&a, &b)| a * b).sum();

        println!("Cosine similarity AI vs ML: {:.4}", dot_ai_ml);
        println!("Cosine similarity AI vs Fruit: {:.4}", dot_ai_fruit);

        assert!(
            dot_ai_ml > dot_ai_fruit,
            "Semantic similarity AI vs ML ({:.4}) must exceed AI vs Fruit ({:.4})",
            dot_ai_ml,
            dot_ai_fruit
        );

        embedder.shutdown().await.unwrap();
    }
}
