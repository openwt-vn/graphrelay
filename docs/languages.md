# Supported languages

31 languages via tree-sitter, taken from `LanguageRegistry::with_defaults()`.

Verified directly against `LanguageRegistry::with_defaults()` — not a marketing list, the actual registry:

| Language | Extensions |
|---|---|
| Rust | `.rs` |
| TypeScript | `.ts` `.tsx` |
| JavaScript | `.js` `.jsx` `.mjs` `.cjs` |
| Python | `.py` `.pyi` |
| Go | `.go` |
| Java | `.java` |
| C | `.c` `.h` |
| C++ | `.cpp` `.cc` `.cxx` `.hpp` `.hxx` |
| C# | `.cs` |
| Ruby | `.rb` |
| Kotlin | `.kt` `.kts` |
| Swift | `.swift` |
| PHP | `.php` |
| Scala | `.scala` `.sc` |
| Lua | `.lua` |
| Vue | `.vue` (SFC `<script>` delegated to JS/TS) |
| Svelte | `.svelte` (SFC `<script>` delegated to JS/TS) |
| Bash | `.sh` `.bash` |
| Zig | `.zig` |
| Elixir | `.ex` `.exs` |
| Julia | `.jl` |
| Fortran | `.f90` `.f95` `.f03` `.f08` `.f` (+ uppercase variants) |
| OCaml | `.ml` `.mli` |
| Common Lisp | `.lisp` `.cl` `.lsp` `.asd` |
| Verilog / SystemVerilog | `.v` `.sv` `.svh` |
| Pascal | `.pas` `.pp` `.dpr` `.dpk` `.lpr` `.inc` |
| Dart | `.dart` |
| Objective-C | `.m` `.mm` |
| Groovy | `.groovy` `.gradle` |
| PowerShell | `.ps1` `.psm1` |
| JSON | `.json` (top-level object keys only — see `crates/graphrelay-parser/src/language/json_lang.rs`) |

Want to add one? You need a tree-sitter grammar (ABI-14 compatible — check this before you pin a version, it's a common trap), a `LanguageSupport` impl in `crates/graphrelay-parser/src/language/`, and a registry entry. `CLAUDE.md`'s "Languages supported" section walks through the ABI gotcha in more detail.
