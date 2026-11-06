//! Python dotted-path string dispatch: `frappe.enqueue(method="pkg.mod.func")`,
//! `frappe.get_attr("pkg.mod.func")`, hook tables. No call expression names
//! `func`, so the call graph never links the enqueuing function to it. A
//! quoted `a.b.c.func` whose module part matches a project file and whose last
//! part is a top-level function there is treated as a call from the enclosing
//! function. Strings in module-level code (e.g. `hooks.py`) have no enclosing
//! function and are skipped.

use graphrelay_core::types::{Edge, EdgeKind, EdgeSource, Language, Symbol, SymbolId, SymbolKind};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn dotted_string_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"["']((?:[A-Za-z_]\w*\.){2,}[A-Za-z_]\w*)["']"#).unwrap())
}

fn is_test_path(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "tests")
        || path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("test_"))
}

pub struct PyTargets {
    by_name: HashMap<String, Vec<(String, SymbolId)>>,
}

/// Top-level Python functions, keyed by name with their dotted module path
/// (from the root-relative qualified name, `pkg/mod.py::func`).
pub fn py_targets(symbols: &[Symbol]) -> PyTargets {
    let mut by_name: HashMap<String, Vec<(String, SymbolId)>> = HashMap::new();
    for sym in symbols {
        if sym.language != Language::Python || sym.kind != SymbolKind::Function {
            continue;
        }
        let Some((file, rest)) = sym.qualified_name.split_once("::") else { continue };
        if rest != sym.name {
            continue;
        }
        let Some(module) = file.strip_suffix(".py") else { continue };
        let module = module.trim_end_matches("/__init__").replace('/', ".");
        by_name.entry(sym.name.clone()).or_default().push((module, sym.id));
    }
    PyTargets { by_name }
}

impl PyTargets {
    pub fn has_targets(&self) -> bool {
        !self.by_name.is_empty()
    }

    fn resolve(&self, dotted: &str) -> Option<SymbolId> {
        let (module, name) = dotted.rsplit_once('.')?;
        let mut found = self.by_name.get(name)?.iter().filter(|(m, _)| {
            m == module || m.strip_suffix(module).is_some_and(|p| p.ends_with('.'))
        });
        let first = found.next()?;
        found.next().is_none().then_some(first.1)
    }
}

pub fn scan(symbols: &[Symbol], sources: &HashMap<PathBuf, String>, targets: &PyTargets) -> Vec<Edge> {
    let mut by_file: HashMap<&PathBuf, Vec<&Symbol>> = HashMap::new();
    for sym in symbols {
        if sym.language == Language::Python
            && matches!(sym.kind, SymbolKind::Function | SymbolKind::Method)
            && !is_test_path(&sym.file_path)
        {
            by_file.entry(&sym.file_path).or_default().push(sym);
        }
    }
    let mut edges = Vec::new();
    for (path, funcs) in by_file {
        let Some(source) = sources.get(path) else { continue };
        for cap in dotted_string_re().captures_iter(source) {
            let Some(to_id) = targets.resolve(&cap[1]) else { continue };
            let at = cap.get(0).map_or(0, |m| m.start()) as u32;
            // Only the narrowest enclosing function owns the string; a nested
            // closure would otherwise double-attribute it to its outer function.
            let owner = funcs
                .iter()
                .filter(|f| f.span.start_byte <= at && at < f.span.end_byte)
                .min_by_key(|f| f.span.end_byte - f.span.start_byte);
            if let Some(owner) = owner.filter(|o| o.id != to_id) {
                edges.push(Edge {
                    from_id: owner.id,
                    to_id: Some(to_id),
                    to_name: None,
                    kind: EdgeKind::Calls,
                    source: EdgeSource::Synthesized,
                });
            }
        }
    }
    edges
}

