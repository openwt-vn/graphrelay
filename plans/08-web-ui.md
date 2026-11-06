# Plan 08 — Web UI (Embedded, Live, Local)

## Goal
Implement an interactive code graph UI served at `http://localhost:3030/` — embedded in the binary, requiring no separate server, no auth, and no data leaving the machine. It provides graph visualization (Cytoscape.js), semantic search, and symbol inspection.

## Success Criteria
- UI loads at `localhost:3030/` with no external network requests (all assets embedded)
- Graph renders up to 1,000 nodes interactively without lag (filtered views for larger repos)
- Search box queries `/api/search` and renders results in < 200 ms
- Symbol inspector shows definition, callers, callees, and source snippet
- Hop navigation: clicking a caller/callee loads that symbol
- Works in Chrome, Firefox, and Safari without extensions

---

## 1. Asset Embedding Strategy

Use `rust-embed` to bake all UI files into the binary at compile time:

```rust
// crates/codegraph-ui/src/lib.rs

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/"]
#[include = "*.html"]
#[include = "*.js"]
#[include = "*.css"]
pub struct Assets;

pub fn router() -> axum::Router {
    axum::Router::new()
        .route("/",          axum::routing::get(serve_index))
        .route("/assets/*path", axum::routing::get(serve_asset))
}

async fn serve_index() -> impl axum::response::IntoResponse {
    serve_file("index.html")
}

async fn serve_asset(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    serve_file(&path)
}

fn serve_file(path: &str) -> impl axum::response::IntoResponse {
    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            axum::response::Response::builder()
                .header("content-type", mime.as_ref())
                .header("cache-control", "no-cache")
                .body(axum::body::Body::from(content.data.to_vec()))
                .unwrap()
        }
        None => axum::response::Response::builder()
            .status(404)
            .body(axum::body::Body::from("not found"))
            .unwrap(),
    }
}
```

All vendor JS (Cytoscape.js, etc.) is included in `crates/codegraph-ui/assets/vendor/` — no CDN requests.

---

## 2. UI Structure

```
crates/codegraph-ui/assets/
├── index.html              (single-page shell)
├── app.js                  (main application logic)
├── style.css               (layout + theme)
└── vendor/
    ├── cytoscape.min.js    (3.27 — graph visualization)
    ├── cytoscape-cola.min.js  (layout algorithm)
    └── cola.min.js         (web-cola dependency)
```

No build step, no bundler — pure vanilla JS. The UI is intentionally minimal: a developer tool, not a product.

---

## 3. `index.html` Shell

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>codegraph</title>
  <link rel="stylesheet" href="/assets/style.css">
</head>
<body>
  <div id="sidebar">
    <h1>codegraph</h1>

    <!-- Search -->
    <div id="search-panel">
      <input id="search-input" type="text" placeholder="Search code by intent…" autocomplete="off">
      <div id="search-results"></div>
    </div>

    <!-- Symbol inspector -->
    <div id="inspector" hidden>
      <div id="inspector-name"></div>
      <div id="inspector-meta"></div>
      <pre id="inspector-snippet"></pre>
      <div id="inspector-callers"></div>
      <div id="inspector-callees"></div>
    </div>

    <!-- Index status -->
    <div id="status-bar">
      <span id="status-dot"></span>
      <span id="status-text">loading…</span>
    </div>
  </div>

  <div id="graph-container">
    <div id="cy"></div>
    <div id="graph-controls">
      <button id="btn-fit">Fit</button>
      <button id="btn-reset">Reset</button>
      <select id="lang-filter">
        <option value="">All languages</option>
      </select>
    </div>
  </div>

  <script src="/assets/vendor/cytoscape.min.js"></script>
  <script src="/assets/vendor/cola.min.js"></script>
  <script src="/assets/vendor/cytoscape-cola.min.js"></script>
  <script src="/assets/app.js"></script>
</body>
</html>
```

---

## 4. `app.js` — Core Logic

```javascript
// assets/app.js

const API = '/api';

// ── State ───────────────────────────────────────────────────────────────
let cy;       // Cytoscape instance
let selected; // currently selected symbol id

