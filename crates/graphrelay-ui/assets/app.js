'use strict';

// window.BASE is injected by the server at index-serve time (e.g. "/graphrelay").
// Falls back to empty string so the file can be opened directly during development.
const BASE = (typeof window !== 'undefined' && window.BASE) ? window.BASE : '';
const API  = BASE + '/api';

// ── State ─────────────────────────────────────────────────────────────────────
let cy;           // Cytoscape instance
let searchTimer;  // debounce handle

// ── Cytoscape init ────────────────────────────────────────────────────────────
function initGraph() {
  cy = cytoscape({
    container: document.getElementById('cy'),
    style: [
      {
        selector: 'node',
        style: {
          label:             'data(label)',
          'background-color':'#4a90d9',
          color:             '#fff',
          'font-size':       '10px',
          'text-valign':     'center',
          'text-halign':     'center',
          'text-wrap':       'ellipsis',
          'text-max-width':  '120px',
          width:             'label',
          height:            'label',
          padding:           '6px',
          shape:             'round-rectangle',
        },
      },
      { selector: 'node[kind="Function"]',  style: { 'background-color': '#4a90d9' } },
      { selector: 'node[kind="Struct"]',    style: { 'background-color': '#7bc67e' } },
      { selector: 'node[kind="Trait"]',     style: { 'background-color': '#e8a838' } },
      { selector: 'node[kind="Class"]',     style: { 'background-color': '#9b59b6' } },
      { selector: 'node[kind="Enum"]',      style: { 'background-color': '#e67e22' } },
      { selector: 'node[kind="Interface"]', style: { 'background-color': '#1abc9c' } },
      { selector: 'node[kind="Method"]',    style: { 'background-color': '#3d8bcd' } },
      {
        selector: 'node:selected',
        style: { 'border-width': 3, 'border-color': '#fff', 'background-color': '#fff', color: '#1a1a2e' },
      },
      {
        selector: 'edge',
        style: {
          width:                1.5,
          'line-color':         '#445',
          'target-arrow-color': '#445',
          'target-arrow-shape': 'triangle',
          'curve-style':        'bezier',
          opacity:              0.7,
        },
      },
    ],
    layout: { name: 'cose', animate: false },
    wheelSensitivity: 0.3,
    minZoom: 0.05,
    maxZoom: 4,
  });

  cy.on('tap', 'node', e => loadSymbol(Number(e.target.data('id'))));
}

// ── Graph load ────────────────────────────────────────────────────────────────
async function loadGraph(langFilter) {
  const params = new URLSearchParams({ limit: 200 });
  if (langFilter) params.set('lang', langFilter);

  try {
    const resp = await fetch(`${API}/graph?${params}`);
    if (!resp.ok) return;
    const json = await resp.json();
    const { symbols, edges } = json.data;

    const nodes = symbols.map(s => ({
      data: { id: String(s.id), label: s.name, kind: s.kind, file: s.file, lines: s.lines },
    }));
    const edgeElems = edges.map((e, i) => ({
      data: { id: `e${i}`, source: String(e.from_id), target: String(e.to_id), kind: e.kind },
    }));

    cy.elements().remove();
    cy.add([...nodes, ...edgeElems]);

    // Cola layout when the extension is loaded; fall back to cose.
    const layoutName = (typeof cytoscapeCola !== 'undefined') ? 'cola' : 'cose';
    const layoutOpts = layoutName === 'cola'
      ? { name: 'cola', animate: true, randomize: false, maxSimulationTime: 3000 }
      : { name: 'cose', animate: false };
    cy.layout(layoutOpts).run();
  } catch (err) {
    console.error('loadGraph error', err);
  }
}

// ── Focus: show 1-hop neighbourhood of a symbol ───────────────────────────────
async function loadFocus(symbolId) {
  try {
    const resp = await fetch(`${API}/graph?focus=${symbolId}&limit=50`);
    if (!resp.ok) return;
    const json = await resp.json();
    const { symbols, edges } = json.data;

    const nodes = symbols.map(s => ({
      data: { id: String(s.id), label: s.name, kind: s.kind, file: s.file, lines: s.lines },
    }));
    const edgeElems = edges.map((e, i) => ({
      data: { id: `e${i}`, source: String(e.from_id), target: String(e.to_id), kind: e.kind },
    }));

    cy.elements().remove();
    cy.add([...nodes, ...edgeElems]);
    cy.layout({ name: 'cose', animate: false }).run();
    cy.getElementById(String(symbolId)).select();
  } catch (err) {
    console.error('loadFocus error', err);
  }
}

