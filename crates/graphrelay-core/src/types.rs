use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type SymbolId = u64;
pub type FileId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Const,
    TypeAlias,
    Method,
    Module,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function  => "Function",
            SymbolKind::Class     => "Class",
            SymbolKind::Struct    => "Struct",
            SymbolKind::Enum      => "Enum",
            SymbolKind::Trait     => "Trait",
            SymbolKind::Interface => "Interface",
            SymbolKind::Const     => "Const",
            SymbolKind::TypeAlias => "TypeAlias",
            SymbolKind::Method    => "Method",
            SymbolKind::Module    => "Module",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Function"  => Some(SymbolKind::Function),
            "Class"     => Some(SymbolKind::Class),
            "Struct"    => Some(SymbolKind::Struct),
            "Enum"      => Some(SymbolKind::Enum),
            "Trait"     => Some(SymbolKind::Trait),
            "Interface" => Some(SymbolKind::Interface),
            "Const"     => Some(SymbolKind::Const),
            "TypeAlias" => Some(SymbolKind::TypeAlias),
            "Method"    => Some(SymbolKind::Method),
            "Module"    => Some(SymbolKind::Module),
            _           => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Calls,
    Imports,
    References,
    Inherits,
    Implements,
}

impl EdgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeKind::Calls      => "Calls",
            EdgeKind::Imports    => "Imports",
            EdgeKind::References => "References",
            EdgeKind::Inherits   => "Inherits",
            EdgeKind::Implements => "Implements",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Calls"      => Some(EdgeKind::Calls),
            "Imports"    => Some(EdgeKind::Imports),
            "References" => Some(EdgeKind::References),
            "Inherits"   => Some(EdgeKind::Inherits),
            "Implements" => Some(EdgeKind::Implements),
            _            => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Ruby,
    Kotlin,
    Swift,
    Php,
    Scala,
    Lua,
    Vue,
    Svelte,
    Bash,
    Zig,
    Elixir,
    Julia,
    Fortran,
    Ocaml,
    CommonLisp,
    Verilog,
    Pascal,
    Dart,
    ObjectiveC,
    Groovy,
    PowerShell,
    Json,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Rust       => "rust",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Python     => "python",
            Language::Go         => "go",
            Language::Java       => "java",
            Language::C          => "c",
            Language::Cpp        => "cpp",
            Language::CSharp     => "csharp",
            Language::Ruby       => "ruby",
            Language::Kotlin     => "kotlin",
            Language::Swift      => "swift",
            Language::Php        => "php",
            Language::Scala      => "scala",
            Language::Lua        => "lua",
            Language::Vue        => "vue",
            Language::Svelte     => "svelte",
            Language::Bash       => "bash",
            Language::Zig        => "zig",
            Language::Elixir     => "elixir",
            Language::Julia      => "julia",
            Language::Fortran    => "fortran",
            Language::Ocaml      => "ocaml",
            Language::CommonLisp => "commonlisp",
            Language::Verilog    => "verilog",
            Language::Pascal     => "pascal",
            Language::Dart       => "dart",
            Language::ObjectiveC => "objectivec",
            Language::Groovy     => "groovy",
            Language::PowerShell => "powershell",
            Language::Json       => "json",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "rust"       => Some(Language::Rust),
            "typescript" => Some(Language::TypeScript),
            "javascript" => Some(Language::JavaScript),
            "python"     => Some(Language::Python),
            "go"         => Some(Language::Go),
            "java"       => Some(Language::Java),
            "c"          => Some(Language::C),
            "cpp"        => Some(Language::Cpp),
            "csharp"     => Some(Language::CSharp),
            "ruby"       => Some(Language::Ruby),
            "kotlin"     => Some(Language::Kotlin),
            "swift"      => Some(Language::Swift),
            "php"        => Some(Language::Php),
            "scala"      => Some(Language::Scala),
            "lua"        => Some(Language::Lua),
            "vue"        => Some(Language::Vue),
            "svelte"     => Some(Language::Svelte),
            "bash"       => Some(Language::Bash),
            "zig"        => Some(Language::Zig),
            "elixir"     => Some(Language::Elixir),
            "julia"      => Some(Language::Julia),
            "fortran"    => Some(Language::Fortran),
            "ocaml"      => Some(Language::Ocaml),
            "commonlisp" => Some(Language::CommonLisp),
            "verilog"    => Some(Language::Verilog),
            "pascal"     => Some(Language::Pascal),
            "dart"       => Some(Language::Dart),
            "objectivec" => Some(Language::ObjectiveC),
            "groovy"     => Some(Language::Groovy),
            "powershell" => Some(Language::PowerShell),
            "json"       => Some(Language::Json),
            _            => None,
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexState {
    Fresh,
    Indexing,
    Stale,
    Error,
}

