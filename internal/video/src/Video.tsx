import React from 'react';
import {AbsoluteFill, Img, Series, interpolate, useCurrentFrame, spring, useVideoConfig, Easing} from 'remotion';
import logo from '../../../docs/brand/logo.png';

const C = {
  bg: '#050505', bg2: '#161618', ink: '#F5F5F4', soft: '#A1A1AA', mid: '#8A8A92',
  main: '#3A3A40', deep: '#2A2A2E', warm: '#FF8A4C', hot: '#FF5A1F', ok: '#F5F5F4',
};
const FONT = 'Inter, -apple-system, "Helvetica Neue", Arial, sans-serif';
const MONO = 'ui-monospace, "SF Mono", Menlo, monospace';

const ramp = (f: number, a: number, b: number) =>
  interpolate(f, [a, b], [0, 1], {extrapolateLeft: 'clamp', extrapolateRight: 'clamp', easing: Easing.out(Easing.cubic)});

// ---------- graph data ----------
type N = {id: number; name: string; x: number; y: number};
const NODES: N[] = [
  {id: 0, name: 'send_invoice', x: 500, y: 330}, {id: 1, name: 'build_email', x: 700, y: 200},
  {id: 2, name: 'get_customer', x: 300, y: 210}, {id: 3, name: 'save_order', x: 250, y: 440},
  {id: 4, name: 'check_stock', x: 90, y: 330}, {id: 5, name: 'calc_tax', x: 710, y: 450},
  {id: 6, name: 'log_event', x: 910, y: 330}, {id: 7, name: 'format_date', x: 880, y: 90},
  {id: 8, name: 'load_config', x: 110, y: 110}, {id: 9, name: 'take_payment', x: 480, y: 570},
  {id: 10, name: 'refund', x: 700, y: 640}, {id: 11, name: 'render_pdf', x: 360, y: 70},
  {id: 12, name: 'auth_user', x: 90, y: 560}, {id: 13, name: 'db_query', x: 290, y: 630},
  {id: 14, name: 'notify', x: 930, y: 540}, {id: 15, name: 'cart_total', x: 560, y: 90},
];
const EDGES: [number, number][] = [
  [0, 1], [0, 2], [0, 5], [0, 11], [1, 14], [1, 7], [2, 13], [2, 12], [3, 2], [3, 4], [3, 13],
  [9, 5], [9, 3], [10, 9], [10, 14], [5, 6], [1, 6], [15, 5], [15, 0], [8, 2], [8, 15],
];
const nbrs = (i: number) => EDGES.filter((e) => e.includes(i)).map((e) => (e[0] === i ? e[1] : e[0]));
const dist = (src: number) => {
  const d: Record<number, number> = {[src]: 0};
  const q = [src];
  while (q.length) {
    const c = q.shift()!;
    nbrs(c).forEach((n) => { if (d[n] === undefined) { d[n] = d[c] + 1; q.push(n); } });
  }
  return d;
};

type NodeStyle = {r?: number; fill?: string; opacity?: number; glow?: number; label?: boolean};
const Graph: React.FC<{
  w?: number; h?: number; node: (n: N) => NodeStyle; edge: (i: number, a: N, b: N) => {p: number; color?: string; opacity?: number};
}> = ({w = 1000, h = 700, node, edge}) => (
  <svg viewBox={`0 0 ${w} ${h}`} style={{width: '100%', height: '100%', overflow: 'visible'}}>
    <defs>
      <filter id="glow" x="-100%" y="-100%" width="300%" height="300%"><feGaussianBlur stdDeviation="9" /></filter>
    </defs>
    {EDGES.map(([a, b], i) => {
      const A = NODES[a], B = NODES[b];
      const s = edge(i, A, B);
      return <line key={i} x1={A.x} y1={A.y} x2={A.x + (B.x - A.x) * s.p} y2={A.y + (B.y - A.y) * s.p}
        stroke={s.color ?? C.mid} strokeWidth={3} strokeLinecap="round" opacity={s.opacity ?? 0.55} />;
    })}
    {NODES.map((n) => {
      const s = node(n);
      const r = s.r ?? 16;
      return (
        <g key={n.id} opacity={s.opacity ?? 1}>
          {s.glow ? <circle cx={n.x} cy={n.y} r={r * 1.8} fill={s.fill ?? C.main} filter="url(#glow)" opacity={s.glow} /> : null}
          <circle cx={n.x} cy={n.y} r={r} fill={s.fill ?? C.main} stroke={C.ink} strokeWidth={3} />
          {s.label !== false && (
            <text x={n.x} y={n.y + r + 28} fill={C.ink} fontSize={24} fontFamily={MONO} textAnchor="middle">{n.name}</text>
          )}
        </g>
      );
    })}
  </svg>
);

