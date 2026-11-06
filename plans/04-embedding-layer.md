# Plan 04 — Embedding Layer

## Goal
Implement a pluggable embedding interface that ships `bge-small-en-v1.5` (ONNX, CPU) by default and can be swapped to `bge-code-v1` or `Voyage-code-3` via config. Produce **graph-aware chunks** that embed not just the symbol body but also its structural neighbors.

## Success Criteria
- Embedding throughput: ≥ 100 chunks/s on an M-series Mac CPU (batch=32)
- Model load time: < 3 s cold start
- Chunk quality: each chunk carries symbol path + callers + callees as prefix context
- Switching model requires only config change — no code change
- Embedding dimension is read from the ONNX model at runtime (not hardcoded)

---

## 1. `EmbeddingModel` Trait

```rust
// crates/codegraph-embed/src/model.rs

#[async_trait::async_trait]
pub trait EmbeddingModel: Send + Sync {
    fn dim(&self) -> usize;
    fn max_tokens(&self) -> usize;

    async fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;

    async fn embed_one(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(self.embed_batch(&[text.to_string()]).await?.remove(0))
    }
}
```

---

## 2. Graph-Aware Chunk Construction

Before embedding, build a chunk that encodes the symbol's structural context:

```rust
// crates/codegraph-core/src/chunk.rs

pub struct ChunkBuilder;

impl ChunkBuilder {
    /// Build a text chunk that carries structural context as a prefix.
    ///
    /// Format:
    ///   [file: src/query/engine.rs | symbol: QueryEngine::execute | kind: Function]
    ///   [callers: search_handler, route_handler]
    ///   [callees: GraphDb::neighbors, VectorDb::search, rerank]
    ///   <docstring if present>
    ///
    ///   <symbol body — truncated to max_tokens>
    pub fn build(
        symbol:   &Symbol,
        body:     &str,           // raw source text of the symbol
        callers:  &[Symbol],      // from graph DB
        callees:  &[Symbol],      // from graph DB
        max_chars: usize,         // model's effective context limit in chars
    ) -> Chunk {
        let mut parts = Vec::new();

        // Structural prefix
        parts.push(format!(
            "[file: {} | symbol: {} | kind: {:?}]",
            symbol.file_path.display(),
            symbol.qualified_name,
            symbol.kind,
        ));

        if !callers.is_empty() {
            let names: Vec<_> = callers.iter().take(5).map(|s| s.name.as_str()).collect();
            parts.push(format!("[callers: {}]", names.join(", ")));
        }
        if !callees.is_empty() {
            let names: Vec<_> = callees.iter().take(5).map(|s| s.name.as_str()).collect();
            parts.push(format!("[callees: {}]", names.join(", ")));
        }

        if let Some(doc) = &symbol.docstring {
            parts.push(doc.chars().take(300).collect::<String>());
        }

        // Body — fill remaining budget
        let prefix = parts.join("\n");
        let body_budget = max_chars.saturating_sub(prefix.len() + 2);
        let body_truncated: String = body.chars().take(body_budget).collect();

        parts.push(body_truncated);

        let text = parts.join("\n");

        Chunk {
            symbol_id:  symbol.id,
            text,
            embedding:  None,
            graph_meta: GraphMeta {
                qualified_name: symbol.qualified_name.clone(),
                file_path:      symbol.file_path.clone(),
                caller_names:   callers.iter().map(|s| s.name.clone()).collect(),
                callee_names:   callees.iter().map(|s| s.name.clone()).collect(),
            },
        }
    }
}
```

### Why graph-aware chunks?
Standard chunking (split by tokens or lines) loses structural relationships. By prepending caller/callee names as metadata, the embedding model can learn that `execute` is called by `search_handler` — making it retrievable when someone queries "how does search_handler trigger execution?".

---

## 3. BGE-small-en-v1.5 via ONNX Runtime (`ort`)

