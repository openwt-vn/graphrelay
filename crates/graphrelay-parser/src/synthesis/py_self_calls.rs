//! Python `self.method()` calls to a method the class inherits. The base
//! resolver keys on the bare name, so a method defined on a parent class in
//! another file stays unresolved when the name is not unique. Here the
//! receiver is `self`, so the callee is the first definition up the class's
//! resolved `Inherits` chain. Only edges the base resolver did not already
//! produce are emitted. A method's class is found by span containment, not by
//! its qualified name, so nested classes and `name#N` siblings work.

use graphrelay_core::types::{Edge, EdgeKind, EdgeSource, Language, Symbol, SymbolId, SymbolKind};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

const MAX_ANCESTORS: usize = 64;

fn self_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bself\.([A-Za-z_]\w*)\s*\(").unwrap())
}

// Docstrings, string literals and comments, so a `self.x()` mentioned in prose is not a call.
fn noise_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?s)""".*?"""|'''.*?'''|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'|#[^\n]*"#).unwrap()
    })
}

#[derive(Default)]
pub struct PyClassIndex {
    method_class: HashMap<SymbolId, SymbolId>,
    methods: HashMap<SymbolId, HashMap<String, SymbolId>>,
}

pub fn class_index(symbols: &[Symbol]) -> PyClassIndex {
    let mut by_file: HashMap<&PathBuf, Vec<&Symbol>> = HashMap::new();
    for s in symbols.iter().filter(|s| s.language == Language::Python) {
        by_file.entry(&s.file_path).or_default().push(s);
    }
    let mut index = PyClassIndex::default();
    for syms in by_file.values_mut() {
        syms.sort_by_key(|s| s.span.start_byte);
        let containers: Vec<&&Symbol> = syms
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Class | SymbolKind::Function))
            .collect();
        for f in syms.iter().filter(|s| s.kind == SymbolKind::Function) {
            let narrowest = containers
                .iter()
                .filter(|c| {
                    c.id != f.id
                        && c.span.start_byte <= f.span.start_byte
                        && f.span.end_byte <= c.span.end_byte
                        && (c.span.end_byte - c.span.start_byte) > (f.span.end_byte - f.span.start_byte)
                })
                .min_by_key(|c| c.span.end_byte - c.span.start_byte);
            if let Some(c) = narrowest.filter(|c| c.kind == SymbolKind::Class) {
                index.method_class.insert(f.id, c.id);
                index.methods.entry(c.id).or_default().entry(f.name.clone()).or_insert(f.id);
            }
        }
    }
    index
}

fn ancestors(class: SymbolId, parents: &HashMap<SymbolId, Vec<SymbolId>>) -> Vec<SymbolId> {
    let mut out = Vec::new();
    let mut seen = HashSet::from([class]);
    let mut stack: Vec<SymbolId> = parents.get(&class).into_iter().flatten().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) || out.len() >= MAX_ANCESTORS {
            continue;
        }
        out.push(id);
        stack.extend(parents.get(&id).into_iter().flatten().rev().copied());
    }
    out
}