impl IndexState {
    pub fn as_str(&self) -> &'static str {
        match self {
            IndexState::Fresh    => "fresh",
            IndexState::Indexing => "indexing",
            IndexState::Stale    => "stale",
            IndexState::Error    => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub start_byte: u32,
    pub end_byte:   u32,
    pub start_line: u32,
    pub end_line:   u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id:             SymbolId,
    pub file_id:        FileId,
    pub name:           String,
    pub qualified_name: String,
    pub kind:           SymbolKind,
    pub span:           Span,
    pub docstring:      Option<String>,
    pub language:       Language,
    pub file_path:      PathBuf,
}

/// Indicates whether an edge was derived from syntax (tree-sitter), semantic
/// analysis (SCIP compiler-backed indexer), or synthesized by a dynamic-
/// dispatch heuristic (e.g. a C function-pointer dispatch table, or a
/// string-keyed callback registration) that recovers a call edge no literal
/// call-site expression exists for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EdgeSource {
    #[default]
    Syntax,
    Semantic,
    Synthesized,
}

impl EdgeSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeSource::Syntax      => "syntax",
            EdgeSource::Semantic    => "semantic",
            EdgeSource::Synthesized => "synthesized",
        }
    }

}

impl std::str::FromStr for EdgeSource {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "semantic"    => EdgeSource::Semantic,
            "synthesized" => EdgeSource::Synthesized,
            _             => EdgeSource::Syntax,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from_id: SymbolId,
    pub to_id:   Option<SymbolId>,
    pub to_name: Option<String>,
    pub kind:    EdgeKind,
    /// Provenance: syntax = tree-sitter same-file, semantic = SCIP cross-file.
    #[serde(default)]
    pub source:  EdgeSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id:         FileId,
    pub path:       PathBuf,
    pub checksum:   u64,
    pub language:   Language,
    pub indexed_at: String,
    pub byte_size:  u64,
}

#[derive(Debug, Clone)]
pub struct FileExtraction {
    pub file_record: FileRecord,
    pub symbols:     Vec<Symbol>,
    pub edges:       Vec<Edge>,
    pub source:      Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStatus {
    pub state:            IndexState,
    pub last_indexed_at:  Option<chrono::DateTime<chrono::Utc>>,
    pub pending_count:    usize,
    pub total_symbols:    u64,
    pub total_files:      u64,
    pub last_error:       Option<String>,
    /// Files whose graph data is indexed but whose chunks are not yet embedded.
    /// `state` can be `Fresh` (graph-queryable) while this is still > 0 — semantic
    /// search results will be incomplete until it reaches 0.
    pub semantic_pending: usize,
}

impl IndexStatus {
    pub fn fresh() -> Self {
        Self {
            state:            IndexState::Fresh,
            last_indexed_at:  None,
            pending_count:    0,
            total_symbols:    0,
            total_files:      0,
            last_error:       None,
            semantic_pending: 0,
        }
    }

    pub fn age_seconds(&self) -> Option<i64> {
        self.last_indexed_at.map(|t| (chrono::Utc::now() - t).num_seconds())
    }
}

/// A text chunk ready for lexical indexing, with graph-aware structural context prefix.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub symbol_id:  SymbolId,
    pub text:       String,
    pub graph_meta: GraphMeta,
}

#[derive(Debug, Clone)]
pub struct GraphMeta {
    pub qualified_name: String,
    pub file_path:      PathBuf,
    pub caller_names:   Vec<String>,
    pub callee_names:   Vec<String>,
}

/// A chunk ready to write into the lexical (FTS5) index.
#[derive(Debug, Clone)]
pub struct IndexedChunk {
    pub symbol_id:      SymbolId,
    pub qualified_name: String,
    pub chunk_text:     String,
    pub file_path:      String,
    pub start_line:     u32,
    pub language:       String,
}

/// A symbol reference (lightweight version for result lists).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolRef {
    pub id:             SymbolId,
    pub qualified_name: String,
    pub kind:           SymbolKind,
}

impl From<Symbol> for SymbolRef {
    fn from(s: Symbol) -> Self {
        SymbolRef {
            id:             s.id,
            qualified_name: s.qualified_name,
            kind:           s.kind,
        }
    }
}

impl From<&Symbol> for SymbolRef {
    fn from(s: &Symbol) -> Self {
        SymbolRef {
            id:             s.id,
            qualified_name: s.qualified_name.clone(),
            kind:           s.kind,
        }
    }
}

/// Impact graph returned by trace_impact.
#[derive(Debug, Clone)]
pub struct ImpactGraph {
    pub root:     SymbolId,
    pub affected: Vec<ImpactNode>,
    pub depth:    u8,
    /// The traversal stopped at its node budget, so `affected` is a lower
    /// bound on the real impact set.
    pub budget_hit: bool,
}

#[derive(Debug, Clone)]
pub struct ImpactNode {
    pub symbol:      Symbol,
    pub path:        PathBuf,
    pub reason:      String,
    /// Source of the edge on the shortest discovered path to this node (real DB value,
    /// not inferred from file-path comparison).
    pub edge_source: EdgeSource,
    /// Hop count from the root symbol on the shortest discovered path.
    pub hop_depth:   u8,
    /// Qualified name of the symbol one hop closer to the root that this node
    /// depends on, on that same path. `None` at hop 1 (the root itself).
    pub via:         Option<String>,
}
