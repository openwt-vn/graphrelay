/// Comprehensive language parser coverage tests — all languages in
/// `LanguageRegistry::with_defaults()` (see that function for the current
/// count; keep this comment in sync rather than hardcoding a number here).
/// `import_query()` IS wired into `EdgeKind::Imports` edges (Extractor::extract_edges).
/// KNOWN GRAMMAR LIMITATIONS (tree-sitter version constraints):
///   (All grammar limitations resolved via keyword-discriminant queries)
use graphrelay_core::types::{EdgeKind, SymbolKind};
use graphrelay_parser::{Extractor, LanguageRegistry};
use std::path::Path;

fn ex() -> Extractor { Extractor::new(LanguageRegistry::default()) }

fn symbols(src: &[u8], ext: &str) -> Vec<(String, SymbolKind)> {
    ex().extract(Path::new(&format!("t.{ext}")), src).unwrap()
        .symbols.into_iter().map(|s| (s.name, s.kind)).collect()
}

fn has_calls(src: &[u8], ext: &str) -> bool {
    ex().extract(Path::new(&format!("t.{ext}")), src).unwrap()
        .edges.iter().any(|e| e.kind == EdgeKind::Calls)
}

fn find(syms: &[(String, SymbolKind)], name: &str) -> Option<SymbolKind> {
    syms.iter().find(|(n, _)| n == name).map(|(_, k)| *k)
}

/// Names of `EdgeKind::Inherits` targets for edges whose `from` symbol is
/// named `from_name`. Base classes in these snippets are never locally
/// declared, so `to_id` stays unresolved and the name comes from `to_name`.
fn inherits_targets(src: &[u8], ext: &str, from_name: &str) -> Vec<String> {
    let r = ex().extract(Path::new(&format!("t.{ext}")), src).unwrap();
    let from_id = r.symbols.iter().find(|s| s.name == from_name).map(|s| s.id);
    let Some(from_id) = from_id else { return Vec::new() };
    r.edges.iter()
        .filter(|e| e.kind == EdgeKind::Inherits && e.from_id == from_id)
        .filter_map(|e| e.to_name.clone())
        .collect()
}

/// Names of `EdgeKind::References` targets for edges whose `from` symbol is
/// named `from_name` — same resolution convention as `inherits_targets`.
fn references_targets(src: &[u8], ext: &str, from_name: &str) -> Vec<String> {
    let r = ex().extract(Path::new(&format!("t.{ext}")), src).unwrap();
    let from_id = r.symbols.iter().find(|s| s.name == from_name).map(|s| s.id);
    let Some(from_id) = from_id else { return Vec::new() };
    r.edges.iter()
        .filter(|e| e.kind == EdgeKind::References && e.from_id == from_id)
        .filter_map(|e| e.to_name.clone())
        .collect()
}

/// Names of `EdgeKind::Implements` targets for edges whose `from` symbol is
/// named `from_name` — same resolution convention as `inherits_targets`.
fn implements_targets(src: &[u8], ext: &str, from_name: &str) -> Vec<String> {
    let r = ex().extract(Path::new(&format!("t.{ext}")), src).unwrap();
    let from_id = r.symbols.iter().find(|s| s.name == from_name).map(|s| s.id);
    let Some(from_id) = from_id else { return Vec::new() };
    r.edges.iter()
        .filter(|e| e.kind == EdgeKind::Implements && e.from_id == from_id)
        .filter_map(|e| e.to_name.clone())
        .collect()
}

// ── RUST ──────────────────────────────────────────────────────────
#[test] fn rust_impl_method() {
    let s = symbols(b"struct Foo; impl Foo { pub fn bar(&self) {} }", "rs");
    assert_eq!(find(&s,"bar"), Some(SymbolKind::Function));
    assert_eq!(find(&s,"Foo"), Some(SymbolKind::Struct));
}
#[test] fn rust_trait() {
    let s = symbols(b"trait Greet { fn hello(&self); }", "rs");
    assert_eq!(find(&s,"Greet"), Some(SymbolKind::Trait));
}
#[test] fn rust_const_type_module() {
    let s = symbols(b"const MAX:u32=0; type A=u8; mod m{}", "rs");
    assert_eq!(find(&s,"MAX"), Some(SymbolKind::Const));
    assert_eq!(find(&s,"A"),   Some(SymbolKind::TypeAlias));
    assert_eq!(find(&s,"m"),   Some(SymbolKind::Module));
}
#[test] fn rust_calls_edge() { assert!(has_calls(b"fn a(){} fn b(){ a(); }", "rs")); }

