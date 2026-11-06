use crossbeam_channel::{bounded, Receiver};
use notify::{event::RenameMode, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

/// Maps a `[index] languages` config entry (e.g. "rust", "csharp") to its
/// real file extension(s) by dispatching through
/// `LanguageRegistry`/`LanguageSupport::file_extensions()` — the same
/// per-language source of truth `Extractor` itself uses to parse a file.
/// A previous hand-maintained match statement duplicated this mapping and
/// drifted from it: it produced the wrong extension for csharp ("csharp"
/// instead of "cs"), ruby ("ruby" instead of "rb"), and kotlin ("kotlin"
/// instead of "kt"/"kts"), so those languages were never actually watched
/// even when explicitly listed in config — and it had no entry at all for
/// several already-supported languages. Deriving from `LanguageRegistry`
/// instead means a language can't silently drift out of sync between "the
/// parser supports it" and "the watcher will ever hand it a file."
fn languages_to_extensions(languages: &[String]) -> HashSet<String> {
    let registry = graphrelay_parser::LanguageRegistry::with_defaults();
    let mut exts = HashSet::new();
    for lang_name in languages {
        match graphrelay_core::types::Language::from_str(lang_name) {
            Some(lang) => match registry.for_language(lang) {
                Some(support) => {
                    exts.extend(support.file_extensions().iter().map(|s| s.to_string()));
                }
                None => {
                    tracing::warn!(
                        "config lists language '{lang_name}' but it has no registered \
                         extractor — no files will be watched for it",
                    );
                }
            },
            None => {
                // Unknown language name: fall back to using it as a literal
                // extension, so a config typo doesn't silently drop the
                // entry outright — it just won't match unless the name
                // happens to be the extension too.
                exts.insert(lang_name.clone());
            }
        }
    }
    exts
}

pub struct FileWatcher {
    _watcher:     RecommendedWatcher,
    events:       Receiver<notify::Result<Event>>,
    root:         PathBuf,
    ignores:      Vec<String>,
    allowed_exts: HashSet<String>,
    overflow:     Arc<AtomicBool>,
}

impl FileWatcher {
    pub fn new(root: PathBuf, ignore_patterns: &[String], languages: &[String]) -> anyhow::Result<Self> {
        let (tx, rx) = bounded(1000);
        let overflow = Arc::new(AtomicBool::new(false));
        let overflow_tx = overflow.clone();

        let mut watcher = notify::recommended_watcher(move |event| {
            if tx.try_send(event).is_err() {
                overflow_tx.store(true, Ordering::Relaxed);
            }
        })?;

        watcher.watch(&root, RecursiveMode::Recursive)?;

        Ok(Self {
            _watcher:     watcher,
            events:       rx,
            root,
            ignores:      ignore_patterns.to_vec(),
            allowed_exts: languages_to_extensions(languages),
            overflow,
        })
    }

    /// Returns true (and clears the flag) if events were dropped due to queue overflow.
    pub fn has_overflow(&self) -> bool {
        self.overflow.swap(false, Ordering::Relaxed)
    }

    /// Re-arm the overflow flag (see `IndexWorker::run`: an overflow noticed
    /// while a full index is already running must be retried, not dropped).
    pub fn mark_overflow(&self) {
        self.overflow.store(true, Ordering::Relaxed);
    }

    /// Drain all pending events from the channel without blocking.
    pub fn drain_all(&self) -> Vec<PathBuf> {
        let mut changed = HashSet::new();
        while let Ok(Ok(event)) = self.events.try_recv() {
            self.process_event(event, &mut changed);
        }
        changed.into_iter().collect()
    }

    /// Collect a debounced batch of changed paths.
    pub fn next_batch(&self, debounce_ms: u64) -> Vec<PathBuf> {
        let mut changed: HashSet<PathBuf> = HashSet::new();

        // Wait for the first event (1-second timeout)
        match self.events.recv_timeout(Duration::from_secs(1)) {
            Ok(Ok(event)) => self.process_event(event, &mut changed),
            _ => return vec![],
        }
        // The window opens at the first event; starting it before the wait let a
        // late first event skip the drain, so bursts arrived one event per batch.
        let deadline = Instant::now() + Duration::from_millis(debounce_ms);

        // Drain within debounce window
        while Instant::now() < deadline {
            match self.events.recv_deadline(deadline) {
                Ok(Ok(event)) => self.process_event(event, &mut changed),
                _ => break,
            }
        }

        changed.into_iter().collect()
    }

    fn process_event(&self, event: Event, changed: &mut HashSet<PathBuf>) {
        match event.kind {
            // Rename{Both}: paths[0] = old name, paths[1] = new name.
            // Enqueue the old path (index_path will delete it since it no longer
            // exists — for a renamed directory, `index_path_graph_only` deletes
            // everything indexed under it) and the new path (to be indexed; a
            // renamed directory's files are enumerated because the OS reports
            // only the directory itself).
            EventKind::Modify(notify::event::ModifyKind::Name(RenameMode::Both))
                if event.paths.len() == 2 =>
            {
                changed.insert(event.paths[0].clone());
                self.enqueue_path(&event.paths[1], changed);
            }
            EventKind::Remove(_) => {
                for path in event.paths {
                    if self.should_index(&path) || self.may_be_removed_directory(&path) {
                        changed.insert(path);
                    }
                }
            }
            // A rename reported as separate halves (no `Both`): the "from" half
            // is a removal (a directory moved out of the tree arrives only
            // this way), the "to" half a creation.
            EventKind::Modify(notify::event::ModifyKind::Name(mode)) => {
                for path in event.paths {
                    let gone = matches!(mode, RenameMode::From) || !path.exists();
                    if gone {
                        if self.should_index(&path) || self.may_be_removed_directory(&path) {
                            changed.insert(path);
                        }
                    } else {
                        self.enqueue_path(&path, changed);
                    }
                }
            }
            // Only creation can introduce a whole directory of files. Plain data/
            // metadata modifications name individual files (or a directory whose
            // timestamp changed — walking that would re-read the whole tree).
            EventKind::Create(_) => {
                for path in event.paths {
                    self.enqueue_path(&path, changed);
                }
            }
            EventKind::Modify(_) => {
                for path in event.paths {
                    if self.should_index(&path) {
                        changed.insert(path);
                    }
                }
            }
            _ => {}
        }
    }

    /// Enqueue a file if indexable; for a directory, every indexable file under it.
    fn enqueue_path(&self, path: &Path, changed: &mut HashSet<PathBuf>) {
        if path.is_dir() {
            if self.is_ignored(path) {
                return;
            }
            let walker = walkdir::WalkDir::new(path)
                .into_iter()
                .filter_entry(|e| !(e.file_type().is_dir() && self.is_ignored(e.path())));
            for entry in walker.filter_map(|e| e.ok()) {
                if entry.file_type().is_file() && self.should_index(entry.path()) {
                    changed.insert(entry.path().to_owned());
                }
            }
        } else if self.should_index(path) {
            changed.insert(path.to_owned());
        }
    }

    /// A removed path we can no longer stat: an extensionless one might have been
    /// a directory whose indexed files must go. Deleting under a path that was
    /// really an extensionless file is a harmless no-op.
    fn may_be_removed_directory(&self, path: &Path) -> bool {
        !path.exists() && path.extension().is_none() && !self.is_ignored(path)
    }

    pub fn is_ignored(&self, path: &Path) -> bool {
        let rel = path.strip_prefix(&self.root).unwrap_or(path);
        let segments: Vec<String> = rel
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
        self.ignores.iter().any(|pattern| matches_ignore(pattern, &segments))
    }

    pub fn should_index(&self, path: &Path) -> bool {
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e,
            None    => return false,
        };
        if !self.allowed_exts.is_empty() && !self.allowed_exts.contains(ext) {
            return false;
        }
        !self.is_ignored(path) && !path.is_dir()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Gitignore-style, segment-aligned matching: `target/**` ignores a `target`
/// directory at any depth, never a file merely containing "target" in its
/// name (the old substring matcher dropped e.g. `domains/distribution.py`
/// via `dist/**`). A leading `/` anchors the pattern at the project root.
/// Matching a run of leading directories ignores everything under them.
fn matches_ignore(pattern: &str, path: &[&str]) -> bool {
    let anchored = pattern.starts_with('/');
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    if pat.is_empty() {
        return false;
    }
    let starts = if anchored { 0..1 } else { 0..path.len() };
    starts.into_iter().any(|i| match_prefix(&pat, &path[i.min(path.len())..]))
}

/// True if `pat` matches some prefix of `path` (zero trailing segments allowed
/// only for a `**`).
fn match_prefix(pat: &[&str], path: &[&str]) -> bool {
    match pat.split_first() {
        None => true,
        Some((&"**", rest)) => (0..=path.len()).any(|k| match_prefix(rest, &path[k..])),
        Some((seg, rest)) => match path.split_first() {
            Some((p, path_rest)) => wildcard_match(seg.as_bytes(), p.as_bytes()) && match_prefix(rest, path_rest),
            None => false,
        },
    }
}

/// `*` and `?` within a single path segment.
fn wildcard_match(pat: &[u8], s: &[u8]) -> bool {
    let (mut p, mut i, mut star, mut mark) = (0, 0, None, 0);
    while i < s.len() {
        if p < pat.len() && (pat[p] == b'?' || pat[p] == s[i]) {
            p += 1;
            i += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = Some(p);
            mark = i;
            p += 1;
        } else if let Some(sp) = star {
            p = sp + 1;
            mark += 1;
            i = mark;
        } else {
            return false;
        }
    }
    pat[p..].iter().all(|&c| c == b'*')
}

#[cfg(test)]
mod tests {
    use super::languages_to_extensions;

    /// Regression test for the specific mismatches found while adding
    /// Kotlin/Lua support: these three previously resolved to the wrong
    /// extension (the language's own config name) because a hand-maintained
    /// match statement had drifted from `LanguageRegistry`'s real
    /// `file_extensions()` for each language.
    #[test]
    fn previously_broken_languages_resolve_to_their_real_extension() {
        let exts = languages_to_extensions(&[
            "csharp".to_string(),
            "ruby".to_string(),
            "kotlin".to_string(),
        ]);
        assert!(exts.contains("cs"), "csharp should map to .cs, got: {exts:?}");
        assert!(!exts.contains("csharp"), "csharp must not map to literal 'csharp', got: {exts:?}");
        assert!(exts.contains("rb"), "ruby should map to .rb, got: {exts:?}");
        assert!(!exts.contains("ruby"), "ruby must not map to literal 'ruby', got: {exts:?}");
        assert!(exts.contains("kt"), "kotlin should map to .kt, got: {exts:?}");
        assert!(exts.contains("kts"), "kotlin should also map to .kts, got: {exts:?}");
        assert!(!exts.contains("kotlin"), "kotlin must not map to literal 'kotlin', got: {exts:?}");
    }

    #[test]
    fn unknown_language_name_falls_back_to_itself() {
        let exts = languages_to_extensions(&["not_a_real_language".to_string()]);
        assert!(exts.contains("not_a_real_language"));
    }

    fn ignored(pattern: &str, path: &str) -> bool {
        let segs: Vec<&str> = path.split('/').collect();
        super::matches_ignore(pattern, &segs)
    }

    #[test]
    fn ignore_patterns_match_whole_segments_not_substrings() {
        assert!(ignored("dist/**", "dist/app.js"));
        assert!(ignored("node_modules/**", "packages/ui/node_modules/x/index.js"));
        assert!(ignored("target/**", "target"));
        assert!(ignored(".git/**", ".git/hooks/pre-commit.py"));
        assert!(!ignored("dist/**", "erpnext/domains/distribution.py"));
        assert!(!ignored("target/**", "patches/move_target_distribution.py"));
        assert!(!ignored("vendor/**", "stock/landed_cost_vendor_invoice/x.py"));
        assert!(!ignored(".git/**", ".github/workflows/ci.py"));
    }

    #[test]
    fn ignore_extension_and_anchored_patterns() {
        assert!(ignored("*.min.js", "static/js/app.min.js"));
        assert!(!ignored("*.min.js", "static/js/app.js"));
        assert!(ignored("*.lock", "Cargo.lock"));
        assert!(ignored("/build", "build/out.rs"));
        assert!(!ignored("/build", "src/build/out.rs"));
        assert!(ignored("build", "src/build/out.rs"));
        assert!(ignored("src/gen/**", "crates/a/src/gen/x.rs"));
        assert!(ignored("a?c", "abc/x.rs"));
    }

    #[test]
    fn multi_extension_languages_get_every_extension() {
        let exts = languages_to_extensions(&["javascript".to_string()]);
        for e in ["js", "jsx", "mjs", "cjs"] {
            assert!(exts.contains(e), "javascript should include .{e}, got: {exts:?}");
        }
    }
}