// ── Cytoscape init ───────────────────────────────────────────────────────
function initGraph() {
  cy = cytoscape({
    container: document.getElementById('cy'),
    style: [
      { selector: 'node', style: {
          'label':           'data(label)',
          'background-color':'#4a90d9',
          'color':           '#fff',
          'font-size':       '10px',
          'text-valign':     'center',
          'width':           'label',
          'height':          'label',
          'padding':         '6px',
          'shape':           'round-rectangle',
      }},
      { selector: 'node[kind="Function"]',  style: { 'background-color': '#4a90d9' }},
      { selector: 'node[kind="Struct"]',    style: { 'background-color': '#7bc67e' }},
      { selector: 'node[kind="Trait"]',     style: { 'background-color': '#e8a838' }},
      { selector: 'node[kind="Class"]',     style: { 'background-color': '#9b59b6' }},
      { selector: 'node:selected',          style: { 'border-width': 3, 'border-color': '#fff' }},
      { selector: 'edge', style: {
          'width':              1.5,
          'line-color':         '#888',
          'target-arrow-color': '#888',
          'target-arrow-shape': 'triangle',
          'curve-style':        'bezier',
          'opacity':            0.6,
      }},
    ],
    layout: { name: 'cola', animate: true, randomize: false, maxSimulationTime: 3000 },
    wheelSensitivity: 0.3,
  });

  cy.on('tap', 'node', e => loadSymbol(e.target.data('id')));
}

// ── Graph data load ───────────────────────────────────────────────────────
async function loadGraph(langFilter = '') {
  const url = langFilter ? `${API}/graph?lang=${langFilter}` : `${API}/graph`;
  const data = await fetch(url).then(r => r.json());

  const nodes = data.symbols.map(s => ({
    data: { id: s.id, label: s.name, kind: s.kind, file: s.file }
  }));
  const edges = data.edges.map(e => ({
    data: { source: e.from_id, target: e.to_id, kind: e.kind }
  }));

  cy.elements().remove();
  cy.add([...nodes, ...edges]);
  cy.layout({ name: 'cola', randomize: false, maxSimulationTime: 2000 }).run();
}

// ── Symbol inspector ─────────────────────────────────────────────────────
async function loadSymbol(symbolId) {
  selected = symbolId;
  const sym = await fetch(`${API}/symbol/${symbolId}`).then(r => r.json());

  document.getElementById('inspector').hidden = false;
  document.getElementById('inspector-name').textContent = sym.qualified_name;
  document.getElementById('inspector-meta').textContent =
    `${sym.kind}  •  ${sym.file}:${sym.start_line}–${sym.end_line}`;
  document.getElementById('inspector-snippet').textContent = sym.definition;

  renderSymbolList('inspector-callers', 'Callers', sym.callers);
  renderSymbolList('inspector-callees', 'Callees', sym.callees);

  // Highlight node in graph
  cy.elements().removeClass('selected-neighbor');
  const node = cy.getElementById(symbolId.toString());
  node?.select();
}

function renderSymbolList(containerId, label, symbols) {
  const el = document.getElementById(containerId);
  el.innerHTML = `<strong>${label} (${symbols.length})</strong>`;
  symbols.forEach(s => {
    const btn = document.createElement('button');
    btn.textContent = s.name;
    btn.className   = 'sym-link';
    btn.onclick     = () => loadSymbol(s.id);
    el.appendChild(btn);
  });
}

// ── Search ───────────────────────────────────────────────────────────────
let searchTimer;
document.getElementById('search-input').addEventListener('input', e => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => runSearch(e.target.value), 250);
});

async function runSearch(query) {
  if (!query.trim()) {
    document.getElementById('search-results').innerHTML = '';
    return;
  }
  const results = await fetch(
    `${API}/search?q=${encodeURIComponent(query)}&n=5`
  ).then(r => r.json());

  const container = document.getElementById('search-results');
  container.innerHTML = '';
  results.results.forEach((r, i) => {
    const el = document.createElement('div');
    el.className = 'result-item';
    el.innerHTML = `
      <div class="result-rank">#${i+1}</div>
      <div class="result-name">${r.symbol}</div>
      <div class="result-file">${r.file}:${r.lines}</div>
      <pre class="result-snippet">${escapeHtml(r.snippet)}</pre>
    `;
    el.onclick = () => loadSymbol(r.symbol_id);
    container.appendChild(el);
  });
}

function escapeHtml(s) {
  return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}

// ── Status polling ────────────────────────────────────────────────────────
async function pollStatus() {
  try {
    const s = await fetch(`${API}/status`).then(r => r.json());
    const dot  = document.getElementById('status-dot');
    const text = document.getElementById('status-text');
    dot.className  = `status-${s.state}`;
    text.textContent = `${s.state} · ${s.total_symbols} symbols · ${s.total_files} files`;
  } catch { /* server not ready yet */ }
}

// ── Language filter ──────────────────────────────────────────────────────
document.getElementById('lang-filter').addEventListener('change', e => {
  loadGraph(e.target.value);
});

document.getElementById('btn-fit').onclick   = () => cy.fit();
document.getElementById('btn-reset').onclick = () => loadGraph();