pub fn synthesize(symbols: &[Symbol], sources: &HashMap<PathBuf, String>) -> Vec<Edge> {
    let targets = py_targets(symbols);
    if !targets.has_targets() {
        return Vec::new();
    }
    scan(symbols, sources, &targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphrelay_core::types::Span;

    fn sym(id: SymbolId, qn: &str, path: &str, kind: SymbolKind, source: &str, needle: &str) -> Symbol {
        let start = source.find(needle).unwrap();
        Symbol {
            id,
            file_id: 1,
            name: qn.rsplit("::").next().unwrap().to_string(),
            qualified_name: qn.to_string(),
            kind,
            span: Span { start_byte: start as u32, end_byte: (start + needle.len()) as u32, start_line: 0, end_line: 0 },
            docstring: None,
            language: Language::Python,
            file_path: PathBuf::from(path),
        }
    }

    #[test]
    fn enqueue_by_dotted_string_links_caller_to_target() {
        let a = "def on_submit(self):\n    frappe.enqueue(method=\"app.jobs.worker.run_job\", queue=\"long\")\n";
        let b = "def run_job():\n    pass\n";
        let symbols = vec![
            sym(1, "app/pcv.py::on_submit", "app/pcv.py", SymbolKind::Function, a, a),
            sym(2, "app/jobs/worker.py::run_job", "app/jobs/worker.py", SymbolKind::Function, b, b),
        ];
        let sources = HashMap::from([(PathBuf::from("app/pcv.py"), a.to_string()), (PathBuf::from("app/jobs/worker.py"), b.to_string())]);
        let edges = synthesize(&symbols, &sources);
        assert_eq!(edges.len(), 1);
        assert_eq!((edges[0].from_id, edges[0].to_id), (1, Some(2)));
    }

    #[test]
    fn unknown_paths_ambiguous_names_and_test_files_are_skipped() {
        let a = "def f():\n    x = \"a.b.missing\"\n    y = \"x.y.dup\"\n    return \"app.jobs.worker.run_job\"\n";
        let b = "def run_job():\n    pass\n";
        let d = "def dup():\n    pass\n";
        let symbols = vec![
            sym(1, "app/test_pcv.py::f", "app/test_pcv.py", SymbolKind::Function, a, a),
            sym(2, "app/jobs/worker.py::run_job", "app/jobs/worker.py", SymbolKind::Function, b, b),
            sym(3, "p/x/y.py::dup", "p/x/y.py", SymbolKind::Function, d, d),
            sym(4, "q/x/y.py::dup", "q/x/y.py", SymbolKind::Function, d, d),
        ];
        let sources = HashMap::from([(PathBuf::from("app/test_pcv.py"), a.to_string())]);
        assert!(synthesize(&symbols, &sources).is_empty());
        let mut s2 = symbols.clone();
        s2[0].file_path = PathBuf::from("app/pcv.py");
        s2[0].qualified_name = "app/pcv.py::f".into();
        let sources = HashMap::from([(PathBuf::from("app/pcv.py"), a.to_string())]);
        let edges = synthesize(&s2, &sources);
        assert_eq!(edges.len(), 1, "only the unambiguous target resolves");
        assert_eq!(edges[0].to_id, Some(2));
    }

    #[test]
    fn a_string_in_a_nested_function_belongs_to_the_inner_function_only() {
        let a = "def outer():\n    def inner():\n        return \"app.jobs.worker.run_job\"\n    return inner\n";
        let inner = "def inner():\n        return \"app.jobs.worker.run_job\"\n";
        let b = "def run_job():\n    pass\n";
        let symbols = vec![
            sym(1, "app/m.py::outer", "app/m.py", SymbolKind::Function, a, a),
            sym(3, "app/m.py::outer::inner", "app/m.py", SymbolKind::Function, a, inner),
            sym(2, "app/jobs/worker.py::run_job", "app/jobs/worker.py", SymbolKind::Function, b, b),
        ];
        let sources = HashMap::from([(PathBuf::from("app/m.py"), a.to_string())]);
        let edges = synthesize(&symbols, &sources);
        assert_eq!(edges.len(), 1);
        assert_eq!((edges[0].from_id, edges[0].to_id), (3, Some(2)));
    }
}