```rust
// crates/codegraph-embed/src/bge_small.rs

use ort::{Environment, Session, SessionBuilder, Value};
use tokenizers::Tokenizer;

pub struct BgeSmallModel {
    session:   Session,
    tokenizer: Tokenizer,
    dim:       usize,
}

impl BgeSmallModel {
    pub fn load(model_path: &Path, tokenizer_path: &Path) -> anyhow::Result<Self> {
        let env = Environment::builder()
            .with_name("codegraph")
            .with_log_level(ort::LoggingLevel::Warning)
            .build()?
            .into_arc();

        let session = SessionBuilder::new(&env)?
            .with_intra_threads(num_cpus::get())?
            .with_model_from_file(model_path)?;

        // Read output dim from session metadata
        let dim = session.outputs[0]
            .dimensions()
            .last()
            .cloned()
            .ok_or_else(|| anyhow!("cannot read embedding dim from ONNX model"))? as usize;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow!("tokenizer load failed: {e}"))?;

        Ok(Self { session, tokenizer, dim })
    }
}

#[async_trait::async_trait]
impl EmbeddingModel for BgeSmallModel {
    fn dim(&self) -> usize { self.dim }
    fn max_tokens(&self) -> usize { 512 }

    async fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        // Tokenize
        let encodings = self.tokenizer
            .encode_batch(texts.iter().map(|s| s.as_str()).collect(), true)
            .map_err(|e| anyhow!("tokenize failed: {e}"))?;

        let batch_size = texts.len();
        let seq_len    = encodings[0].get_ids().len();

        // Build input tensors: input_ids, attention_mask, token_type_ids
        let input_ids: Vec<i64> = encodings.iter()
            .flat_map(|e| e.get_ids().iter().map(|&id| id as i64))
            .collect();
        let attention_mask: Vec<i64> = encodings.iter()
            .flat_map(|e| e.get_attention_mask().iter().map(|&m| m as i64))
            .collect();
        let token_type_ids: Vec<i64> = vec![0i64; batch_size * seq_len];

        let shape = vec![batch_size as i64, seq_len as i64];
        let inputs = vec![
            Value::from_array(([batch_size, seq_len], input_ids.as_slice()))?,
            Value::from_array(([batch_size, seq_len], attention_mask.as_slice()))?,
            Value::from_array(([batch_size, seq_len], token_type_ids.as_slice()))?,
        ];

        let outputs = self.session.run(inputs)?;

        // Masked mean pooling over token dimension, then L2-normalize
        // IMPORTANT: must pass attention_mask to exclude padding tokens from the average
        let last_hidden: ort::tensor::OrtOwnedTensor<f32, _> = outputs[0].try_extract()?;
        let view = last_hidden.view();
        let pooled = mean_pool_and_normalize(view, &attention_mask, batch_size, seq_len, self.dim);

        Ok(pooled)
    }
}

fn mean_pool_and_normalize(
    hidden:  ndarray::ArrayViewD<f32>,
    masks:   &[i64],            // flattened attention_mask [batch * seq]
    batch: usize, seq: usize, dim: usize,
) -> Vec<Vec<f32>> {
    (0..batch).map(|b| {
        let mut v       = vec![0f32; dim];
        let mut n_valid = 0f32;
        for t in 0..seq {
            // BGE mean-pool must exclude padding tokens (mask == 0)
            let mask_val = masks[b * seq + t];
            if mask_val == 0 { continue; }
            n_valid += 1.0;
            for d in 0..dim {
                v[d] += hidden[[b, t, d]];
            }
        }
        let n = n_valid.max(1.0);   // avoid div-by-zero on empty sequences
        for x in &mut v { *x /= n; }
        // L2 normalize
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 1e-9 { for x in &mut v { *x /= norm; } }
        v
    }).collect()
}
```

---

## 4. Model Download & Caching

Ship ONNX weights as a first-time download, not bundled in the binary (too large ~50 MB):

```rust
// crates/codegraph-embed/src/download.rs

const BGE_SMALL_ONNX_URL: &str = "https://huggingface.co/BAAI/bge-small-en-v1.5/resolve/main/onnx/model.onnx";
const BGE_SMALL_TOK_URL:  &str = "https://huggingface.co/BAAI/bge-small-en-v1.5/resolve/main/tokenizer.json";

pub async fn ensure_model(model_dir: &Path, model_name: &str) -> anyhow::Result<ModelPaths> {
    let (onnx_url, tok_url) = match model_name {
        "bge-small-en-v1.5" => (BGE_SMALL_ONNX_URL, BGE_SMALL_TOK_URL),
        "bge-code-v1"       => (BGE_CODE_ONNX_URL, BGE_CODE_TOK_URL),
        other => return Err(anyhow!("unknown model: {other}")),
    };

    let onnx_path = model_dir.join(format!("{model_name}.onnx"));
    let tok_path  = model_dir.join(format!("{model_name}-tokenizer.json"));

    if !onnx_path.exists() {
        eprintln!("Downloading {model_name} model weights (~50 MB)...");
        download_file(onnx_url, &onnx_path).await?;
    }
    if !tok_path.exists() {
        download_file(tok_url, &tok_path).await?;
    }

    Ok(ModelPaths { onnx: onnx_path, tokenizer: tok_path })
}
```