// ── Boot ─────────────────────────────────────────────────────────────────
initGraph();
loadGraph();
setInterval(pollStatus, 5000);
pollStatus();
```

---

## 5. `style.css` — Layout

```css
* { box-sizing: border-box; margin: 0; padding: 0; }
body { display: flex; height: 100vh; font-family: system-ui, sans-serif;
       background: #1a1a2e; color: #e0e0e0; }

#sidebar { width: 320px; min-width: 280px; display: flex; flex-direction: column;
           background: #16213e; border-right: 1px solid #2a2a5a; padding: 12px;
           overflow-y: auto; gap: 12px; }

h1 { font-size: 16px; font-weight: 700; color: #4a90d9; letter-spacing: .05em; }

#search-input { width: 100%; padding: 8px 10px; border-radius: 6px;
                border: 1px solid #2a2a5a; background: #0f3460; color: #e0e0e0;
                font-size: 13px; }

.result-item { background: #0f3460; border-radius: 6px; padding: 8px 10px;
               margin-top: 6px; cursor: pointer; }
.result-item:hover { background: #1a4a80; }
.result-name { font-weight: 600; font-size: 12px; color: #7ecfff; }
.result-file { font-size: 11px; color: #888; margin: 2px 0; }
.result-snippet { font-size: 11px; max-height: 80px; overflow: hidden;
                  font-family: monospace; color: #ccc; }

#inspector { background: #0f3460; border-radius: 6px; padding: 10px; }
#inspector-name { font-weight: 700; font-size: 13px; color: #7ecfff; word-break: break-all; }
#inspector-meta { font-size: 11px; color: #888; margin: 4px 0; }
#inspector-snippet { font-size: 11px; font-family: monospace; max-height: 200px;
                     overflow-y: auto; background: #1a1a2e; padding: 8px;
                     border-radius: 4px; margin: 8px 0; }

.sym-link { background: none; border: none; color: #7ecfff; font-size: 11px;
            cursor: pointer; display: block; padding: 2px 0; text-align: left; }
.sym-link:hover { text-decoration: underline; }

#status-bar { margin-top: auto; padding-top: 8px; border-top: 1px solid #2a2a5a;
              font-size: 11px; color: #888; display: flex; align-items: center; gap: 6px; }
#status-dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
.status-fresh    { background: #2ecc71; }
.status-indexing { background: #f39c12; }
.status-stale    { background: #e74c3c; }
.status-error    { background: #e74c3c; }

#graph-container { flex: 1; position: relative; }
#cy { width: 100%; height: 100%; }
#graph-controls { position: absolute; top: 12px; right: 12px;
                  display: flex; gap: 8px; align-items: center; }
#graph-controls button { background: #16213e; border: 1px solid #2a2a5a;
                         color: #e0e0e0; padding: 4px 10px; border-radius: 4px;
                         cursor: pointer; font-size: 12px; }
#graph-controls button:hover { background: #1a4a80; }
#lang-filter { background: #16213e; border: 1px solid #2a2a5a;
               color: #e0e0e0; padding: 4px 8px; border-radius: 4px; font-size: 12px; }
```

---

## 6. Graph Size Management

For repos with > 1,000 symbols, rendering the full graph is impractical. Strategies:

1. **Default view**: show top-100 symbols by in-degree (most-called)
2. **Language filter**: `/api/graph?lang=rust` limits to one language
3. **Search-centered view**: clicking a search result shows only that symbol + its 1-hop neighborhood
4. **"Load more" button**: expand the visible set by 50 nodes at a time

The API endpoint `/api/graph` accepts:
- `?lang=<language>` — filter by language
- `?limit=<n>` — limit node count (default 200, max 500)
- `?focus=<symbol_id>` — return 1-hop neighborhood of a symbol

---

## 7. Implementation Steps

1. Create `crates/codegraph-ui/assets/` directory structure
2. Download vendor JS files (cytoscape.min.js 3.27, cola.min.js, cytoscape-cola.min.js) into `assets/vendor/`
3. Write `index.html`
4. Write `style.css`
5. Write `app.js`
6. Implement `codegraph-ui/src/lib.rs` with `RustEmbed` + axum router
7. Wire UI router into `main.rs`
8. Verify: `curl http://localhost:3030/` returns HTML
9. Manual test: open in browser, load graph, run a search, click a node
10. Test graph-size limit: on a 50k LOC repo, graph renders within 2 s

---

## 8. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| Graph library | Cytoscape.js / D3.js / vis.js | Cytoscape.js — best performance for large graphs, good layout algos |
| Layout algorithm | cola / cose / dagre | cola (web-cola) — best for code graphs; force-directed with constraints |
| Build step | Bundler (Vite) / vanilla JS | Vanilla JS — no build step, assets served directly; simpler maintenance |
| Dark theme | Yes / No | Yes — code tool default |
| Language filter UI | Dropdown / chip toggles | Dropdown — simpler for v1 |
| Max visible nodes | 200 / 500 / 1000 | 200 default, configurable up to 500 |