// ---------- layout pieces ----------
const Bg: React.FC = () => {
  const f = useCurrentFrame();
  return (
    <AbsoluteFill style={{background: `radial-gradient(circle at 70% 20%, ${C.bg2}, ${C.bg} 70%)`}}>
      <svg width="1920" height="1080" style={{position: 'absolute', opacity: 0.35}}>
        {Array.from({length: 24 * 14}).map((_, i) => (
          <circle key={i} cx={(i % 24) * 84 + 30} cy={Math.floor(i / 24) * 84 + 30 + Math.sin((f + i * 7) / 40) * 3} r={2} fill={C.mid} />
        ))}
      </svg>
    </AbsoluteFill>
  );
};

const Scene: React.FC<{dur: number; step?: string; title: string; plain: string; children: React.ReactNode}> = ({dur, step, title, plain, children}) => {
  const f = useCurrentFrame();
  const o = Math.min(ramp(f, 0, 15), 1 - ramp(f, dur - 12, dur));
  return (
    <AbsoluteFill style={{fontFamily: FONT, color: C.ink, opacity: o}}>
      <div style={{position: 'absolute', left: 90, top: 70, right: 90}}>
        {step && <div style={{color: C.warm, fontSize: 30, fontWeight: 700, letterSpacing: 4, textTransform: 'uppercase', transform: `translateY(${(1 - ramp(f, 0, 18)) * 20}px)`}}>{step}</div>}
        <div style={{fontSize: 74, fontWeight: 800, lineHeight: 1.05, marginTop: 8, transform: `translateY(${(1 - ramp(f, 4, 24)) * 30}px)`}}>{title}</div>
      </div>
      <Img src={logo} style={{position: 'absolute', right: 70, top: 60, width: 84, height: 84, opacity: 0.9}} />
      {children}
      <div style={{position: 'absolute', left: 90, right: 90, bottom: 60, opacity: ramp(f, 25, 45), transform: `translateY(${(1 - ramp(f, 25, 45)) * 20}px)`,
        background: 'rgba(255,255,255,0.06)', border: `2px solid ${C.main}`, borderRadius: 24, padding: '22px 34px', display: 'flex', gap: 24, alignItems: 'center'}}>
        <div style={{background: C.warm, color: C.bg, fontWeight: 800, fontSize: 24, padding: '8px 18px', borderRadius: 14, whiteSpace: 'nowrap'}}>IN PLAIN WORDS</div>
        <div style={{fontSize: 38, lineHeight: 1.3}}>{plain}</div>
      </div>
    </AbsoluteFill>
  );
};

const Stage: React.FC<{children: React.ReactNode; left?: number; w?: number; top?: number}> = ({children, left = 560, w = 820, top = 255}) => (
  <div style={{position: 'absolute', left, top, width: w, height: 700 * (w / 1000)}}>{children}</div>
);

const S = (s: number) => Math.round(s * 30);
const D = {title: S(6), problem: S(10), map: S(9), read: S(10), search: S(10), follow: S(10), rank: S(10), tools: S(14), impact: S(9), safe: S(8), end: S(11)};
export const TOTAL = Object.values(D).reduce((a, b) => a + b, 0);

// ---------- scenes ----------
const Title: React.FC = () => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const s = spring({frame: f - 40, fps, config: {damping: 14}});
  return (
    <AbsoluteFill style={{fontFamily: FONT, color: C.ink, opacity: 1 - ramp(f, D.title - 12, D.title)}}>
      <div style={{position: 'absolute', left: 560, top: 60, width: 800, height: 560, opacity: 0.35}}>
        <Graph w={1000} h={700} node={(n) => ({r: 14, opacity: ramp(f, n.id * 3, n.id * 3 + 20), glow: 0.5, label: false})}
          edge={(i) => ({p: ramp(f, 20 + i * 2, 50 + i * 2)})} />
      </div>
      <Img src={logo} style={{position: 'absolute', left: 810, top: 120, width: 300, height: 300, transform: `scale(${0.6 + 0.4 * spring({frame: f, fps, config: {damping: 12}})})`, opacity: ramp(f, 0, 20)}} />
      <div style={{position: 'absolute', top: 640, width: '100%', textAlign: 'center', transform: `scale(${0.8 + 0.2 * s})`, opacity: s}}>
        <div style={{fontSize: 150, fontWeight: 900, letterSpacing: -3}}>Graph<span style={{color: C.mid}}>Relay</span></div>
        <div style={{fontSize: 48, color: C.soft, marginTop: 10, opacity: ramp(f, 80, 100)}}>A map of your code, so AI helpers stop getting lost</div>
        <div style={{fontSize: 34, color: C.warm, marginTop: 30, opacity: ramp(f, 110, 130)}}>Explained for everyone — no tech background needed</div>
      </div>
    </AbsoluteFill>
  );
};

