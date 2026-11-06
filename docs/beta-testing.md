# Beta Testing Guide

Thanks for testing GraphRelay. This guide covers what to test, what "working correctly" looks like, and how to report what you find.

---

## What GraphRelay does

GraphRelay indexes your codebase and gives Claude Code — or any MCP-compatible agent — graph-aware semantic search. Instead of reading files one by one, the agent calls `search_code(intent: "how does X work")` and gets back ranked symbols with source snippets and call relationships, in one round trip, using roughly 30× fewer tokens than grepping around would.

---

## Setup checklist

Before you start testing, confirm each of these:

- [ ] Installed via `curl -fsSL https://raw.githubusercontent.com/openwt-vn/graphrelay/main/scripts/install.sh | sh`
- [ ] `gr --help` runs without error
- [ ] `gr init` completes (`.graphrelay/` created)
- [ ] `gr start` shows the daemon running (nothing is downloaded)
- [ ] `gr status` shows your project's state as `fresh` with non-zero file/symbol counts
- [ ] `claude mcp list` shows your project connected with 4 tools
- [ ] You've added the GraphRelay hook to `CLAUDE.md` — the fastest way is `gr setup`; see [claude-code-hook.md](claude-code-hook.md) for the manual version

---

## What to test

### 1. Basic search quality

Ask Claude Code questions about your codebase that genuinely require finding code, not questions it could answer from memory alone:

```
How does [some feature] work in this project?
Where is [some function] called from?
What breaks if I change [some interface]?
Show me all places where [some pattern] is used.
```

**Good:** Claude reaches for `search_code` or `inspect` first, comes back with real code, file paths, and line numbers, and the answer is actually correct without it having read every file in the repo along the way.

**Worth flagging:** Claude reads raw files instead of using GraphRelay, or the results it gets back are wrong or irrelevant.

### 2. Incremental indexing (the file watcher)

1. With the daemon running, edit a file in your project — add a function, rename something.
2. Wait a second or two.
3. Ask Claude about the change.

**Good:** the answer reflects your edit without you having to restart the daemon.

**Worth flagging:** the edit isn't picked up, or the daemon crashes after a file change.

### 3. Multi-language projects

If your project mixes languages — TypeScript + Python, Go + TypeScript, whatever — try queries that cross that boundary:

```
What Python functions are called from the TypeScript frontend?
Where is the API contract defined?
```

### 4. Large-repo performance

For a repo with 50k+ lines, time how long it takes to go from `gr start` to `state: fresh`:

```sh
time (gr start && sleep 5 && gr status)
```

Under 2 minutes for a 100k-line repo's first index is what we're aiming for. Anything past 5 minutes, or a crash mid-index, is worth reporting.

### 5. Stress-testing the daemon

Leave the daemon running overnight with the file watcher active, then check in on it the next morning:

```sh
gr status
cat ~/.graphrelay/daemon.log | tail -50
```

Flag it if the daemon has crashed or memory usage has grown without bound.

---

## Known gaps — no need to report these

- **macOS Intel (x86_64)** has no prebuilt release binary (releases cover macOS Apple Silicon, Linux x86_64 and Linux ARM64); build from source.
- **`gr index` while the daemon is running** errors out by design — stop the daemon first.
- **The SCIP semantic layer** (optional deeper indexing via `rust-analyzer`, `scip-python`, and similar tools) exists but isn't documented for general use yet — skip it for now.

---

## Collecting logs for a bug report

```sh
# Full daemon log
cat ~/.graphrelay/daemon.log

# Just the tail, with timestamps
tail -100 ~/.graphrelay/daemon.log

# Verbose logging — restart the daemon with it on, then reproduce the issue
gr stop
RUST_LOG=debug gr start
# ...reproduce...
cat ~/.graphrelay/daemon.log
```

And your system info:
```sh
gr --version 2>/dev/null || gr --help | head -1
uname -a
```

---

## Filing an issue

Open one at **https://github.com/openwt-vn/graphrelay/issues** using the bug report template. The reports that are easiest to act on include:

1. What you were trying to do
2. What actually happened (the error message, or the wrong output)
3. Steps to reproduce it
4. The daemon log (`~/.graphrelay/daemon.log`)
5. The output of `gr status`
6. Your OS and graphrelay version

---

## What we're especially interested in hearing about

- **Search quality** — did GraphRelay find the right code? Was the ranking sensible?
- **Token savings** — did using it actually cut down the number of file reads Claude needed?
- **Setup friction** — was anything confusing or broken in install/init/start?
- **Performance** — how long did indexing take, and how large was your project (files/LOC)?
- **Missing languages** — what did you need that isn't supported yet?

General feedback fits best as a GitHub Discussion; concrete bugs belong in an Issue.

---

## Quick reference

| Command | Description |
|---|---|
| `gr status` | Check daemon + project state |
| `gr restart` | Restart after a config change |
| `gr stop && gr index && gr start` | Force a full re-index |
| `cat ~/.graphrelay/daemon.log` | Daemon logs |
| `RUST_LOG=debug gr start` | Verbose logging |
| `claude mcp list` | Verify the MCP connection |