// ── PYTHON ────────────────────────────────────────────────────────
#[test] fn python_async_def() {
    assert_eq!(find(&symbols(b"async def fetch(url):\n pass\n","py"),"fetch"), Some(SymbolKind::Function));
}
#[test] fn python_decorated_fn() {
    let s = symbols(b"@staticmethod\ndef helper():\n pass\n","py");
    assert_eq!(find(&s,"helper"), Some(SymbolKind::Function));
}
#[test] fn python_class_method() {
    let s = symbols(b"class Foo:\n    @classmethod\n    def create(cls): pass\n","py");
    assert!(find(&s,"create").is_some());
    assert_eq!(find(&s,"Foo"), Some(SymbolKind::Class));
}
#[test] fn python_calls_edge() { assert!(has_calls(b"def a(): pass\ndef b():\n a()\n","py")); }

// ── TYPESCRIPT ────────────────────────────────────────────────────
#[test] fn ts_arrow_const() {
    assert_eq!(find(&symbols(b"const f = (x:number):number => x;","ts"),"f"), Some(SymbolKind::Function));
}
#[test] fn ts_async_arrow() {
    let s = symbols(b"const fetch = async (url:string):Promise<string> => { return ''; };","ts");
    assert_eq!(find(&s,"fetch"), Some(SymbolKind::Function));
}
#[test] fn ts_fn_expr_const() {
    assert_eq!(find(&symbols(b"const h = function(e:Event):void {};","ts"),"h"), Some(SymbolKind::Function));
}
#[test] fn ts_interface_type_enum() {
    let s = symbols(b"interface I{} type T=string; enum E{A}", "ts");
    assert_eq!(find(&s,"I"), Some(SymbolKind::Interface));
    assert_eq!(find(&s,"T"), Some(SymbolKind::TypeAlias));
    assert_eq!(find(&s,"E"), Some(SymbolKind::Enum));
}
#[test] fn ts_class_method() {
    let s = symbols(b"class Svc { async process(): Promise<void> {} }","ts");
    assert!(find(&s,"process").is_some());
}
#[test] fn ts_calls_edge() { assert!(has_calls(b"function a(){} function b(){ a(); }","ts")); }

// ── JAVASCRIPT ────────────────────────────────────────────────────
#[test] fn js_arrow_const() {
    assert_eq!(find(&symbols(b"const add = (a,b) => a+b;","js"),"add"), Some(SymbolKind::Function));
}
#[test] fn js_fn_expr_const() {
    assert_eq!(find(&symbols(b"const h = function(e){ return e; };","js"),"h"), Some(SymbolKind::Function));
}
#[test] fn js_var_fn_expr() {
    assert_eq!(find(&symbols(b"var old = function(){};","js"),"old"), Some(SymbolKind::Function));
}
#[test] fn js_class_method() {
    assert!(find(&symbols(b"class Api { async get(url){} }","js"),"get").is_some());
}
#[test] fn js_calls_edge() { assert!(has_calls(b"function a(){} function b(){ a(); }","js")); }

// ── GO ────────────────────────────────────────────────────────────
#[test] fn go_struct_kind() {
    let s = symbols(b"package p\ntype Point struct { X,Y int }","go");
    assert_eq!(find(&s,"Point"), Some(SymbolKind::Struct), "struct must be Struct, not TypeAlias");
}
#[test] fn go_interface_kind() {
    let s = symbols(b"package p\ntype R interface { Read() int }","go");
    assert_eq!(find(&s,"R"), Some(SymbolKind::Interface));
}
#[test] fn go_named_type() {
    let s = symbols(b"package p\ntype Status int","go");
    assert_eq!(find(&s,"Status"), Some(SymbolKind::TypeAlias));
}
#[test] fn go_method() {
    let s = symbols(b"package p\ntype F struct{}\nfunc (f F) Bar(){}","go");
    assert_eq!(find(&s,"Bar"), Some(SymbolKind::Method));
}
#[test] fn go_calls_edge() { assert!(has_calls(b"package p\nfunc a(){}\nfunc b(){ a() }","go")); }

// ── JAVA ──────────────────────────────────────────────────────────
#[test] fn java_record() {
    assert_eq!(find(&symbols(b"public record Point(int x,int y){}","java"),"Point"), Some(SymbolKind::Struct));
}
#[test] fn java_annotation_type() {
    let s = symbols(b"public @interface Tx { boolean ro() default false; }","java");
    assert_eq!(find(&s,"Tx"), Some(SymbolKind::Interface));
}
#[test] fn java_interface_enum() {
    let s = symbols(b"public interface R{} public enum S{A,B}","java");
    assert_eq!(find(&s,"R"), Some(SymbolKind::Interface));
    assert_eq!(find(&s,"S"), Some(SymbolKind::Enum));
}
#[test] fn java_calls_edge() { assert!(has_calls(b"class A{ void a(){} void b(){ a(); } }","java")); }