const Problem: React.FC = () => {
  const f = useCurrentFrame();
  const cols = 14, rows = 9;
  const seen = Math.floor(ramp(f, 40, D.problem - 30) * cols * rows * 0.55);
  const order = (i: number) => (i * 37) % (cols * rows);
  return (
    <Scene dur={D.problem} step="The problem" title="A huge library, no catalogue" plain="Software can be a million lines long. An AI helper has to flip through pages one by one to find what matters — slow, and it often misses things.">
      <div style={{position: 'absolute', left: 90, top: 250, display: 'grid', gridTemplateColumns: `repeat(${cols}, 70px)`, gap: 12}}>
        {Array.from({length: cols * rows}).map((_, i) => {
          const visited = order(i) < seen;
          return <div key={i} style={{width: 70, height: 56, borderRadius: 8, background: visited ? C.ink : C.bg2, border: `2px solid ${visited ? C.ink : C.deep}`, opacity: ramp(f, i / 4, i / 4 + 15)}} />;
        })}
      </div>
      <div style={{position: 'absolute', right: 70, top: 300, width: 460, textAlign: 'center'}}>
        <div style={{fontSize: 96, fontWeight: 900, color: C.warm}}>{Math.round(ramp(f, 20, 120) * 1000000).toLocaleString()}</div>
        <div style={{fontSize: 34, color: C.soft}}>lines of code</div>
        <div style={{fontSize: 30, marginTop: 28, opacity: ramp(f, 90, 120)}}>Yellow = pages already flipped…<br />most are still unread</div>
      </div>
    </Scene>
  );
};

const MapScene: React.FC = () => {
  const f = useCurrentFrame();
  return (
    <Scene dur={D.map} step="The idea" title="Draw a map instead" plain="Every function (a small job the code does) becomes a dot. Every “this one uses that one” becomes a line. Like a subway map for your software.">
      <Stage>
        <Graph node={(n) => ({opacity: ramp(f, 20 + n.id * 3, 40 + n.id * 3), glow: 0.3})} edge={(i) => ({p: ramp(f, 60 + i * 3, 90 + i * 3)})} />
      </Stage>
      <div style={{position: 'absolute', left: 90, top: 330, width: 340, fontSize: 36, lineHeight: 1.4, opacity: ramp(f, 100, 130)}}>
        <b style={{color: C.mid}}>● Dot</b> = a job<br /><b style={{color: C.mid}}>━ Line</b> = “uses”
      </div>
    </Scene>
  );
};

const SNIP = ['def send_invoice(order):', '  customer = get_customer(order)', '  tax = calc_tax(order)', '  pdf = render_pdf(order)', '  build_email(customer, pdf)'];
const Read: React.FC = () => {
  const f = useCurrentFrame();
  return (
    <Scene dur={D.read} step="Step 1 · Read" title="Read the code once" plain="GraphRelay reads every file one time, right on your own computer, and notes who uses whom. After that it just keeps the map fresh.">
      <div style={{position: 'absolute', left: 90, top: 260, width: 640, background: '#0B0B0C', border: `2px solid ${C.deep}`, borderRadius: 20, padding: 28, fontFamily: MONO, fontSize: 30, lineHeight: 1.7}}>
        {SNIP.map((l, i) => {
          const hl = ramp(f, 40 + i * 22, 55 + i * 22);
          return <div key={i} style={{color: C.ink, background: `rgba(255,106,44,${hl * 0.25 * (1 - ramp(f, 160 + i * 4, 180 + i * 4))})`, borderRadius: 6, whiteSpace: 'pre'}}>{l}</div>;
        })}
      </div>
      <div style={{position: 'absolute', left: 740, top: 450, fontSize: 90, color: C.warm, opacity: ramp(f, 120, 140)}}>→</div>
      <Stage left={900} w={800}>
        <Graph node={(n) => ({label: false, opacity: ramp(f, 140 + n.id * 2, 155 + n.id * 2), glow: 0.3})} edge={(i) => ({p: ramp(f, 150 + i * 2, 175 + i * 2)})} />
      </Stage>
    </Scene>
  );
};

