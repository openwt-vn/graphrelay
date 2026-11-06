// gokeys emits a compiler-checked static call graph of a Go module as JSON, for building structural
// benchmark keys independent of GraphRelay. Usage: go run . <module root> > graph.json
// Only direct static calls are edges. Per function it also records value references (a function used
// but not called, e.g. a route handler) and calls made from package-level initialisers; the
// generator treats either as a reason to exclude a target, because those paths are invisible here.
package main

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/types"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"golang.org/x/tools/go/packages"
)

type Fn struct {
	Key        string   `json:"key"`
	File       string   `json:"file"`
	Name       string   `json:"name"`
	IsMethod   bool     `json:"is_method"`
	Generic    bool     `json:"generic"`
	Callers    []string `json:"callers"`
	CallSites  int      `json:"call_sites"`
	ValueRefs  int      `json:"value_refs"`
	InitCalled bool     `json:"init_called"`
}

func main() {
	root, _ := filepath.Abs(os.Args[1])
	cfg := &packages.Config{
		Mode: packages.NeedName | packages.NeedFiles | packages.NeedSyntax | packages.NeedTypes |
			packages.NeedTypesInfo | packages.NeedImports | packages.NeedDeps,
		Dir: root, Tests: false,
	}
	pkgs, err := packages.Load(cfg, "./...")
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fns := map[string]*Fn{}
	callers := map[string]map[string]bool{}
	rel := func(p string) (string, bool) {
		r, err := filepath.Rel(root, p)
		if err != nil || strings.HasPrefix(r, "..") || strings.HasSuffix(r, "_test.go") {
			return "", false
		}
		return filepath.ToSlash(r), true
	}
	keyOf := func(pkg *packages.Package, o *types.Func) (string, string, bool) {
		o = o.Origin()
		f, ok := rel(pkg.Fset.Position(o.Pos()).Filename)
		if !ok {
			return "", "", false
		}
		name := o.Name()
		if sig, ok := o.Type().(*types.Signature); ok && sig.Recv() != nil {
			t := sig.Recv().Type()
			if p, ok := t.(*types.Pointer); ok {
				t = p.Elem()
			}
			if n, ok := t.(*types.Named); ok {
				name = n.Obj().Name() + "." + name
			}
		}
		return f + "::" + name, f, true
	}
	get := func(k, f, name string, o *types.Func) *Fn {
		if fns[k] == nil {
			sig := o.Type().(*types.Signature)
			fns[k] = &Fn{Key: k, File: f, Name: name, IsMethod: sig.Recv() != nil, Generic: sig.TypeParams().Len() > 0 || (sig.Recv() != nil && sig.RecvTypeParams().Len() > 0)}
		}
		return fns[k]
	}
	seen := map[*packages.Package]bool{}
	for _, pkg := range pkgs {
		if seen[pkg] || pkg.TypesInfo == nil {
			continue
		}
		seen[pkg] = true
		info := pkg.TypesInfo
		for _, file := range pkg.Syntax {
			if _, ok := rel(pkg.Fset.Position(file.Pos()).Filename); !ok {
				continue
			}
			callIdents := map[*ast.Ident]bool{}
			addEdge := func(caller string, init bool, id *ast.Ident) {
				o, _ := info.Uses[id].(*types.Func)
				if o == nil {
					return
				}
				k, f, ok := keyOf(pkg, o)
				if !ok {
					return
				}
				callIdents[id] = true
				fn := get(k, f, k[strings.Index(k, "::")+2:], o.Origin())
				if sig, _ := o.Type().(*types.Signature); sig != nil && sig.Recv() != nil && types.IsInterface(sig.Recv().Type()) {
					fn.ValueRefs += 1000 // interface method: callers invisible, never eligible
				}
				if init {
					fn.InitCalled = true
					return
				}
				if caller == k {
					return
				}
				fn.CallSites++
				if callers[k] == nil {
					callers[k] = map[string]bool{}
				}
				callers[k][caller] = true
			}
			calls := func(root ast.Node, caller string, init bool) {
				ast.Inspect(root, func(n ast.Node) bool {
					c, ok := n.(*ast.CallExpr)
					if !ok {
						return true
					}
					switch f := ast.Unparen(c.Fun).(type) {
					case *ast.Ident:
						addEdge(caller, init, f)
					case *ast.SelectorExpr:
						addEdge(caller, init, f.Sel)
					}
					return true
				})
			}
			for _, d := range file.Decls {
				switch d := d.(type) {
				case *ast.FuncDecl:
					o, _ := info.Defs[d.Name].(*types.Func)
					if o == nil {
						continue
					}
					k, f, ok := keyOf(pkg, o)
					if !ok {
						continue
					}
					get(k, f, k[strings.Index(k, "::")+2:], o)
					if d.Body != nil {
						calls(d.Body, k, false)
					}
				case *ast.GenDecl:
					calls(d, "", true)
				}
			}
			ast.Inspect(file, func(n ast.Node) bool {
				id, ok := n.(*ast.Ident)
				if !ok || callIdents[id] {
					return true
				}
				if o, _ := info.Uses[id].(*types.Func); o != nil {
					if k, f, ok := keyOf(pkg, o); ok {
						get(k, f, k[strings.Index(k, "::")+2:], o.Origin()).ValueRefs++
					}
				}
				return true
			})
		}
	}
	out := make([]*Fn, 0, len(fns))
	for k, fn := range fns {
		for c := range callers[k] {
			fn.Callers = append(fn.Callers, c)
		}
		sort.Strings(fn.Callers)
		out = append(out, fn)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Key < out[j].Key })
	json.NewEncoder(os.Stdout).Encode(out)
}
