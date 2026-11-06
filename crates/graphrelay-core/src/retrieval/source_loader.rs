use crate::retrieval::pipeline::is_generated_filename;
use crate::types::Symbol;
use dashmap::DashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Loads and caches source file content, providing snippet extraction.
pub struct SourceLoader {
    cache: DashMap<PathBuf, Arc<String>>,
    /// Plan 22 item 1: per-path "is this file machine-generated" verdict,
    /// cached alongside `cache` (invalidated together in `invalidate`) since
    /// computing it needs the same file content `enrich()` already loads for
    /// `snippet` — the content-banner scan is essentially free once the
    /// content is in hand, but still worth caching across the many
    /// candidates from the same file a single query can enrich.
    generated_cache: DashMap<PathBuf, bool>,
}

impl SourceLoader {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
            generated_cache: DashMap::new(),
        }
    }

    pub fn snippet(&self, symbol: &Symbol) -> anyhow::Result<String> {
        let source = self.load(&symbol.file_path)?;
        let lines: Vec<&str> = source.lines().collect();

        let start = symbol.span.start_line as usize;
        let end   = (symbol.span.end_line as usize + 1).min(lines.len());

        if start >= lines.len() {
            tracing::warn!(
                symbol = %symbol.qualified_name,
                file   = %symbol.file_path.display(),
                start_line = symbol.span.start_line,
                total_lines = lines.len(),
                "symbol span out of bounds — stale index entry?"
            );
            return Ok(String::new());
        }
        Ok(lines[start..end].join("\n"))
    }

    /// Plan 23: whole-file text for `get_file_context`'s opt-in
    /// `include_source` mode — a thin public wrapper around the existing
    /// private, cached `load()`. `snippet()` already returns a full,
    /// untruncated symbol span; this exposes the same cached whole-file read
    /// for a caller that needs the entire file rather than one symbol's span.
    pub fn full_text(&self, path: &PathBuf) -> anyhow::Result<Arc<String>> {
        self.load(path)
    }

    fn load(&self, path: &PathBuf) -> anyhow::Result<Arc<String>> {
        if let Some(cached) = self.cache.get(path) {
            return Ok(cached.clone());
        }
        // Do NOT cache failures — the file might reappear after a re-index.
        let content = std::fs::read_to_string(path).map_err(|e| {
            tracing::warn!(path = %path.display(), "source file unreadable: {e}");
            e
        })?;
        let arc = Arc::new(content);
        self.cache.insert(path.clone(), arc.clone());
        Ok(arc)
    }

    /// Invalidate cached content for a path (call after re-indexing a file).
    pub fn invalidate(&self, path: &Path) {
        self.cache.remove(path);
        self.generated_cache.remove(path);
    }

    /// Plan 22 item 1: is `path` machine-generated? Filename convention
    /// first (cheap, no I/O), then a content-banner scan of the file's
    /// leading comment block (reuses whatever `load` already cached for
    /// `snippet`). Unreadable files are conservatively judged not-generated
    /// — same posture as `snippet`'s own graceful-degradation on a stale
    /// index entry, and consistent with this feature's stated bias toward
    /// misses over false positives.
    pub fn is_generated(&self, path: &Path) -> bool {
        if is_generated_filename(path) {
            return true;
        }
        if let Some(cached) = self.generated_cache.get(path) {
            return *cached;
        }
        let verdict = match self.load(&path.to_path_buf()) {
            Ok(content) => has_generated_banner(&content),
            Err(_) => false,
        };
        self.generated_cache.insert(path.to_path_buf(), verdict);
        verdict
    }
}