// ── KOTLIN ────────────────────────────────────────────────────────
// Re-enabled via tree-sitter-kotlin-ng (a maintained fork compatible with
// tree-sitter 0.24).
#[test] fn kotlin_interface_as_class() {
    let s = symbols(b"interface Printable { fun print() }","kt");
    assert_eq!(find(&s,"Printable"), Some(SymbolKind::Interface));
}
#[test] fn kotlin_enum_entries() {
    let s = symbols(b"enum class Color { RED, GREEN }","kt");
    assert!(find(&s,"RED").is_some() || find(&s,"GREEN").is_some());
}
#[test] fn kotlin_object() {
    assert_eq!(find(&symbols(b"object S { fun i() = this }","kt"),"S"), Some(SymbolKind::Module));
}
#[test] fn kotlin_calls_edge_fixed() {
    assert!(has_calls(b"fun a(){}\nfun b(){ a() }","kt"));
}
#[test] fn kotlin_function_and_class() {
    let s = symbols(b"class Greeter {\n    fun greet() {}\n}","kt");
    assert_eq!(find(&s,"Greeter"), Some(SymbolKind::Class));
    assert_eq!(find(&s,"greet"), Some(SymbolKind::Function));
}

// ── SWIFT ─────────────────────────────────────────────────────────
#[test] fn swift_protocol_trait() {
    assert_eq!(find(&symbols(b"protocol Greetable { func greet() -> String }","swift"),"Greetable"),
        Some(SymbolKind::Trait));
}
#[test] fn swift_struct_correct_kind() {
    // GRAMMAR LIMITATION: tree-sitter-swift uses class_declaration for both class and struct
    let s = symbols(b"struct Point { var x: Double }","swift");
    assert_eq!(find(&s,"Point"), Some(SymbolKind::Struct), "Swift struct must be Struct kind");
}
#[test] fn swift_enum_function() {
    let s = symbols(b"enum Dir { case up }\nfunc greet(n:String)->String{\"hi\"}","swift");
    assert_eq!(find(&s,"Dir"), Some(SymbolKind::Enum), "Swift enum must now be Enum kind");
    assert_eq!(find(&s,"greet"), Some(SymbolKind::Function));
}
#[test] fn swift_calls_edge() {
    assert!(has_calls(b"func a()->Int{0}\nfunc b()->Int{a()}","swift"));
}

// ── C ────────────────────────────────────────────────────────────
#[test] fn c_fn_struct_enum_typedef() {
    let s = symbols(b"int add(int a,int b){return a+b;}\nstruct P{int x;};\nenum C{R,G};\ntypedef int D;","c");
    assert_eq!(find(&s,"add"), Some(SymbolKind::Function));
    assert_eq!(find(&s,"P"),   Some(SymbolKind::Struct));
    assert_eq!(find(&s,"C"),   Some(SymbolKind::Enum));
    assert_eq!(find(&s,"D"),   Some(SymbolKind::TypeAlias));
}
#[test] fn c_calls_edge() { assert!(has_calls(b"int a(){return 1;}\nint b(){return a();}","c")); }

// ── C++ ──────────────────────────────────────────────────────────
#[test] fn cpp_namespace_class_struct_enum() {
    let s = symbols(b"namespace ns{ class Foo{}; struct Bar{}; enum Kind{A,B}; }","cpp");
    assert_eq!(find(&s,"ns"),   Some(SymbolKind::Module));
    assert_eq!(find(&s,"Foo"),  Some(SymbolKind::Class));
    assert_eq!(find(&s,"Bar"),  Some(SymbolKind::Struct));
    assert_eq!(find(&s,"Kind"), Some(SymbolKind::Enum));
}
#[test] fn cpp_calls_edge() { assert!(has_calls(b"int a(){return 0;}\nint b(){return a();}","cpp")); }

// ── C# ───────────────────────────────────────────────────────────
#[test] fn csharp_all_types() {
    let s = symbols(b"namespace App{ interface I{} class C{} struct S{} enum E{A} }","cs");
    assert_eq!(find(&s,"I"), Some(SymbolKind::Interface));
    assert_eq!(find(&s,"C"), Some(SymbolKind::Class));
    assert_eq!(find(&s,"S"), Some(SymbolKind::Struct));
    assert_eq!(find(&s,"E"), Some(SymbolKind::Enum));
}
#[test] fn csharp_simple_namespace() {
    assert_eq!(find(&symbols(b"namespace Foo{ class X{} }","cs"),"Foo"), Some(SymbolKind::Module));
}
#[test] fn csharp_qualified_namespace() {
    assert_eq!(find(&symbols(b"namespace Foo.Bar{ class X{} }","cs"),"Foo.Bar"), Some(SymbolKind::Module));
}
#[test] fn csharp_calls_edge() { assert!(has_calls(b"class A{ void a(){} void b(){ a(); } }","cs")); }

