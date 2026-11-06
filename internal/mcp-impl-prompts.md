# GraphRelay MCP — Implementation & Cleanup Test Prompts

10 prompts that require GraphRelay to guide actual code changes against the
Node.js source at `<workspace>/node`.
Each prompt tests a different capability: impact analysis, pattern matching,
dead-code detection, cross-file refactoring, and safe deletion.

---

## 1. Implement — per-EventEmitter warning threshold

```
In <workspace>/node, add a new option to
setMaxListeners() that accepts 0 to mean "unlimited" instead of requiring
Infinity. Before touching any file, use trace_impact on the current
setMaxListeners implementation to list every call site and test that would
be affected. Then make the change and update those call sites.
```

**Key capability:** Impact analysis before edit, multi-file coordinated change.
**What breaks without graph:** Blind edits miss call sites in test files.

---

## 2. Implement — fs.readFile() with timeout

```
Add a timeout option to fs.readFile() in the Node.js lib/ layer (no C++
changes needed). If the read takes longer than timeout ms, abort and
callback with an ETIMEDOUT error. First use inspect (path: lib/fs.js)
to understand the existing option-handling pattern, then implement it
consistently with how other fs options (like signal) are handled.
```

**Key capability:** Pattern matching against existing code, adding to existing option object.
**What breaks without graph:** Wrong option-handling style without context.

---

## 3. Implement — EventEmitter.onceWithTimeout()

```
Add EventEmitter.prototype.onceWithTimeout(event, timeout, listener) that
behaves like once() but calls listener(new Error('timeout')) if the event
does not fire within timeout ms. Use inspect (symbol) on the existing once()
implementation to understand its internals, then implement onceWithTimeout
without duplicating the listener-wrapping logic.
```

**Key capability:** Extends existing symbol, requires understanding internals to avoid duplication.
**What breaks without graph:** Reimplements instead of reuses existing logic.

---

## 4. Implement — require() load time logging

```
Add an opt-in environment variable NODE_PRINT_REQUIRE_TIME=1 that makes
require() print how long each module took to load (in ms) to stderr.
Use search_code (intent) to locate where module loading starts and ends
timing-wise. The output format should match existing NODE_DEBUG output style.
Find where NODE_DEBUG is parsed and follow the same pattern.
```

**Key capability:** Env-var feature pattern matching, timing instrumentation location.
**What breaks without graph:** Wrong hook-in point, off-by-one in timing boundary.

---

## 5. Implement — dns.lookup() result cache

```
Add an optional in-memory TTL cache to dns.lookup() controlled by a new
option { cache: true, ttl: 30000 }. Use inspect (path: lib/dns.js)
to understand how lookup() dispatches, then implement the cache as a Map
keyed by (hostname, family) with entry expiry. Show where to hook in the
cache check before the OS call and the cache write after.
```

**Key capability:** Adding a cache layer to an existing call chain, TTL logic placement.
**What breaks without graph:** Cache check inserted after OS call (wrong order).

---

## 6. Delete — legacy linkedlist module

```
The internal lib/internal/linkedlist.js (or _linklist) is reportedly
replaced by a more modern structure. Use trace_impact to find every
file that imports it. For each caller, determine if the module is still
actively used or if it was already replaced but the import left behind.
Remove dead imports and, if nothing uses the module, delete it entirely.
```

**Key capability:** Dead code detection, multi-file cleanup, verify nothing breaks.
**What breaks without graph:** Removes a module still in use elsewhere.

---

## 7. Clean up — duplicate error construction

```
Search the lib/ directory for places where new Error() or new TypeError()
is constructed with an identical message string in more than one location.
Find the top 3 most-duplicated error messages, refactor them into named
error factory functions in lib/internal/errors.js following the existing
pattern there, and replace all duplicate sites.
```

**Key capability:** Cross-file deduplication, existing pattern recognition in errors.js.
**What breaks without graph:** Misses duplicates in unvisited files.

---

## 8. Delete — unused helpers in fs/utils.js

```
In lib/internal/fs/utils.js, use trace_impact and search_code to
identify any helper functions that are defined but never called from outside
the file and not exported. List them, confirm they are dead, and remove them.
Then verify the file still passes its associated tests by checking what test
files import from that path.
```

**Key capability:** Dead code within a single file, export surface analysis.
**What breaks without graph:** Deletes a function that IS called externally.

---

## 9. Implement + Clean up — stream.pipeline() error aggregation

```
Currently stream.pipeline() stops on the first error. Add an option
{ aggregateErrors: true } that collects all errors from all pipeline
stages and rejects with an AggregateError. First use inspect (symbol) on
the current pipeline() error handling to understand the flow. Then:
(a) implement the option, and
(b) remove any now-redundant single-error early-exit paths that the new
    mode supersedes.
```

**Key capability:** Implement new behavior and remove old code paths simultaneously.
**What breaks without graph:** Leaves dead early-exit paths behind after implementing new behavior.

---

## 10. Delete — end-of-life deprecated APIs

```
Find all symbols in lib/ decorated with a DEP00xx deprecation notice that
are marked as "End-of-Life" (not just "Legacy" or "Documentation-only").
For each one:
(a) use trace_impact to check if any non-deprecated code still calls it,
(b) if truly unused by live code, delete the implementation and replace it
    with a throw that says the API was removed,
(c) list each deletion with its DEP number.
```

**Key capability:** Structured multi-step cleanup, deprecation metadata parsing, safe deletion with tombstones.
**What breaks without graph:** Deletes things still called by live code.

---

## Capability matrix

| # | Title | Primary stress | Without graph |
|---|---|---|---|
| 1 | setMaxListeners(0) | Impact-first workflow | Blind edits miss call sites in test files |
| 2 | fs.readFile timeout | Pattern extraction | Wrong option-handling style |
| 3 | onceWithTimeout | Internals before extending | Reimplements instead of reuses |
| 4 | require() timing | Find exact timing boundary | Wrong hook-in point |
| 5 | dns.lookup cache | Cache insertion point in call chain | Cache check after OS call |
| 6 | Legacy linkedlist | Dead import vs active import | Removes a module still in use |
| 7 | Duplicate errors | Cross-file duplicate detection | Misses duplicates in unvisited files |
| 8 | Dead fs helpers | Export surface vs call graph | Deletes a function called externally |
| 9 | pipeline AggregateError | Implement + simultaneously remove | Leaves dead early-exit paths behind |
| 10 | EOL deprecations | Metadata + impact + safe tombstone | Deletes things still called by live code |
