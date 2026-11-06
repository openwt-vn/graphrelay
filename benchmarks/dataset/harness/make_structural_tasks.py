#!/usr/bin/env python3
"""Generate the structural-slice tasks (tasks/structural_tasks.yaml) from the pinned ERPNext checkout.

Answer keys come from Python's `ast`, not from GraphRelay, so the test is not circular.
A target is eligible only when its name is unambiguous: defined exactly once in non-test code and
never mentioned inside a string literal or hooks.py (dispatch we cannot see statically would make the
key incomplete). Selection is a seeded shuffle, fixed before any tool-arm result exists.
Usage: make_structural_tasks.py <erpnext checkout> <out.yaml>
"""
import ast, json, random, sys
from collections import defaultdict
from pathlib import Path

SEED = 27
N_DIRECT, N_DEPTH2 = 6, 6
MIN_CALLERS, MAX_CALLERS = 3, 9

def py_files(root):
    for p in sorted((root / "erpnext").rglob("*.py")):
        s = p.relative_to(root).as_posix()
        if "/test" in s or s.rsplit("/", 1)[-1].startswith("test_") or "/patches/" in s or "/migrations/" in s:
            continue
        yield p, s

def collect(root):
    defs = defaultdict(list)       # name -> [(path, qual)]
    calls = defaultdict(set)       # callee simple name -> {(path, qual)}
    strings = defaultdict(int)     # identifier-looking string content -> count
    class V(ast.NodeVisitor):
        def __init__(s, path): s.path, s.stack = path, []
        def _fn(s, n):
            s.stack.append(n.name); defs[n.name].append((s.path, ".".join(s.stack))); s.generic_visit(n); s.stack.pop()
        visit_FunctionDef = visit_AsyncFunctionDef = _fn
        def visit_ClassDef(s, n): s.stack.append(n.name); s.generic_visit(n); s.stack.pop()
        def visit_Call(s, n):
            f = n.func
            name = f.id if isinstance(f, ast.Name) else f.attr if isinstance(f, ast.Attribute) else None
            if name and s.stack: calls[name].add((s.path, ".".join(s.stack)))
            s.generic_visit(n)
        def visit_Constant(s, n):
            if isinstance(n.value, str) and len(n.value) < 200:
                for tok in n.value.replace("/", ".").replace(" ", ".").split("."):
                    if tok.isidentifier(): strings[tok] += 1
    for p, rel in py_files(root):
        try: V(rel).visit(ast.parse(p.read_text(encoding="utf-8")))
        except SyntaxError: pass
    hooks = (root / "erpnext" / "hooks.py").read_text(encoding="utf-8")
    return defs, calls, strings, hooks

def fmt(path, qual): return f"{path}::{qual}"

def main():
    root, out = Path(sys.argv[1]), Path(sys.argv[2])
    defs, calls, strings, hooks = collect(root)
    def clean(name):
        return (len(defs.get(name, [])) == 1 and strings.get(name, 0) == 0 and name not in hooks
                and not name.startswith("__") and len(name) >= 8 and "_" in name)
    cand = []
    for name in sorted(defs):
        if not clean(name): continue
        own = defs[name][0]
        cs = {c for c in calls.get(name, ()) if c != own}
        if MIN_CALLERS <= len(cs) <= MAX_CALLERS: cand.append((name, own, sorted(cs)))
    rng = random.Random(SEED); rng.shuffle(cand)
    direct = cand[:N_DIRECT]; used = {c[0] for c in direct}
    depth2 = []
    for name, own, cs in cand[N_DIRECT:]:
        if name in used: continue
        # depth-2 keys are exact only if every depth-1 caller is itself unambiguous and unreferenced
        leaf = [c[1].split(".")[-1] for c in cs]
        if not all(clean(l) for l in leaf): continue
        d2 = set()
        for l in leaf:
            d2 |= {c for c in calls.get(l, ()) if c != defs[l][0]}
        d2 -= set(cs)
        if 2 <= len(d2) <= 12: depth2.append((name, own, cs, sorted(d2)))
        if len(depth2) == N_DEPTH2: break
    tasks = []
    for i, (name, own, cs) in enumerate(direct, 1):
        tasks.append({
            "id": f"S{i:02d}", "group": "S", "group_title": "Structural slice: callers / impact (ast-keyed)",
            "origin_note": f"direct callers of {name}",
            "question": (f"In the ERPNext codebase (excluding tests, patches and migrations), the function `{name}` "
                         f"is defined in {own[0]}. List every function or method that calls `{name}` directly "
                         f"(file path and function name for each). Be complete: if `{name}` changes its signature, "
                         f"these are the places that must be updated."),
            "key_facts": [f"`{c[1]}` in {c[0]} calls `{name}`." for c in cs],
            "anchor_files": sorted({own[0], *[c[0] for c in cs]}),
        })
    for j, (name, own, cs, d2) in enumerate(depth2, N_DIRECT + 1):
        tasks.append({
            "id": f"S{j:02d}", "group": "S", "group_title": "Structural slice: callers / impact (ast-keyed)",
            "origin_note": f"callers and callers-of-callers of {name}",
            "question": (f"In the ERPNext codebase (excluding tests, patches and migrations), the function `{name}` "
                         f"is defined in {own[0]}. If its behaviour changes, which functions are affected? List (1) every "
                         f"function that calls `{name}` directly and (2) every function that calls one of those "
                         f"direct callers (two hops out). Give file path and function name for each."),
            "key_facts": ([f"`{c[1]}` in {c[0]} calls `{name}` directly." for c in cs]
                          + [f"`{c[1]}` in {c[0]} is a second-hop caller (calls a direct caller of `{name}`)." for c in d2]),
            "anchor_files": sorted({own[0], *[c[0] for c in cs], *[c[0] for c in d2]}),
        })
    import yaml
    out.write_text("# GENERATED by harness/make_structural_tasks.py (seed %d) at ERPNext pin; do not hand-edit.\n" % SEED
                   + yaml.safe_dump({"tasks": tasks}, sort_keys=False, width=120, allow_unicode=True), encoding="utf-8")
    print(f"{len(cand)} eligible targets; wrote {len(tasks)} tasks ({len(direct)} direct, {len(depth2)} depth-2)")
    if len(tasks) < N_DIRECT + N_DEPTH2: print("WARNING: fewer depth-2 targets than requested", file=sys.stderr)

if __name__ == "__main__": main()