// ── RUBY ─────────────────────────────────────────────────────────
#[test] fn ruby_module_class_method() {
    let s = symbols(b"module Auth\n class M\n  def auth(t)\n   true\n  end\n end\nend\n","rb");
    assert_eq!(find(&s,"Auth"), Some(SymbolKind::Module));
    assert_eq!(find(&s,"M"),    Some(SymbolKind::Class));
    assert!(find(&s,"auth").is_some());
}
#[test] fn ruby_singleton_method() {
    let s = symbols(b"class Foo\n def self.create\n  new\n end\nend\n","rb");
    assert_eq!(find(&s,"create"), Some(SymbolKind::Method));
}

// ── SCALA ────────────────────────────────────────────────────────
#[test] fn scala_core_types() {
    let s = symbols(b"trait G{def h():String}\ncase class P(x:Int)\nobject C{}\ndef f()=1","scala");
    assert_eq!(find(&s,"G"), Some(SymbolKind::Trait));
    assert_eq!(find(&s,"P"), Some(SymbolKind::Class));
    assert_eq!(find(&s,"C"), Some(SymbolKind::Module));
    assert_eq!(find(&s,"f"), Some(SymbolKind::Function));
}
#[test] fn scala_calls_edge() { assert!(has_calls(b"def a():Int=1\ndef b():Int=a()","scala")); }

// ── PHP ──────────────────────────────────────────────────────────
#[test] fn php_function_class_method() {
    let s = symbols(b"<?php\nfunction greet(string $n): string { return \"Hi\"; }\nclass U {\n public function find(int $id): ?User { return null; }\n}","php");
    assert_eq!(find(&s,"greet"), Some(SymbolKind::Function));
    assert_eq!(find(&s,"U"),     Some(SymbolKind::Class));
    assert!(find(&s,"find").is_some());
}
#[test] fn php_interface_trait() {
    let s = symbols(b"<?php\ninterface S{}\ntrait L{}","php");
    assert_eq!(find(&s,"S"), Some(SymbolKind::Interface));
    assert_eq!(find(&s,"L"), Some(SymbolKind::Trait));
}
#[test] fn php_enum() {
    let s = symbols(b"<?php\nenum Status: string { case Active = 'active'; }","php");
    assert_eq!(find(&s,"Status"), Some(SymbolKind::Enum));
}
#[test] fn php_calls_edge() { assert!(has_calls(b"<?php\nfunction a(){}\nfunction b(){ a(); }","php")); }

// ── LUA ──────────────────────────────────────────────────────────
// tree-sitter-lua pinned to =0.2.0, not the newer 0.5.0 (ABI 15, incompatible
// with our tree-sitter 0.24 runtime).
#[test] fn lua_global_and_local_function() {
    let s = symbols(b"function hello()\nend\nlocal function greet(name)\nend","lua");
    assert_eq!(find(&s,"hello"), Some(SymbolKind::Function));
    assert_eq!(find(&s,"greet"), Some(SymbolKind::Function));
}
#[test] fn lua_calls_edge() {
    assert!(has_calls(b"local function a() end\nfunction b() a() end","lua"));
}
#[test] fn lua_require_import_edge() {
    // `extract_edges` attaches an import edge to its enclosing symbol
    // (falling back to the file's first symbol) — a source with no symbols
    // at all has nothing to attach the edge to, so include a function.
    let r = ex().extract(Path::new("t.lua"), b"local m = require(\"mymodule\")\nfunction f() end").unwrap();
    assert!(
        r.edges.iter().any(|e| e.kind == EdgeKind::Imports && e.to_name.as_deref() == Some("mymodule")),
        "expected an Imports edge to \"mymodule\", got: {:?}", r.edges,
    );
}

// ── BASH (Phase A) ───────────────────────────────────────────────
#[test] fn bash_function() {
    assert_eq!(find(&symbols(b"function foo() {\n echo hi\n}\n","sh"),"foo"), Some(SymbolKind::Function));
}
#[test] fn bash_calls_edge() {
    assert!(has_calls(b"function a() { echo hi\n}\nfunction b() { a\n}\n","sh"));
}
#[test] fn bash_source_import_edge() {
    let r = ex().extract(Path::new("t.sh"), b"function f() {\n echo hi\n}\nsource lib.sh\n").unwrap();
    assert!(
        r.edges.iter().any(|e| e.kind == EdgeKind::Imports && e.to_name.as_deref() == Some("lib.sh")),
        "expected an Imports edge to \"lib.sh\", got: {:?}", r.edges,
    );
}