/// Plan 22 item 1: extract the file's leading comment block — lines from the
/// top, stopping at the first line that isn't blank and isn't a comment
/// (first non-whitespace chars are a recognized comment token). Comment-
/// syntax-agnostic on purpose: a per-language banner table across 30+
/// languages would be unmaintainable and could drift the way the watcher's
/// old hand-written extension map once did (see CLAUDE.md). Capped at 40
/// lines / 4KB — banners are always near the very top of a file.
///
/// Deliberately does **not** treat a bare `'` as a comment token even though
/// some Lisp dialects use it for a quote form — found the hard way against
/// real lexical corpus data: `sidebars.js` opens with `'use strict';`, and
/// treating that line as a comment would have let the scan continue straight
/// into a later, unrelated JSDoc-style comment mentioning "generated" in
/// plain prose ("The sidebars can be generated from the filesystem..."),
/// which must NOT count as a banner. Stopping at `'use strict';` (correctly,
/// since it's real code) keeps that prose out of the leading block entirely.
fn leading_comment_text(source: &str) -> String {
    const COMMENT_PREFIXES: &[&str] = &[
        "//", "/*", "*/", "*", "--", ";", "%", "<!--", "(*", "#",
    ];
    const MAX_LINES: usize = 40;
    const MAX_BYTES: usize = 4096;

    let mut out = String::new();
    for line in source.lines().take(MAX_LINES) {
        if out.len() >= MAX_BYTES {
            break;
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        // "REM" (Batch files) is a whole-word, case-insensitive marker, not a
        // fixed-position prefix like the others.
        let content = if trimmed.len() >= 3
            && trimmed[..3].eq_ignore_ascii_case("rem")
            && trimmed.as_bytes().get(3).is_none_or(|b| b.is_ascii_whitespace())
        {
            Some(trimmed[3..].trim_start())
        } else {
            COMMENT_PREFIXES
                .iter()
                .find_map(|p| trimmed.strip_prefix(p).map(str::trim_start))
        };
        match content {
            Some(c) => {
                out.push_str(c);
                out.push('\n');
            }
            None => break,
        }
    }
    out
}

/// Plan 22 item 1: does the file's leading comment block look like a
/// generated-code banner? Four patterns, all found in real generator output
/// (not hypothesized):
/// 1. A `Code generated ... DO NOT EDIT` line (protoc-gen-go and the many
///    tools that copy its convention) — case-insensitive on both halves.
/// 2. A standalone `@generated` token.
/// 3. `<auto-generated` (the .NET/XML-doc-comment convention).
/// 4. "generated" and a "don't touch this" phrase both present somewhere in
///    the block — **not required to be on the same line**. Verified
///    necessary against real keycloak JAXB output: its actual banner reads
///    "This file was generated by the JavaTM Architecture for XML
///    Binding(JAXB)..." on one `//` line and "Any modifications to this file
///    will be lost upon recompilation of the source schema." on the next —
///    it never says "do not edit" at all, and the two phrases are on
///    different lines. A same-line-only rule (as originally specced) misses
///    this real, already-confirmed-present corpus example entirely.
fn has_generated_banner(source: &str) -> bool {
    let block = leading_comment_text(source);
    if block.is_empty() {
        return false;
    }
    let lower = block.to_lowercase();

    for line in block.lines() {
        let t = line.trim();
        if let Some(rest) = case_insensitive_strip_prefix(t, "code generated ") {
            if case_insensitive_ends_with(rest.trim_end_matches('.'), "do not edit") {
                return true;
            }
        }
    }

    if lower
        .split(|c: char| !c.is_alphanumeric() && c != '@')
        .any(|w| w == "@generated")
    {
        return true;
    }

    if lower.contains("<auto-generated") {
        return true;
    }

    let mentions_generated = lower.contains("generated");
    let mentions_do_not_touch = lower.contains("do not edit")
        || lower.contains("do not modify")
        || lower.contains("will be lost");
    mentions_generated && mentions_do_not_touch
}

/// Byte-slicing at a fixed offset would panic on a multi-byte UTF-8 boundary
/// for arbitrary source content — `is_char_boundary` guards that before ever
/// slicing (both `prefix`/`suffix` here are ASCII literals, but `s` is not).
fn case_insensitive_strip_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() < prefix.len() || !s.is_char_boundary(prefix.len()) {
        return None;
    }
    if s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn case_insensitive_ends_with(s: &str, suffix: &str) -> bool {
    if s.len() < suffix.len() {
        return false;
    }
    let boundary = s.len() - suffix.len();
    s.is_char_boundary(boundary) && s[boundary..].eq_ignore_ascii_case(suffix)
}

impl Default for SourceLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod generated_banner_tests {
    use super::has_generated_banner;

    /// Real corpus-scan finding (Plan 22 item 1): erpnext's indented
    /// in-class "auto-generated types" region starts well after `import`/
    /// `class` lines, so it's never part of the leading comment block at all
    /// — this pins that a loose whole-file scan would have wrongly caught
    /// it, but the leading-block-only design doesn't.
    #[test]
    fn erpnext_shaped_indented_in_class_region_is_not_generated() {
        let source = r#"import frappe
from frappe.model.document import Document


class Account(Document):
	# begin: auto-generated types
	# This code is auto-generated. Do not modify anything in this block.

	from typing import TYPE_CHECKING

	if TYPE_CHECKING:
		from frappe.types import DF

	account_name: DF.Data
	# end: auto-generated types

	def validate(self):
		pass
"#;
        assert!(!has_generated_banner(source));
    }

    /// Real corpus-scan finding: a test file's `"@generated"` inside a
    /// string literal (not a comment, not in the leading block) must not
    /// count.
    #[test]
    fn generated_token_inside_a_string_literal_is_not_generated() {
        let source = r#"describe('markers', () => {
  it('should render an @generated marker as literal text', () => {
    expect(render('@generated')).toBe('@generated');
  });
});
"#;
        assert!(!has_generated_banner(source));
    }

    /// Real corpus-scan finding: `sidebars.js`'s own leading comment
    /// eventually says "The sidebars can be generated from the filesystem"
    /// — prose, not a banner. Also regression-pins that `'use strict';`
    /// (a real statement, not a Lisp-style quote-comment) correctly ends the
    /// leading comment block before that later prose is ever reached.
    #[test]
    fn sidebars_js_shaped_prose_comment_is_not_generated() {
        let source = "/**\n * Copyright (c) Meta Platforms, Inc. and affiliates.\n */\n\n'use strict';\n/**\n * Creating a sidebar enables you to:\n - create an ordered group\n\n The sidebars can be generated from the filesystem, or explicitly defined here.\n */\n";
        assert!(!has_generated_banner(source));
    }

    /// Real corpus finding: keycloak's actual JAXB banner never says "do not
    /// edit" — it says "Any modifications to this file will be lost upon
    /// recompilation", on a *different* line than "generated". A same-line-
    /// only rule would miss this; the whole-leading-block co-occurrence
    /// check here must not.
    #[test]
    fn jaxb_shaped_license_header_then_banner_is_generated() {
        let source = "/*\n * Copyright 2016 Red Hat, Inc. and/or its affiliates\n * Licensed under the Apache License, Version 2.0\n */\n\n//\n// This file was generated by the JavaTM Architecture for XML Binding(JAXB) Reference Implementation, vhudson-jaxb-ri-2.1-661\n// See <a href=\"http://java.sun.com/xml/jaxb\">http://java.sun.com/xml/jaxb</a>\n// Any modifications to this file will be lost upon recompilation of the source schema.\n// Generated on: 2008.12.08 at 05:45:20 PM CST\n//\n\npackage org.keycloak.dom.saml.v2.ac;\n";
        assert!(has_generated_banner(source));
    }

    #[test]
    fn go_protoc_gen_go_banner_is_generated() {
        let source = "// Code generated by protoc-gen-go. DO NOT EDIT.\n// source: example.proto\n\npackage examplepb\n";
        assert!(has_generated_banner(source));
    }

    #[test]
    fn pb2_shaped_file_with_encoding_line_and_banner_is_generated() {
        let source = "# -*- coding: utf-8 -*-\n# Generated by the protocol buffer compiler.  DO NOT EDIT!\nimport sys\n";
        assert!(has_generated_banner(source));
    }

    #[test]
    fn standalone_generated_token_is_generated() {
        let source = "// @generated\n// This file is maintained by a code generator.\n\npackage main\n";
        assert!(has_generated_banner(source));
    }

    #[test]
    fn auto_generated_xml_doc_comment_is_generated() {
        let source = "//------------------------------------------------------------------------------\n// <auto-generated>\n//     This code was generated by a tool.\n// </auto-generated>\n//------------------------------------------------------------------------------\n\nnamespace Foo {\n";
        assert!(has_generated_banner(source));
    }

    #[test]
    fn ordinary_hand_written_file_is_not_generated() {
        let source = "// Copyright 2026 Example Corp.\n// Licensed under the MIT license.\n\npackage main\n\nfunc main() {}\n";
        assert!(!has_generated_banner(source));
    }

    #[test]
    fn empty_source_is_not_generated() {
        assert!(!has_generated_banner(""));
    }
}