// ── Symbol inspector ──────────────────────────────────────────────────────────
async function loadSymbol(symbolId) {
  try {
    const resp = await fetch(`${API}/symbol/${symbolId}`);
    if (!resp.ok) return;
    const json = await resp.json();
    const sym = json.data;

    document.getElementById('inspector').hidden = false;
    document.getElementById('inspector-name').textContent = sym.qualified_name;
    document.getElementById('inspector-meta').textContent =
      `${sym.kind}  ·  ${sym.file}  ·  lines ${sym.start_line}–${sym.end_line}`;
    document.getElementById('inspector-snippet').textContent = sym.definition || '(no source)';

    renderSymbolList('inspector-callers', 'Callers', sym.callers || []);
    renderSymbolList('inspector-callees', 'Callees', sym.callees || []);

    // Highlight in graph; if not present zoom into 1-hop neighbourhood.
    const node = cy.getElementById(String(symbolId));
    if (node.length) {
      cy.elements().unselect();
      node.select();
      cy.animate({ fit: { eles: node, padding: 80 }, duration: 300 });
    } else {
      loadFocus(symbolId);
    }
  } catch (err) {
    console.error('loadSymbol error', err);
  }
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

// ── Search ────────────────────────────────────────────────────────────────────
document.getElementById('search-input').addEventListener('input', e => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => runSearch(e.target.value.trim()), 250);
});

async function runSearch(query) {
  const container = document.getElementById('search-results');
  if (!query) { container.innerHTML = ''; return; }

  try {
    const resp = await fetch(`${API}/search?${new URLSearchParams({ q: query, n: 5 })}`);
    if (!resp.ok) { container.innerHTML = '<div class="result-item">Index not ready.</div>'; return; }
    const json = await resp.json();
    const results = json.data.results || [];

    container.innerHTML = '';
    if (!results.length) {
      container.innerHTML = '<div class="result-item" style="color:#888">No results.</div>';
      return;
    }

    results.forEach(r => {
      const el = document.createElement('div');
      el.className = 'result-item';
      el.innerHTML = `
        <div class="result-rank">#${r.rank} &nbsp; ${r.kind}</div>
        <div class="result-name">${escapeHtml(r.symbol)}</div>
        <div class="result-file">${escapeHtml(r.file)}:${escapeHtml(r.lines)}</div>
        <pre class="result-snippet">${escapeHtml(r.snippet || '')}</pre>
      `;
      el.onclick = () => loadSymbol(r.symbol_id);
      container.appendChild(el);
    });
  } catch (err) {
    console.error('runSearch error', err);
  }
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

// ── Status polling ────────────────────────────────────────────────────────────
async function pollStatus() {
  try {
    const s = await fetch(`${API}/status`).then(r => r.json());
    const dot  = document.getElementById('status-dot');
    const text = document.getElementById('status-text');
    dot.className  = `status-${s.state}`;
    text.textContent =
      `${s.state} · ${s.total_symbols.toLocaleString()} symbols · ${s.total_files.toLocaleString()} files`;
  } catch { /* server not yet reachable */ }
}

// ── Language filter ───────────────────────────────────────────────────────────
async function populateLanguageFilter() {
  try {
    const resp = await fetch(`${API}/languages`);
    if (!resp.ok) return;
    const json  = await resp.json();
    const langs = (json.data && json.data.languages) || [];
    const sel   = document.getElementById('lang-filter');
    langs.forEach(l => {
      const opt = document.createElement('option');
      opt.value       = l.language;
      opt.textContent = `${l.language} (${l.file_count})`;
      sel.appendChild(opt);
    });
  } catch { /* ignore */ }
}

document.getElementById('lang-filter').addEventListener('change', e => {
  loadGraph(e.target.value || undefined);
});

document.getElementById('btn-fit').onclick   = () => cy.fit();
document.getElementById('btn-reset').onclick = () => {
  document.getElementById('lang-filter').value = '';
  loadGraph();
};

// ── Boot ──────────────────────────────────────────────────────────────────────
initGraph();
loadGraph();
populateLanguageFilter();
pollStatus();
setInterval(pollStatus, 5000);