const LIT = [0, 1, 11];
const Search: React.FC = () => {
  const f = useCurrentFrame();
  const q = 'where do we email invoices?';
  const typed = q.slice(0, Math.floor(ramp(f, 20, 90) * q.length));
  return (
    <Scene dur={D.search} step="Step 2 · Find" title="Match the words" plain="You ask in everyday language. GraphRelay finds the dots whose names and descriptions share your words — those are the starting points.">
      <div style={{position: 'absolute', left: 90, top: 250, background: C.ink, color: C.bg, borderRadius: 40, padding: '18px 34px', fontSize: 38, fontWeight: 600, width: 760}}>
        {typed}<span style={{opacity: Math.floor(f / 10) % 2}}>|</span>
      </div>
      <Stage top={340} w={740} left={640}>
        <Graph node={(n) => {
          const on = LIT.includes(n.id) ? ramp(f, 100 + n.id * 3, 120 + n.id * 3) : 0;
          return {fill: on > 0.5 ? C.warm : C.main, r: 16 + on * 14, glow: on * 0.9, opacity: on ? 1 : 0.55};
        }} edge={() => ({p: 1, opacity: 0.25})} />
      </Stage>
    </Scene>
  );
};

const Follow: React.FC = () => {
  const f = useCurrentFrame();
  const d = dist(0);
  return (
    <Scene dur={D.follow} step="Step 3 · Follow" title="Follow the lines" plain="Good answers hide next door. So it walks along the lines to neighbours: who uses this? what does this use? That is the “graph” in GraphRelay.">
      <Stage>
        <Graph node={(n) => {
          const hop = LIT.includes(n.id) ? 0 : nbrs(0).includes(n.id) || LIT.some((l) => nbrs(l).includes(n.id)) ? 1 : 2;
          const on = hop === 0 ? 1 : hop === 1 ? ramp(f, 60 + n.id * 4, 90 + n.id * 4) : 0;
          return {fill: hop === 0 ? C.warm : on > 0.5 ? C.ok : C.main, r: 16 + on * 8, glow: on * 0.7, opacity: hop === 2 ? 0.45 : 1};
        }} edge={(i, a, b) => {
          const touch = LIT.includes(a.id) || LIT.includes(b.id) || nbrs(0).includes(a.id) && nbrs(0).includes(b.id);
          return {p: 1, color: touch ? C.ok : C.mid, opacity: touch ? 0.4 + 0.6 * ramp(f, 50, 100) : 0.2};
        }} />
      </Stage>
      <div style={{position: 'absolute', left: 90, top: 330, width: 420, fontSize: 30, lineHeight: 1.5, opacity: ramp(f, 100, 130)}}>
        <b style={{color: C.warm}}>● Found by words</b><br /><b style={{color: C.ok}}>● Found by<br />&nbsp;&nbsp;following lines</b>
      </div>
    </Scene>
  );
};

const IMP: Record<number, number> = {0: 1, 2: 0.85, 5: 0.75, 1: 0.7, 13: 0.55, 15: 0.45};
const RANK = [0, 2, 5, 1];
const Rank: React.FC = () => {
  const f = useCurrentFrame();
  const g = ramp(f, 40, 110);
  return (
    <Scene dur={D.rank} step="Step 4 · Rank" title="Busy crossroads matter most" plain="Like a friend everyone knows, a function that many others rely on is probably important. GraphRelay scores that and puts the best answers first.">
      <Stage left={420} w={800}>
        <Graph node={(n) => {
          const im = IMP[n.id] ?? 0.2;
          return {r: 14 + g * im * 26, fill: im > 0.6 ? C.warm : C.main, glow: g * im, opacity: 0.4 + 0.6 * Math.max(im, 1 - g)};
        }} edge={() => ({p: 1, opacity: 0.35})} />
      </Stage>
      <div style={{position: 'absolute', right: 90, top: 270, width: 480}}>
        <div style={{fontSize: 30, color: C.soft, marginBottom: 14, opacity: ramp(f, 100, 120)}}>Best answers</div>
        {RANK.map((id, i) => (
          <div key={id} style={{display: 'flex', gap: 16, alignItems: 'center', background: 'rgba(255,255,255,0.07)', borderRadius: 16, padding: '14px 20px', marginBottom: 14,
            opacity: ramp(f, 120 + i * 20, 140 + i * 20), transform: `translateX(${(1 - ramp(f, 120 + i * 20, 140 + i * 20)) * 40}px)`}}>
            <div style={{fontSize: 40, fontWeight: 900, color: C.warm}}>{i + 1}</div>
            <div style={{fontSize: 32, fontFamily: MONO}}>{NODES[id].name}</div>
          </div>
        ))}
      </div>
    </Scene>
  );
};