For air-gapped environments: `CODEGRAPH_MODEL_PATH=/path/to/model.onnx` env var overrides download.

---

## 5. Batch Embedding Queue

The watcher sends chunks to an async batch queue to maximize GPU/CPU utilization:

```rust
// crates/codegraph-embed/src/batch.rs

pub struct BatchEmbedder {
    model:      Arc<dyn EmbeddingModel>,
    batch_size: usize,
}

impl BatchEmbedder {
    pub async fn embed_chunks(&self, chunks: Vec<Chunk>) -> anyhow::Result<Vec<EmbeddedChunk>> {
        let mut results = Vec::with_capacity(chunks.len());

        for batch in chunks.chunks(self.batch_size) {
            let texts: Vec<String> = batch.iter().map(|c| c.text.clone()).collect();
            let embeddings = self.model.embed_batch(&texts).await?;

            for (chunk, embedding) in batch.iter().zip(embeddings) {
                results.push(EmbeddedChunk {
                    symbol_id:     chunk.symbol_id,
                    qualified_name: chunk.graph_meta.qualified_name.clone(),
                    chunk_text:    chunk.text.clone(),
                    file_path:     chunk.graph_meta.file_path.to_string_lossy().to_string(),
                    start_line:    0, // filled by caller
                    language:      String::new(),
                    vector:        embedding,
                });
            }
        }

        Ok(results)
    }
}
```

---

## 6. Pluggable Model Config

```toml
# codegraph.toml
[embedding]
model       = "bge-small-en-v1.5"   # or "bge-code-v1", "voyage-code-3"
model_path  = ""                     # override auto-download
batch_size  = 32
device      = "cpu"
```

```rust
pub fn load_model_from_config(cfg: &EmbeddingConfig) -> anyhow::Result<Arc<dyn EmbeddingModel>> {
    match cfg.model.as_str() {
        "bge-small-en-v1.5" | "bge-code-v1" => {
            let paths = tokio::runtime::Handle::current()
                .block_on(ensure_model(&cfg.model_dir(), &cfg.model))?;
            Ok(Arc::new(BgeSmallModel::load(&paths.onnx, &paths.tokenizer)?))
        },
        "voyage-code-3" => {
            Ok(Arc::new(VoyageModel::new(
                std::env::var("VOYAGE_API_KEY")
                    .map_err(|_| anyhow!("VOYAGE_API_KEY env var required for voyage-code-3"))?
            )))
        },
        other => Err(anyhow!("unknown embedding model: {other}")),
    }
}
```

---

## 7. Query Embedding

When a user query arrives at MCP/REST, embed it with the same model. BGE-small uses query prefix:

```rust
pub async fn embed_query(model: &dyn EmbeddingModel, query: &str) -> anyhow::Result<Vec<f32>> {
    // BGE-small: prefix with "Represent this sentence for searching relevant passages: "
    let prefixed = format!("Represent this sentence for searching relevant passages: {query}");
    model.embed_one(&prefixed).await
}

pub async fn embed_chunk_text(model: &dyn EmbeddingModel, text: &str) -> anyhow::Result<Vec<f32>> {
    // Chunks are embedded without prefix
    model.embed_one(text).await
}
```

---

## 8. Implementation Steps

1. Write `EmbeddingModel` trait in `codegraph-embed/src/model.rs`
2. Implement `ensure_model()` with sha256 checksum verification
3. Implement `BgeSmallModel::load()` via `ort`
4. Implement `embed_batch()` with mean-pool + L2-normalize
5. Implement `ChunkBuilder::build()` in `codegraph-core/src/chunk.rs`
6. Implement `BatchEmbedder` with configurable batch size
7. Implement model factory `load_model_from_config()`
8. Benchmark: embed 32 chunks → must be < 320 ms (≥ 100 chunks/s)
9. Test: embed same text twice → cosine similarity must be 1.0 (deterministic)
10. Test: query prefix yields higher cosine similarity to its correct document than random noise

---

## 9. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| ONNX runtime | `ort` crate vs `candle` | `ort` — production-ready, better perf, easier ONNX import |
| Quantization | FP32 vs INT8 | FP32 default; add INT8 flag later for speed if needed |
| Model download | HuggingFace Hub vs bundled | Download on first run, cache in `~/.codegraph/models/` |
| Voyage-code-3 | HTTP client | `reqwest` async, same interface via `EmbeddingModel` trait |
| Tokenizer padding | right-pad to `max_tokens` | Yes — consistent batching |
