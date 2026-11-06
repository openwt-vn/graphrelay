use crate::server::McpAppState;
use anyhow::anyhow;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Args {
    pub path:        String,
    pub max_symbols: Option<usize>,
}

pub async fn handle(state: &McpAppState, args: Args) -> anyhow::Result<Value> {
    let path = args.path.trim().trim_end_matches('/');
    if path.is_empty() {
        return Err(anyhow!("path must not be empty — did you mean to summarize the whole index?"));
    }

    let mut files = state.graph_db.files_in_prefix(path)?;
    // `path` is documented as a directory/module prefix, but agents naturally pass a
    // specific file (e.g. the same path they'd hand get_file_context) when they mean
    // "summarize the module this file lives in" — `files_in_prefix`'s `%/{prefix}/%`
    // pattern requires something *after* the prefix, which a leaf file never has, so
    // this silently returned zero results instead of the file's containing directory.
    // Falling back to the parent directory of a single matched file recovers the
    // clearly-intended behavior instead of erroring on a foreseeable input shape.
    if files.is_empty() {
        if let Some(f) = state.graph_db.file_by_path(Path::new(path))?
            .or_else(|| state.graph_db.files_by_suffix(Path::new(path)).ok()
                .and_then(|mut v| if v.len() == 1 { Some(v.remove(0)) } else { None }))
        {
            if let Some(parent) = f.path.parent() {
                files = state.graph_db.files_in_prefix(&parent.to_string_lossy())?;
            }
        }
    }
    if files.is_empty() {
        return Err(anyhow!("no indexed files found under: {}", args.path));
    }

    let max_syms = args.max_symbols.unwrap_or(10).min(50);

    let file_ids: Vec<_> = files.iter().map(|f| f.id).collect();
    let all_symbols = state.graph_db.symbols_in_files(&file_ids)?;

    let mut kind_counts: HashMap<String, usize> = HashMap::new();
    for sym in &all_symbols {
        *kind_counts.entry(sym.kind.as_str().to_string()).or_insert(0) += 1;
    }

    let mut langs: Vec<String> = files
        .iter()
        .map(|f| f.language.as_str().to_string())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    langs.sort();

    let symbol_ids: Vec<_> = all_symbols.iter().map(|s| s.id).collect();
    let degrees = state.graph_db.in_degrees_batch(&symbol_ids)?;
    let mut sym_with_degree: Vec<_> = all_symbols
        .iter()
        .map(|s| {
            let deg = degrees.get(&s.id).copied().unwrap_or(0);
            (s, deg)
        })
        .collect();
    sym_with_degree.sort_by_key(|&(_, deg)| std::cmp::Reverse(deg));
    sym_with_degree.truncate(max_syms);

    let key_symbols: Vec<Value> = sym_with_degree
        .into_iter()
        .map(|(s, deg)| {
            serde_json::json!({
                "name":           s.name,
                "qualified_name": s.qualified_name,
                "kind":           s.kind.as_str(),
                "file":           s.file_path.to_string_lossy(),
                "in_degree":      deg
            })
        })
        .collect();

    Ok(serde_json::json!({
        "module":       args.path,
        "file_count":   files.len(),
        "symbol_count": all_symbols.len(),
        "languages":    langs,
        "symbol_kinds": kind_counts,
        "key_symbols":  key_symbols
    }))
}