const TOOLS = [
  {n: 'search_code', ic: '01', t: 'Find it', d: 'Ask a question in plain language and get the most relevant code.'},
  {n: 'inspect', ic: '02', t: 'Look closer', d: 'See one function or file: what it does, who uses it, what it uses.'},
  {n: 'trace_impact', ic: '03', t: 'What breaks?', d: 'Before changing something, see which functions call it, directly and indirectly.'},
  {n: 'watch_status', ic: '04', t: 'Is it fresh?', d: 'Check the map is up to date with your latest edits.'},
];
const Tools: React.FC = () => {
  const f = useCurrentFrame();
  return (
    <Scene dur={D.tools} step="What the AI gets" title="Four simple tools" plain="The AI helper plugs in through a standard connector (called MCP) and can use these four tools — like four buttons on a remote.">
      <div style={{position: 'absolute', left: 90, right: 90, top: 270, display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 28}}>
        {TOOLS.map((t, i) => {
          const o = ramp(f, 40 + i * 35, 65 + i * 35);
          return (
            <div key={t.n} style={{background: 'rgba(255,255,255,0.06)', border: `2px solid ${C.main}`, borderRadius: 28, padding: 30, height: 440, opacity: o, transform: `translateY(${(1 - o) * 50}px)`}}>
              <div style={{fontSize: 34, fontFamily: MONO, color: C.warm, letterSpacing: 4}}>{t.ic}</div>
              <div style={{fontSize: 44, fontWeight: 800, marginTop: 10}}>{t.t}</div>
              <div style={{fontSize: 24, fontFamily: MONO, color: C.warm, margin: '8px 0 16px'}}>{t.n}</div>
              <div style={{fontSize: 30, lineHeight: 1.35, color: C.soft}}>{t.d}</div>
            </div>
          );
        })}
      </div>
    </Scene>
  );
};

const Impact: React.FC = () => {
  const f = useCurrentFrame();
  const d = dist(5);
  const t = (n: number) => ramp(f, 50 + (d[n] ?? 4) * 28, 70 + (d[n] ?? 4) * 28);
  return (
    <Scene dur={D.impact} step="Example · trace_impact" title="“If I change this, what breaks?”" plain="Say you change the tax calculation. GraphRelay lights up the functions that call it, ring by ring, so you know where to look first.">
      <Stage>
        <Graph node={(n) => {
          const on = t(n.id);
          return {fill: n.id === 5 ? C.hot : on > 0.5 ? (d[n.id] === 1 ? C.warm : '#FFC9A8') : C.main, r: n.id === 5 ? 30 : 16 + on * 8, glow: on * 0.8};
        }} edge={(i, a, b) => ({p: 1, color: t(a.id) > 0.5 && t(b.id) > 0.5 ? C.hot : C.mid, opacity: 0.5})} />
      </Stage>
      <div style={{position: 'absolute', left: 90, top: 330, width: 340, fontSize: 34, lineHeight: 1.5}}>
        <b style={{color: C.hot}}>● Changed</b><br /><b style={{color: C.warm}}>● Affected first</b><br /><b style={{color: '#FFC9A8'}}>● Affected next</b>
      </div>
    </Scene>
  );
};

