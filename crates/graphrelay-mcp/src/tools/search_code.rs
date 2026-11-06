/// Plan 23 phase 2: `search_code` merges `find_relevant_code` (natural-
/// language intent) and `find_implementations` (named type lookup) behind
/// one MCP tool name. Both are graph/lexical search entry points that
/// differ mainly in query shape; consolidating the *listed* tool surface
/// (7 → 4) is what the benchmarked competitor tool does to reduce agent
/// mis-picks, without touching either underlying implementation — this is a
/// pure routing layer, the retrieval/graph logic in `find_relevant_code`
/// and `find_implementations` is untouched.
use crate::server::McpAppState;
use crate::tools::{find_implementations, find_relevant_code};
use anyhow::anyhow;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Args {
    pub intent:     Option<String>,
    pub implements: Option<String>,
    pub top_n:      Option<usize>,
    pub language:   Option<String>,
}

pub async fn handle(state: &McpAppState, args: Args) -> anyhow::Result<Value> {
    match (args.intent, args.implements) {
        (Some(_), Some(_)) => Err(anyhow!(
            "search_code: pass exactly one of `intent` or `implements` — not both. \
             `intent` is a natural-language description for fuzzy search; `implements` \
             is a specific interface/trait/base-class name for a graph-edge lookup."
        )),
        (None, None) => Err(anyhow!(
            "search_code: pass exactly one of `intent` or `implements` — neither was given. \
             Use `intent` for a natural-language question, `implements` to find every \
             implementation of a named interface/trait/base class."
        )),
        (Some(intent), None) => {
            find_relevant_code::handle(state, find_relevant_code::Args {
                intent,
                top_n: args.top_n,
                language: args.language,
            }).await
        }
        (None, Some(implements)) => {
            find_implementations::handle(state, find_implementations::Args {
                type_name: implements,
            }).await
        }
    }
}
