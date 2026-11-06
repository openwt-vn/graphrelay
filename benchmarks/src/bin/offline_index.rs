//! Build an index for a corpus into a *scratch* DB (never the live one), following
//! `full_index`'s order: graph pass -> optional SCIP ingest -> chunk backlog.
//!
//!   offline_index <corpus-root> <out.db> <scip|no-scip|upgrade>
//!
//! `scip` ingests `<root>/.graphrelay/index.scip` if present but never launches an
//! external indexer (`auto_run` forced off); `no-scip` skips enrichment entirely.
//! `upgrade` re-indexes an EXISTING db (a `.backup` copy of a pre-upgrade live index)
//! with SCIP off, exercising the migrations and old-symbol cleanup an installed
//! project goes through.
//!
//! Measures indexing time / memory / upgrade behaviour. It is NOT a retrieval-quality benchmark:
//! the source of truth for that is the agent benchmark in `benchmarks/dataset` (`run-sot.sh`).
use graphrelay_core::types::{IndexState, IndexStatus};
use graphrelay_core::{retrieval::SourceLoader, Config};
use graphrelay_parser::{Extractor, LanguageRegistry};
use graphrelay_store::{GraphDb, StoreCoordinator};
use graphrelay_watcher::{FileWatcher, IndexWorker};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).with_target(false).init();
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 4 {
        anyhow::bail!("usage: offline_index <corpus-root> <out.db> <scip|no-scip|upgrade>");
    }
    let root = PathBuf::from(&a[1]).canonicalize()?;
    let out = PathBuf::from(&a[2]);
    let (with_scip, upgrade) = match a[3].as_str() {
        "scip" => (true, false),
        "no-scip" => (false, false),
        "upgrade" => (false, true),
        other => anyhow::bail!("unknown mode {other}"),
    };
    if upgrade != out.exists() {
        anyhow::bail!(
            "{} {}; `upgrade` needs an existing db, the other modes refuse to overwrite one",
            out.display(),
            if out.exists() { "already exists" } else { "does not exist" },
        );
    }

    {
        let avail = graphrelay_watcher::memory_budget::available_bytes();
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        eprintln!(
            "memory probe: available {} MiB, cores {cores}, parse pool {}",
            avail / 1048576,
            graphrelay_watcher::memory_budget::resolve_pool_size(None, cores),
        );
    }
    let cfg_path = root.join(".graphrelay").join("config.toml");
    let mut config = if cfg_path.exists() { Config::load(&cfg_path)? } else { Config::default() };
    config.scip.auto_run = false;

    let graph_db = Arc::new(GraphDb::open(&out)?);
    let coordinator = Arc::new(StoreCoordinator::new(graph_db.clone()));
    let source_loader = Arc::new(SourceLoader::new());
    let status = Arc::new(RwLock::new(IndexStatus {
        state: IndexState::Indexing,
        last_indexed_at: None,
        pending_count: 0,
        total_symbols: 0,
        total_files: 0,
        last_error: None,
        semantic_pending: 0,
    }));
    let mut ignore = config.watcher.ignore.clone();
    ignore.extend(config.index.ignore.iter().cloned());
    let watcher = Arc::new(FileWatcher::new(root.clone(), &ignore, &config.index.languages)?);
    let extractor = Arc::new(Extractor::new(LanguageRegistry::with_defaults()).with_root(root.clone()));
    let worker = Arc::new(IndexWorker::new(
        watcher,
        extractor,
        coordinator,
        status,
        source_loader,
        config.watcher,
        config.scip,
    ));

    let t0 = std::time::Instant::now();
    let backlog = graphrelay_watcher::full_index_graph(&root, &worker).await?;
    eprintln!("graph pass: {:?}, backlog {}", t0.elapsed(), backlog.len());
    if with_scip {
        graphrelay_watcher::run_scip_enrichment(&root, &worker).await?;
        eprintln!("scip done: {:?}", t0.elapsed());
    }
    graphrelay_watcher::index_chunk_backlog(&worker, backlog).await?;
    eprintln!(
        "done: {} files, {} symbols, {:?}",
        graph_db.total_file_count().unwrap_or(0),
        graph_db.total_symbol_count().unwrap_or(0),
        t0.elapsed()
    );
    Ok(())
}