const Safe: React.FC = () => {
  const f = useCurrentFrame();
  const B = [
    {ic: '01', t: 'One small program', d: 'Called “gr”. Nothing else to install or run.'},
    {ic: '02', t: 'Stays on your computer', d: 'Your code never leaves your machine. No account, no key.'},
    {ic: '03', t: 'Always up to date', d: 'It watches your files and refreshes the map when you save.'},
  ];
  return (
    <Scene dur={D.safe} step="Why people like it" title="Simple, private, current" plain="No cloud, no sign-up, no AI model to download. It is just a fast, careful map-maker sitting next to your code.">
      <div style={{position: 'absolute', left: 90, right: 90, top: 280, display: 'grid', gridTemplateColumns: 'repeat(3,1fr)', gap: 34}}>
        {B.map((b, i) => {
          const o = ramp(f, 40 + i * 30, 65 + i * 30);
          return <div key={i} style={{background: 'rgba(255,255,255,0.06)', border: `2px solid ${C.main}`, borderRadius: 28, padding: 34, height: 380, opacity: o, transform: `scale(${0.9 + 0.1 * o})`}}>
            <div style={{fontSize: 34, fontFamily: MONO, color: C.warm, letterSpacing: 4}}>{b.ic}</div>
            <div style={{fontSize: 46, fontWeight: 800, margin: '10px 0'}}>{b.t}</div>
            <div style={{fontSize: 32, color: C.soft, lineHeight: 1.35}}>{b.d}</div>
          </div>;
        })}
      </div>
    </Scene>
  );
};

const End: React.FC = () => {
  const f = useCurrentFrame();
  const cmds = ['gr init', 'gr start', 'gr setup'];
  const cap = ['register your project', 'build the map', 'connect your AI helper'];
  return (
    <AbsoluteFill style={{fontFamily: FONT, color: C.ink, opacity: ramp(f, 0, 15)}}>
      <div style={{position: 'absolute', top: 90, width: '100%', textAlign: 'center'}}>
        <div style={{fontSize: 60, fontWeight: 800}}>Honest status</div>
        <div style={{fontSize: 38, color: C.soft, width: 1400, margin: '16px auto 0', lineHeight: 1.4, opacity: ramp(f, 20, 45)}}>
          In our tests on a million-line project, GraphRelay scores slightly above a plain-search AI helper, but the gap is within the noise. We publish the numbers, good or not — and we are still finding where it truly helps.
        </div>
      </div>
      <div style={{position: 'absolute', top: 480, width: '100%', display: 'flex', justifyContent: 'center', gap: 36}}>
        {cmds.map((c, i) => {
          const o = ramp(f, 80 + i * 25, 105 + i * 25);
          return <div key={c} style={{opacity: o, transform: `translateY(${(1 - o) * 30}px)`, background: '#0B0B0C', border: `2px solid ${C.main}`, borderRadius: 20, padding: '24px 40px', textAlign: 'center'}}>
            <div style={{fontFamily: MONO, fontSize: 52, color: C.ok}}>$ {c}</div>
            <div style={{fontSize: 28, color: C.soft, marginTop: 8}}>{cap[i]}</div>
          </div>;
        })}
      </div>
      <div style={{position: 'absolute', bottom: 120, width: '100%', textAlign: 'center', opacity: ramp(f, 190, 220)}}>
        <div style={{fontSize: 110, fontWeight: 900, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 28}}>
          <Img src={logo} style={{width: 130, height: 130}} />
          <span>Graph<span style={{color: C.mid}}>Relay</span></span>
        </div>
        <div style={{fontSize: 38, color: C.soft}}>Give your AI helper a map.</div>
      </div>
    </AbsoluteFill>
  );
};

export const Intro: React.FC = () => (
  <AbsoluteFill>
  <Bg />
  <Series>
    <Series.Sequence durationInFrames={D.title}><Title /></Series.Sequence>
    <Series.Sequence durationInFrames={D.problem}><Problem /></Series.Sequence>
    <Series.Sequence durationInFrames={D.map}><MapScene /></Series.Sequence>
    <Series.Sequence durationInFrames={D.read}><Read /></Series.Sequence>
    <Series.Sequence durationInFrames={D.search}><Search /></Series.Sequence>
    <Series.Sequence durationInFrames={D.follow}><Follow /></Series.Sequence>
    <Series.Sequence durationInFrames={D.rank}><Rank /></Series.Sequence>
    <Series.Sequence durationInFrames={D.tools}><Tools /></Series.Sequence>
    <Series.Sequence durationInFrames={D.impact}><Impact /></Series.Sequence>
    <Series.Sequence durationInFrames={D.safe}><Safe /></Series.Sequence>
    <Series.Sequence durationInFrames={D.end}><End /></Series.Sequence>
  </Series>
  </AbsoluteFill>
);