// ── ZIG (Phase A) ────────────────────────────────────────────────
#[test] fn zig_function_and_struct() {
    let s = symbols(b"const Foo = struct {\n x: i32,\n};\nfn bar(a: i32) i32 {\n return a;\n}\n","zig");
    assert_eq!(find(&s,"bar"), Some(SymbolKind::Function));
    assert_eq!(find(&s,"Foo"), Some(SymbolKind::Struct));
}
#[test] fn zig_calls_edge() {
    assert!(has_calls(b"fn a() void {}\nfn b() void {\n a();\n}\n","zig"));
}

// ── ELIXIR (Phase A) ─────────────────────────────────────────────
#[test] fn elixir_module_and_functions() {
    let s = symbols(b"defmodule Foo do\n def bar(x) do\n  x\n end\n defp baz(x), do: x\nend\n","ex");
    assert_eq!(find(&s,"Foo"), Some(SymbolKind::Module));
    assert_eq!(find(&s,"bar"), Some(SymbolKind::Function));
    assert_eq!(find(&s,"baz"), Some(SymbolKind::Function));
}
#[test] fn elixir_calls_edge() {
    assert!(has_calls(b"defmodule Foo do\n def a(x) do\n  b(x)\n end\n def b(x), do: x\nend\n","ex"));
}

// ── JULIA (Phase A) ──────────────────────────────────────────────
#[test] fn julia_function_struct_module() {
    let s = symbols(b"module Foo\nstruct Bar\n x::Int\nend\nfunction baz(x)\n x\nend\nend\n","jl");
    assert_eq!(find(&s,"Foo"), Some(SymbolKind::Module));
    assert_eq!(find(&s,"Bar"), Some(SymbolKind::Struct));
    assert_eq!(find(&s,"baz"), Some(SymbolKind::Function));
}
#[test] fn julia_calls_edge() {
    assert!(has_calls(b"function a(x)\n b(x)\nend\nfunction b(x)\n x\nend\n","jl"));
}

// ── FORTRAN (Phase A) ────────────────────────────────────────────
#[test] fn fortran_module_subroutine_function() {
    let s = symbols(
        b"module foo\ncontains\nsubroutine bar(x)\n integer :: x\n call baz(x)\nend subroutine bar\nfunction qux(x) result(y)\n integer :: x, y\n y = x\nend function qux\nend module foo\n",
        "f90",
    );
    assert_eq!(find(&s,"foo"), Some(SymbolKind::Module));
    assert_eq!(find(&s,"bar"), Some(SymbolKind::Function));
    assert_eq!(find(&s,"qux"), Some(SymbolKind::Function));
}
#[test] fn fortran_calls_edge() {
    assert!(has_calls(
        b"module m\ncontains\nsubroutine a()\n call b()\nend subroutine a\nsubroutine b()\nend subroutine b\nend module m\n",
        "f90",
    ));
}

// ── OCAML (Phase A) ──────────────────────────────────────────────
#[test] fn ocaml_function_record_module() {
    let s = symbols(
        b"let foo x = x + 1\ntype point = { x: int; y: int }\nmodule M = struct\n let baz x = x\nend\n",
        "ml",
    );
    assert_eq!(find(&s,"foo"), Some(SymbolKind::Function));
    assert_eq!(find(&s,"point"), Some(SymbolKind::Struct));
    assert_eq!(find(&s,"M"), Some(SymbolKind::Module));
}
#[test] fn ocaml_calls_edge() {
    assert!(has_calls(b"let bar x = x\nlet foo x = bar x\n","ml"));
}

// ── COMMON LISP (Phase A) ────────────────────────────────────────
#[test] fn commonlisp_defun() {
    assert_eq!(find(&symbols(b"(defun foo (x) (bar x))\n","lisp"),"foo"), Some(SymbolKind::Function));
}
#[test] fn commonlisp_calls_edge() {
    assert!(has_calls(b"(defun a (x) (b x))\n(defun b (x) x)\n","lisp"));
}

// ── VERILOG (Phase A) ────────────────────────────────────────────
#[test] fn verilog_module() {
    assert_eq!(find(&symbols(b"module foo(input a);\nendmodule\n","v"),"foo"), Some(SymbolKind::Module));
}
#[test] fn verilog_instantiation_calls_edge() {
    // Instantiator (`foo`) must precede the instantiated module (`bar`) in
    // file order — see the WHY comment on `VerilogLang::call_query`.
    assert!(has_calls(
        b"module foo(input a, output b);\n bar baz_inst(.a(a), .b(b));\nendmodule\nmodule bar(input a, output b);\nendmodule\n",
        "v",
    ));
}