/// `symbols` are the (chunk of) symbols whose bodies to scan; `index` comes
/// from the full Python symbol set.
pub fn scan(
    symbols: &[Symbol],
    sources: &HashMap<PathBuf, String>,
    index: &PyClassIndex,
    parents: &HashMap<SymbolId, Vec<SymbolId>>,
    existing: &HashSet<(SymbolId, SymbolId)>,
) -> Vec<Edge> {
    let mut edges = Vec::new();
    for sym in symbols {
        if sym.language != Language::Python || sym.kind != SymbolKind::Function {
            continue;
        }
        let Some(&class_id) = index.method_class.get(&sym.id) else { continue };
        let Some(source) = sources.get(&sym.file_path) else { continue };
        let end = (sym.span.end_byte as usize).min(source.len());
        let Some(body) = source.get(sym.span.start_byte as usize..end) else { continue };
        let body = noise_re().replace_all(body, " ");
        let mut chain: Option<Vec<SymbolId>> = None;
        let mut emitted: HashSet<SymbolId> = HashSet::new();
        for cap in self_call_re().captures_iter(&body) {
            let name = &cap[1];
            let target = match index.methods.get(&class_id).and_then(|m| m.get(name)) {
                Some(&id) => Some(id),
                None => chain
                    .get_or_insert_with(|| ancestors(class_id, parents))
                    .iter()
                    .find_map(|a| index.methods.get(a)?.get(name).copied()),
            };
            let Some(to_id) = target.filter(|&t| t != sym.id && !existing.contains(&(sym.id, t))) else { continue };
            if emitted.insert(to_id) {
                edges.push(Edge {
                    from_id: sym.id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use graphrelay_core::types::Span;

    fn sym(id: SymbolId, name: &str, path: &str, kind: SymbolKind, source: &str, needle: &str) -> Symbol {
        let start = source.find(needle).unwrap();
        Symbol {
            id,
            file_id: 1,
            name: name.to_string(),
            qualified_name: format!("{path}::{name}"),
            kind,
            span: Span { start_byte: start as u32, end_byte: (start + needle.len()) as u32, start_line: 0, end_line: 0 },
            docstring: None,
            language: Language::Python,
            file_path: PathBuf::from(path),
        }
    }

    const CHILD: &str = "class Child(Base):\n    def go(self):\n        self.shared()\n        self.shared()\n        self.own()\n    def own(self):\n        pass\n";
    const BASE: &str = "class Base:\n    def shared(self):\n        pass\n";

    fn fixture() -> (Vec<Symbol>, HashMap<PathBuf, String>, HashMap<SymbolId, Vec<SymbolId>>) {
        let symbols = vec![
            sym(1, "Child", "c.py", SymbolKind::Class, CHILD, CHILD),
            sym(2, "go", "c.py", SymbolKind::Function, CHILD, "def go(self):\n        self.shared()\n        self.shared()\n        self.own()\n"),
            sym(3, "own", "c.py", SymbolKind::Function, CHILD, "def own(self):\n        pass\n"),
            sym(4, "Base", "b.py", SymbolKind::Class, BASE, BASE),
            sym(5, "shared", "b.py", SymbolKind::Function, BASE, "def shared(self):\n        pass\n"),
        ];
        let sources = HashMap::from([(PathBuf::from("c.py"), CHILD.to_string()), (PathBuf::from("b.py"), BASE.to_string())]);
        (symbols, sources, HashMap::from([(1, vec![4])]))
    }

    fn run(existing: HashSet<(SymbolId, SymbolId)>) -> Vec<Edge> {
        let (symbols, sources, parents) = fixture();
        scan(&symbols, &sources, &class_index(&symbols), &parents, &existing)
    }

    #[test]
    fn self_call_resolves_up_the_inherits_chain_once_per_target() {
        let edges = run(HashSet::new());
        let pairs: Vec<_> = edges.iter().map(|e| (e.from_id, e.to_id.unwrap())).collect();
        assert_eq!(pairs.len(), 2, "two call sites of self.shared() give one edge: {pairs:?}");
        assert_eq!(pairs.into_iter().collect::<HashSet<_>>(), HashSet::from([(2, 5), (2, 3)]));
    }

    #[test]
    fn edges_the_base_resolver_already_made_are_not_duplicated() {
        let edges = run(HashSet::from([(2, 3)]));
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_id, Some(5));
    }

    #[test]
    fn calls_named_only_in_strings_and_comments_are_ignored() {
        let src = "class Child(Base):\n    def go(self):\n        \"\"\"self.shared() in prose\"\"\"\n        # self.shared()\n        x = 'self.shared()'\n";
        let symbols = vec![
            sym(1, "Child", "c.py", SymbolKind::Class, src, src),
            sym(2, "go", "c.py", SymbolKind::Function, src, "def go(self):\n        \"\"\"self.shared() in prose\"\"\"\n        # self.shared()\n        x = 'self.shared()'\n"),
            sym(4, "Base", "b.py", SymbolKind::Class, BASE, BASE),
            sym(5, "shared", "b.py", SymbolKind::Function, BASE, "def shared(self):\n        pass\n"),
        ];
        let sources = HashMap::from([(PathBuf::from("c.py"), src.to_string()), (PathBuf::from("b.py"), BASE.to_string())]);
        let parents = HashMap::from([(1, vec![4])]);
        assert!(scan(&symbols, &sources, &class_index(&symbols), &parents, &HashSet::new()).is_empty());
    }

    #[test]
    fn nested_class_methods_and_overloaded_names_are_handled() {
        let src = "class Outer:\n    class Inner(Base):\n        def go(self):\n            self.shared()\n    @property\n    def v(self):\n        pass\n    @v.setter\n    def v(self, x):\n        self.shared()\n";
        let inner = "class Inner(Base):\n        def go(self):\n            self.shared()\n";
        let go = "def go(self):\n            self.shared()\n";
        let v1 = "def v(self):\n        pass\n";
        let v2 = "def v(self, x):\n        self.shared()\n";
        let symbols = vec![
            sym(1, "Outer", "n.py", SymbolKind::Class, src, src),
            sym(2, "Inner", "n.py", SymbolKind::Class, src, inner),
            sym(3, "go", "n.py", SymbolKind::Function, src, go),
            sym(6, "v", "n.py", SymbolKind::Function, src, v1),
            sym(7, "v", "n.py", SymbolKind::Function, src, v2),
            sym(4, "Base", "b.py", SymbolKind::Class, BASE, BASE),
            sym(5, "shared", "b.py", SymbolKind::Function, BASE, "def shared(self):\n        pass\n"),
        ];
        let sources = HashMap::from([(PathBuf::from("n.py"), src.to_string()), (PathBuf::from("b.py"), BASE.to_string())]);
        let parents = HashMap::from([(2, vec![4]), (1, vec![4])]);
        let pairs: HashSet<_> = scan(&symbols, &sources, &class_index(&symbols), &parents, &HashSet::new())
            .iter()
            .map(|e| (e.from_id, e.to_id.unwrap()))
            .collect();
        assert_eq!(pairs, HashSet::from([(3, 5), (7, 5)]));
    }
}
