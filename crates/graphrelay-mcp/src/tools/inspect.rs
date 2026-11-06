/// Plan 23 phase 2: `inspect` merges `explain_symbol` (single-symbol deep
/// dive), `get_file_context` (whole-file orientation), and `summarize_module`
/// (directory/module orientation) behind one MCP tool name. Grouped by *what
/// shape of question the agent has* ("I know a symbol" / "I know a path"),
/// not by response shape — `path` further routes to file-context or
/// module-summary behavior depending on what it actually resolves to. Pure
/// routing layer; none of the three underlying implementations are rewritten.
use crate::server::McpAppState;
use crate::tools::{explain_symbol, get_file_context, resolve_file_lookup, summarize_module, PathLookup};
use anyhow::anyhow;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Args {
    pub symbol:         Option<String>,
    pub path:           Option<String>,
    pub focus_line:     Option<u32>,
    pub include_source: Option<bool>,
    pub max_symbols:    Option<usize>,
}

pub async fn handle(state: &McpAppState, args: Args) -> anyhow::Result<Value> {
    match (args.symbol, args.path) {
        (Some(_), Some(_)) => Err(anyhow!(
            "inspect: pass exactly one of `symbol` or `path` — not both. `symbol` looks up \
             one specific function/class/struct; `path` looks up a file or a directory/module."
        )),
        (None, None) => Err(anyhow!(
            "inspect: pass exactly one of `symbol` or `path` — neither was given."
        )),
        (Some(symbol), None) => {
            explain_symbol::handle(state, explain_symbol::Args { symbol }).await
        }
        (None, Some(path)) => {
            // Hard error on a real ambiguity (multiple indexed files share the
            // given suffix) — never silently fall through to a module summary,
            // which would quietly answer a different question than the one asked.
            match resolve_file_lookup(state, &path)? {
                PathLookup::File(_) => {
                    // Default to true (get_file_context's own module-level
                    // default stays false, for its unit tests and any other
                    // internal caller) — a root-cause investigation after
                    // shipping this opt-in found the agent essentially never
                    // passed `include_source: true` explicitly (0/6 in a
                    // real benchmark sample), then `Read` the same file
                    // anyway every single time, making the capability a
                    // dead letter in practice. Defaulting to on removes the
                    // redundant Read without depending on the agent
                    // reliably choosing to opt in.
                    get_file_context::handle(state, get_file_context::Args {
                        path,
                        focus_line: args.focus_line,
                        include_source: Some(args.include_source.unwrap_or(true)),
                    }).await
                }
                PathLookup::Ambiguous(e) => Err(e),
                PathLookup::NotAFile => {
                    summarize_module::handle(state, summarize_module::Args {
                        path,
                        max_symbols: args.max_symbols,
                    }).await
                }
            }
        }
    }
}