// ── PASCAL (Phase A) ─────────────────────────────────────────────
#[test] fn pascal_procedure_and_function() {
    let s = symbols(
        b"program Foo;\nprocedure Bar(x: Integer);\nbegin\n  Baz(x);\nend;\nfunction Qux(x: Integer): Integer;\nbegin\n  Qux := x;\nend;\nbegin\nend.\n",
        "pas",
    );
    assert_eq!(find(&s,"Bar"), Some(SymbolKind::Function));
    assert_eq!(find(&s,"Qux"), Some(SymbolKind::Function));
}
#[test] fn pascal_calls_edge() {
    assert!(has_calls(
        b"program P;\nprocedure Bar(x: Integer);\nbegin\n  Baz(x);\nend;\nbegin\n  Bar(1);\nend.\n",
        "pas",
    ));
}

// ── DART (Phase A) ───────────────────────────────────────────────
#[test] fn dart_class_and_method() {
    let s = symbols(b"class Foo {\n  int bar(int x) {\n    return x;\n  }\n}\n","dart");
    assert_eq!(find(&s,"Foo"), Some(SymbolKind::Class));
    assert_eq!(find(&s,"bar"), Some(SymbolKind::Method));
}
#[test] fn dart_calls_edge() {
    assert!(has_calls(
        b"class Foo {\n  int bar(int x) {\n    return baz(x);\n  }\n  int baz(int x) {\n    return x;\n  }\n}\n",
        "dart",
    ));
}

// ── OBJECTIVE-C (Phase A) ────────────────────────────────────────
#[test] fn objc_class_and_method() {
    let s = symbols(
        b"@interface Foo : NSObject\n- (int)bar:(int)x;\n@end\n@implementation Foo\n- (int)bar:(int)x {\n  return x;\n}\n@end\n",
        "m",
    );
    assert_eq!(find(&s,"Foo"), Some(SymbolKind::Class));
    assert_eq!(find(&s,"bar"), Some(SymbolKind::Method));
}
#[test] fn objc_calls_edge() {
    assert!(has_calls(
        b"@implementation Foo\n- (int)bar:(int)x {\n  return [self baz:x];\n}\n- (int)baz:(int)x {\n  return x;\n}\n@end\n",
        "m",
    ));
}

// ── GROOVY (Phase A) ─────────────────────────────────────────────
#[test] fn groovy_class_and_method() {
    let s = symbols(b"class Foo {\n  def bar(String x) {\n    baz(x)\n  }\n}\n","groovy");
    assert_eq!(find(&s,"Foo"), Some(SymbolKind::Class));
    assert_eq!(find(&s,"bar"), Some(SymbolKind::Method));
}
#[test] fn groovy_top_level_function() {
    assert_eq!(find(&symbols(b"def baz(String x) { x }\n","groovy"),"baz"), Some(SymbolKind::Function));
}
#[test] fn groovy_calls_edge() {
    assert!(has_calls(b"class Foo {\n  def bar(String x) {\n    baz(x)\n  }\n}\n","groovy"));
}

// ── POWERSHELL (Phase A) ─────────────────────────────────────────
#[test] fn powershell_function() {
    assert_eq!(find(&symbols(b"function Bar {\n Write-Output \"hi\"\n}\n","ps1"),"Bar"), Some(SymbolKind::Function));
}
#[test] fn powershell_calls_edge() {
    assert!(has_calls(b"function Bar {\n Baz\n}\nfunction Baz {\n \"hi\"\n}\n","ps1"));
}

// ── JSON (Phase C) ───────────────────────────────────────────────
#[test] fn json_top_level_keys_only() {
    let s = symbols(br#"{
  "name": "Sales Order",
  "fields": [
    {"fieldname": "order_type", "nested": {"deep": "value"}}
  ],
  "actions": []
}"#, "json");
    assert_eq!(find(&s,"name"), Some(SymbolKind::Const));
    assert_eq!(find(&s,"fields"), Some(SymbolKind::Const));
    assert_eq!(find(&s,"actions"), Some(SymbolKind::Const));
    // Nested keys must NOT become symbols — only the top level.
    assert_eq!(find(&s,"fieldname"), None);
    assert_eq!(find(&s,"nested"), None);
    assert_eq!(find(&s,"deep"), None);
}
#[test] fn json_array_root_has_no_symbols() {
    let s = symbols(br#"[{"a": 1}, {"b": 2}]"#, "json");
    assert!(s.is_empty(), "an array-rooted JSON file has no top-level object keys");
}
#[test] fn json_no_calls_or_imports() {
    let r = ex().extract(Path::new("t.json"), br#"{"a": 1, "b": 2}"#).unwrap();
    assert!(r.edges.is_empty(), "JSON has no call/import semantics");
}