#[cfg(test)]
mod generated_filename_tests {
    use super::super::pipeline::is_generated_filename;
    use std::path::Path;

    #[test]
    fn filename_rule_hit_with_no_banner_is_generated() {
        assert!(is_generated_filename(Path::new("api_grpc.pb.go")));
        assert!(is_generated_filename(Path::new("messages_pb2.py")));
        assert!(is_generated_filename(Path::new("messages_pb2_grpc.py")));
        assert!(is_generated_filename(Path::new("widget.g.dart")));
        assert!(is_generated_filename(Path::new("widget.freezed.dart")));
        assert!(is_generated_filename(Path::new("Form1.Designer.cs")));
        assert!(is_generated_filename(Path::new("Model.g.cs")));
        assert!(is_generated_filename(Path::new("zz_generated.deepcopy.go")));
        assert!(is_generated_filename(Path::new("types.generated.ts")));
        assert!(is_generated_filename(Path::new("foo_generated_types.py")));
    }

    #[test]
    fn ordinary_filenames_are_not_generated() {
        assert!(!is_generated_filename(Path::new("main.go")));
        assert!(!is_generated_filename(Path::new("account.py")));
        assert!(!is_generated_filename(Path::new("LexicalEditor.ts")));
        // "generated" as a real English word in a hand-written filename
        // component, without the `.generated.`/`_generated.`/`_generated_`
        // delimited-segment shape, must not false-positive.
        assert!(!is_generated_filename(Path::new("regenerated_report.py")));
    }
}