// ── INHERITS EDGES (EdgeKind::Inherits, see extractor.rs's inherits_query wiring) ──
#[test] fn python_inherits_edge() {
    assert_eq!(inherits_targets(b"class Foo(Bar):\n pass\n","py","Foo"), vec!["Bar"]);
}
#[test] fn ts_inherits_edge() {
    let t = inherits_targets(b"class Foo extends Bar implements Baz {}","ts","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn js_inherits_edge() {
    assert_eq!(inherits_targets(b"class Foo extends Bar {}","js","Foo"), vec!["Bar"]);
}
#[test] fn java_inherits_edge() {
    let t = inherits_targets(b"class Foo extends Bar implements Baz {}","java","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn kotlin_inherits_edge() {
    let t = inherits_targets(b"class Foo : Bar(), Baz {}","kt","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn swift_inherits_edge() {
    let t = inherits_targets(b"class Foo: Bar, Baz {}","swift","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn cpp_inherits_edge() {
    assert_eq!(inherits_targets(b"class Foo : public Bar {};","cpp","Foo"), vec!["Bar"]);
}
#[test] fn csharp_inherits_edge() {
    let t = inherits_targets(b"class Foo : Bar, Baz {}","cs","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn ruby_inherits_edge() {
    assert_eq!(inherits_targets(b"class Foo < Bar\nend\n","rb","Foo"), vec!["Bar"]);
}
#[test] fn php_inherits_edge() {
    let t = inherits_targets(b"<?php\nclass Foo extends Bar implements Baz {}","php","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn scala_inherits_edge() {
    assert_eq!(inherits_targets(b"class Foo extends Bar {}","scala","Foo"), vec!["Bar"]);
}
#[test] fn dart_inherits_edge() {
    let t = inherits_targets(b"class Foo extends Bar implements Baz {}","dart","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn groovy_inherits_edge() {
    let t = inherits_targets(b"class Foo extends Bar implements Baz {}","groovy","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}

// ── INHERITS EDGES, MODULE/NAMESPACE-QUALIFIED BASE (e.g. `frappe.ValidationError`) ──
// Regression coverage for a real gap found on the erpnext corpus: every language's
// first-pass inherits_query only matched bare identifiers, silently missing the
// (very common in real code) qualified form.
#[test] fn python_inherits_edge_qualified() {
    assert_eq!(inherits_targets(b"class Foo(pkg.Bar):\n pass\n","py","Foo"), vec!["Bar"]);
}
#[test] fn ts_inherits_edge_qualified() {
    let t = inherits_targets(b"class Foo extends ns.Bar implements ns.Baz {}","ts","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn js_inherits_edge_qualified() {
    assert_eq!(inherits_targets(b"class Foo extends ns.Bar {}","js","Foo"), vec!["Bar"]);
}
#[test] fn java_inherits_edge_qualified() {
    let t = inherits_targets(b"class Foo extends pkg.Bar implements pkg.Baz {}","java","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn kotlin_inherits_edge_qualified() {
    let t = inherits_targets(b"class Foo : ns.Bar(), ns.Baz {}","kt","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn swift_inherits_edge_qualified() {
    assert_eq!(inherits_targets(b"class Foo: ns.Bar {}","swift","Foo"), vec!["Bar"]);
}
#[test] fn cpp_inherits_edge_qualified() {
    assert_eq!(inherits_targets(b"class Foo : public ns::Bar {};","cpp","Foo"), vec!["Bar"]);
}
#[test] fn csharp_inherits_edge_qualified() {
    assert_eq!(inherits_targets(b"class Foo : Ns.Bar {}","cs","Foo"), vec!["Bar"]);
}
#[test] fn ruby_inherits_edge_qualified() {
    assert_eq!(inherits_targets(b"class Foo < Ns::Bar\nend\n","rb","Foo"), vec!["Bar"]);
}
#[test] fn php_inherits_edge_qualified() {
    let t = inherits_targets(b"<?php\nclass Foo extends \\Ns\\Bar implements \\Ns\\Baz {}","php","Foo");
    assert!(t.contains(&"Bar".to_string()) && t.contains(&"Baz".to_string()));
}
#[test] fn scala_inherits_edge_qualified() {
    assert_eq!(inherits_targets(b"class Foo extends ns.Bar {}","scala","Foo"), vec!["Bar"]);
}
#[test] fn dart_inherits_edge_qualified() {
    assert_eq!(inherits_targets(b"class Foo extends ns.Bar {}","dart","Foo"), vec!["Bar"]);
}
#[test] fn groovy_inherits_edge_qualified() {
    assert_eq!(inherits_targets(b"class Foo extends ns.Bar {}","groovy","Foo"), vec!["Bar"]);
}

// ── REFERENCES EDGES (EdgeKind::References, type annotations only) ──
#[test] fn python_references_edge() {
    let t = references_targets(b"def foo(acct: Account) -> RootNotEditable:\n pass\n","py","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn python_references_edge_qualified() {
    assert_eq!(references_targets(b"def foo(x: pkg.Bar):\n pass\n","py","foo"), vec!["Bar"]);
}
#[test] fn ts_references_edge() {
    let t = references_targets(b"function foo(acct: Account): RootNotEditable {}","ts","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn java_references_edge() {
    let t = references_targets(b"class C { RootNotEditable foo(Account acct) {} }","java","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn csharp_references_edge() {
    let t = references_targets(b"class C { RootNotEditable Foo(Account acct) {} }","cs","Foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn cpp_references_edge() {
    let t = references_targets(b"RootNotEditable foo(Account acct) { return x; }","cpp","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn go_references_edge() {
    let t = references_targets(b"func Foo(acct Account) RootNotEditable { }","go","Foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn rust_references_edge() {
    let t = references_targets(b"fn foo(acct: Account) -> RootNotEditable { }","rs","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn kotlin_references_edge() {
    let t = references_targets(b"fun foo(acct: Account): RootNotEditable { }","kt","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn swift_references_edge() {
    let t = references_targets(b"func foo(acct: Account) -> RootNotEditable { }","swift","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn php_references_edge() {
    let t = references_targets(b"<?php\nfunction foo(Account $acct): RootNotEditable { }","php","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn scala_references_edge() {
    let t = references_targets(b"def foo(acct: Account): RootNotEditable = { }","scala","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn dart_references_edge() {
    let t = references_targets(b"class C { RootNotEditable foo(Account acct) { } }","dart","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}
#[test] fn groovy_references_edge() {
    let t = references_targets(b"class C { RootNotEditable foo(Account acct) { } }","groovy","foo");
    assert!(t.contains(&"Account".to_string()) && t.contains(&"RootNotEditable".to_string()));
}

// ── IMPLEMENTS EDGES (EdgeKind::Implements — Rust's `impl Trait for Type`) ──
#[test] fn rust_implements_edge() {
    assert_eq!(
        implements_targets(b"struct Foo;\nimpl Bar for Foo {}\n","rs","Foo"),
        vec!["Bar"],
    );
}
#[test] fn rust_implements_edge_generic_type() {
    assert_eq!(
        implements_targets(b"struct Foo<T>(T);\nimpl Bar for Foo<i32> {}\n","rs","Foo"),
        vec!["Bar"],
    );
}
#[test] fn rust_inherent_impl_is_not_an_implements_edge() {
    // `impl Foo { }` (no trait) has no `trait:` field — must not match.
    assert!(implements_targets(b"struct Foo;\nimpl Foo {\n fn bar(&self) {}\n}\n","rs","Foo").is_empty());
}
#[test] fn rust_implements_edge_qualified_trait() {
    // `impl fmt::Display for Foo` — the majority shape in real code, not
    // the bare-identifier minority case the first draft only handled.
    assert_eq!(
        implements_targets(b"struct Foo;\nimpl fmt::Display for Foo {}\n","rs","Foo"),
        vec!["Display"],
    );
}
#[test] fn rust_implements_edge_deeply_qualified_trait() {
    assert_eq!(
        implements_targets(b"struct Foo;\nimpl std::error::Error for Foo {}\n","rs","Foo"),
        vec!["Error"],
    );
}
#[test] fn rust_implements_edge_generic_trait() {
    // `impl From<Bar> for Foo` — the trait itself is generic, not the type.
    assert_eq!(
        implements_targets(b"struct Foo;\nimpl From<Bar> for Foo {}\n","rs","Foo"),
        vec!["From"],
    );
}
#[test] fn rust_implements_edge_generic_trait_and_generic_type() {
    assert_eq!(
        implements_targets(b"struct Foo<T>(T);\nimpl PartialEq<Other> for Foo<i32> {}\n","rs","Foo"),
        vec!["PartialEq"],
    );
}
#[test] fn rust_implements_edge_resolves_to_id_for_a_locally_defined_trait() {
    // When the trait is defined in the same file, to_id resolves locally
    // and to_name becomes None (same convention as Inherits/References) —
    // implements_targets (which reads to_name) reports it as absent, but
    // the edge itself must still exist and correctly resolve.
    let r = ex().extract(Path::new("t.rs"), b"trait Bar {}\nstruct Foo;\nimpl Bar for Foo {}\n").unwrap();
    let foo_id = r.symbols.iter().find(|s| s.name == "Foo").unwrap().id;
    let bar_id = r.symbols.iter().find(|s| s.name == "Bar").unwrap().id;
    let edge = r.edges.iter().find(|e| e.kind == EdgeKind::Implements && e.from_id == foo_id);
    assert_eq!(edge.and_then(|e| e.to_id), Some(bar_id));
}
