import React from 'react';
import {AbsoluteFill, Img, Sequence, interpolate, spring, useCurrentFrame, useVideoConfig, Easing} from 'remotion';
import logo from '../../../docs/brand/logo.png';

// Palette: docs/brand/COLORS.md (luxury black & white, small orange accent)
const C = {
  bg: '#050505', panel: '#0C0C0D', panel2: '#161618', line: '#2A2A2E', line2: '#3A3A40',
  ink: '#F5F5F4', soft: '#A1A1AA', mid: '#8A8A92', dim: '#6B6B72', acc: '#FF5A1F', acc2: '#FF8A4C',
};
const SANS = 'Inter, -apple-system, "Helvetica Neue", Arial, sans-serif';
const MONO = 'ui-monospace, "SF Mono", Menlo, monospace';
const FPS = 30;

const ease = Easing.bezier(0.22, 1, 0.36, 1);
const ramp = (f: number, a: number, b: number) =>
  interpolate(f, [a, b], [0, 1], {extrapolateLeft: 'clamp', extrapolateRight: 'clamp', easing: ease});

// ---------- scene wrapper: crossfade + gentle push-in ----------
const FadeScene: React.FC<{dur: number; children: React.ReactNode}> = ({dur, children}) => {
  const f = useCurrentFrame();
  const o = Math.min(ramp(f, 0, 14), 1 - ramp(f, dur - 14, dur));
  const z = 1.035 - 0.035 * ramp(f, 0, 40);
  return <AbsoluteFill style={{opacity: o, transform: `scale(${z})`, fontFamily: SANS, color: C.ink}}>{children}</AbsoluteFill>;
};

const Backdrop: React.FC = () => {
  const f = useCurrentFrame();
  return (
    <AbsoluteFill style={{background: `radial-gradient(ellipse at 70% 0%, ${C.panel2}, ${C.bg} 65%)`}}>
      <svg width="1920" height="1080" style={{position: 'absolute', opacity: 0.5}}>
        {Array.from({length: 26 * 15}).map((_, i) => (
          <circle key={i} cx={(i % 26) * 76 + 20} cy={Math.floor(i / 26) * 76 + 20 + Math.sin((f + i * 9) / 55) * 2.5} r={1.6} fill={C.line2} />
        ))}
      </svg>
    </AbsoluteFill>
  );
};

const Label: React.FC<{children: React.ReactNode; f: number; at?: number}> = ({children, f, at = 0}) => (
  <div style={{color: C.acc2, fontSize: 26, fontWeight: 700, letterSpacing: 6, textTransform: 'uppercase', opacity: ramp(f, at, at + 16), transform: `translateY(${(1 - ramp(f, at, at + 16)) * 14}px)`}}>{children}</div>
);
const Head: React.FC<{children: React.ReactNode; f: number; at?: number; size?: number}> = ({children, f, at = 4, size = 70}) => (
  <div style={{fontSize: size, fontWeight: 800, lineHeight: 1.08, letterSpacing: -1.5, marginTop: 10, opacity: ramp(f, at, at + 20), transform: `translateY(${(1 - ramp(f, at, at + 20)) * 26}px)`}}>{children}</div>
);
const Caption: React.FC<{children: React.ReactNode; f: number; at: number}> = ({children, f, at}) => (
  <div style={{position: 'absolute', left: 90, right: 90, bottom: 56, display: 'flex', gap: 22, alignItems: 'center', background: 'rgba(255,255,255,0.05)', border: `1.5px solid ${C.line}`, borderRadius: 22, padding: '20px 30px', opacity: ramp(f, at, at + 20), transform: `translateY(${(1 - ramp(f, at, at + 20)) * 22}px)`}}>
    <div style={{background: C.acc, color: C.bg, fontWeight: 800, fontSize: 20, padding: '7px 16px', borderRadius: 12, letterSpacing: 1, whiteSpace: 'nowrap'}}>IN PLAIN WORDS</div>
    <div style={{fontSize: 34, lineHeight: 1.3, color: C.ink}}>{children}</div>
  </div>
);
const Mark: React.FC<{size?: number}> = ({size = 78}) => <Img src={logo} style={{position: 'absolute', right: 70, top: 56, width: size, height: size}} />;

// ---------- terminal ----------
type Span = [string, string?];
type TLine = {at: number; spans: Span[]; type?: boolean; cps?: number; gap?: boolean};
const plain = (l: TLine) => l.spans.map((s) => s[0]).join('');

const Term: React.FC<{lines: TLine[]; f: number; w: number; h: number; font?: number; title?: string; chrome?: boolean}> = ({lines, f, w, h, font = 22, title = 'zsh', chrome = true}) => {
  const lh = font * 1.55;
  let revealed = 0;
  const rows = lines.map((l) => {
    const total = plain(l).length;
    const shown = l.type ? Math.min(total, Math.max(0, Math.floor((f - l.at) * (l.cps ?? 1.1)))) : f >= l.at ? total : 0;
    const prog = l.type ? (total ? shown / total : 1) : ramp(f, l.at, l.at + 10);
    revealed += f >= l.at ? (l.type ? Math.min(1, prog + 0.001) : prog) : 0;
    return {l, shown, prog, total};
  });
  const maxRows = Math.floor((h - (chrome ? 110 : 60)) / lh);
  const scroll = Math.max(0, revealed - maxRows) * lh;
  const last = [...rows].reverse().find((r) => f >= r.l.at && r.l.type);
  return (
    <div style={{width: w, height: h, background: 'rgba(12,12,13,0.94)', border: `2px solid ${C.line}`, borderRadius: 30, overflow: 'hidden', boxShadow: '0 30px 80px rgba(0,0,0,0.6)', position: 'relative'}}>
      {chrome && (
        <div style={{display: 'flex', gap: 12, alignItems: 'center', padding: '22px 28px'}}>
          <i style={{width: 16, height: 16, borderRadius: 8, background: C.acc}} /><i style={{width: 16, height: 16, borderRadius: 8, background: C.line2}} /><i style={{width: 16, height: 16, borderRadius: 8, background: C.line2}} />
          <span style={{marginLeft: 'auto', color: C.dim, fontFamily: MONO, fontSize: 18}}>{title}</span>
        </div>
      )}
      <div style={{padding: chrome ? '4px 34px' : '30px 34px', fontFamily: MONO, fontSize: font, lineHeight: `${lh}px`, whiteSpace: 'pre', transform: `translateY(${-scroll}px)`}}>
        {rows.map(({l, shown, prog}, i) => {
          if (f < l.at) return <div key={i} style={{height: lh}} />;
          if (l.gap) return <div key={i} style={{height: lh * 0.6}} />;
          let used = 0;
          const out = l.spans.map(([t, c], k) => {
            const take = l.type ? Math.max(0, Math.min(t.length, shown - used)) : t.length;
            used += t.length;
            return <span key={k} style={{color: c ?? C.ink}}>{t.slice(0, take)}</span>;
          });
          const isLast = last && last.l === l && shown < plain(l).length + 1;
          return (
            <div key={i} style={{height: lh, opacity: l.type ? 1 : prog, transform: l.type ? undefined : `translateX(${(1 - prog) * 14}px)`}}>
              {out}{isLast && Math.floor(f / 8) % 2 === 0 ? <span style={{background: C.ink, color: C.bg}}> </span> : null}
            </div>
          );
        })}
      </div>
    </div>
  );
};

// ---------- real data (Gitea @ f35c67bd, via the running tools) ----------
const SEARCH = [
  ['CreatePullRequests', 'services/migrations/gitea_uploader.go', '0.947'],
  ['NewPullRequest', 'services/pull/pull.go', '0.936'],
  ['NewPullRequest', 'services/mailer/notify.go', '0.933'],
];
const pad = (s: string, n: number) => s + ' '.repeat(Math.max(1, n - s.length));

// ---------- scenes ----------
const D = {title: 150, problem: 240, idea: 240, setup: 300, session: 720, tools: 270, trust: 240, outro: 180};
const OV = 14;
const order: (keyof typeof D)[] = ['title', 'problem', 'idea', 'setup', 'session', 'tools', 'trust', 'outro'];
const starts: Record<string, number> = {};
{ let t = 0; order.forEach((k) => { starts[k] = t; t += D[k] - OV; }); }
export const TOTAL_V2 = order.reduce((a, k) => a + D[k], 0) - OV * (order.length - 1);

const Title: React.FC = () => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const s = spring({frame: f - 4, fps, config: {damping: 14, mass: 0.8}});
  const word = 'GraphRelay';
  return (
    <FadeScene dur={D.title}>
      <svg viewBox="0 0 1920 1080" style={{position: 'absolute', inset: 0, opacity: 0.5}}>
        {Array.from({length: 22}).map((_, i) => {
          const a = (i / 22) * Math.PI * 2, r = 330 + (i % 3) * 70, x = 960 + Math.cos(a + f / 260) * r * 1.5, y = 400 + Math.sin(a + f / 260) * r * 0.7;
          const x2 = 960 + Math.cos(a + 0.7 + f / 260) * (330 + ((i + 1) % 3) * 70) * 1.5, y2 = 400 + Math.sin(a + 0.7 + f / 260) * (330 + ((i + 1) % 3) * 70) * 0.7;
          return <g key={i} opacity={ramp(f, i, i + 30)}><line x1={x} y1={y} x2={x2} y2={y2} stroke={C.line2} strokeWidth={1.5} /><circle cx={x} cy={y} r={6} fill={C.panel2} stroke={C.mid} strokeWidth={1.5} /></g>;
        })}
      </svg>
      <Img src={logo} style={{position: 'absolute', left: 810, top: 150, width: 300, height: 300, transform: `scale(${0.55 + 0.45 * s}) rotate(${(1 - s) * -25}deg)`, opacity: ramp(f, 0, 18)}} />
      <div style={{position: 'absolute', top: 560, width: '100%', textAlign: 'center', fontSize: 150, fontWeight: 900, letterSpacing: -4}}>
        {word.split('').map((ch, i) => (
          <span key={i} style={{display: 'inline-block', color: i < 5 ? C.ink : C.mid, opacity: ramp(f, 26 + i * 3, 42 + i * 3), transform: `translateY(${(1 - ramp(f, 26 + i * 3, 42 + i * 3)) * 40}px)`}}>{ch}</span>
        ))}
      </div>
      <div style={{position: 'absolute', top: 760, width: '100%', textAlign: 'center', fontSize: 44, color: C.soft, opacity: ramp(f, 70, 95)}}>Give your AI coding agent a map of your code.</div>
      <div style={{position: 'absolute', top: 850, width: '100%', textAlign: 'center', fontSize: 28, color: C.acc2, letterSpacing: 3, opacity: ramp(f, 95, 120)}}>EXPLAINED FOR EVERYONE</div>
    </FadeScene>
  );
};

const Problem: React.FC = () => {
  const f = useCurrentFrame();
  const calls: [string, string][] = [
    ['Grep', 'pattern: "pull request"'], ['Read', 'services/pull/pull.go'], ['Grep', 'pattern: "NewPullRequest"'], ['Read', 'routers/api/v1/repo/pull.go'],
    ['Grep', 'pattern: "CreatePullRequest"'], ['Read', 'routers/web/repo/pull.go'], ['Grep', 'pattern: "ProcReceive"'], ['Read', 'services/agit/agit.go'],
  ];
  const lines: TLine[] = [{at: 10, type: true, spans: [['> ', C.acc], ['Where do we create a pull request?', C.ink]], cps: 1.4}, {at: 50, gap: true, spans: []}];
  calls.forEach(([n, a], i) => lines.push({at: 58 + i * 20, spans: [['● ', C.acc], [n, C.ink], [`(${a})`, C.mid]]}));
  const count = Math.min(calls.length, Math.max(0, Math.floor((f - 58) / 20) + 1));
  return (
    <FadeScene dur={D.problem}>
      <Mark />
      <div style={{position: 'absolute', left: 90, top: 70, width: 760}}>
        <Label f={f}>The problem</Label>
        <Head f={f}>Without a map, an AI agent hunts page by page.</Head>
        <div style={{marginTop: 44, display: 'flex', alignItems: 'baseline', gap: 18, opacity: ramp(f, 40, 60)}}>
          <span style={{fontSize: 150, fontWeight: 900, color: C.acc, fontFamily: MONO}}>{count}</span>
          <span style={{fontSize: 34, color: C.soft}}>tool calls and still searching…</span>
        </div>
      </div>
      <div style={{position: 'absolute', right: 90, top: 190, opacity: ramp(f, 0, 20)}}><Term lines={lines} f={f} w={920} h={640} font={22} title="agent · illustrative" /></div>
      <Caption f={f} at={120}>A codebase can be a million lines long. Searching and reading one page at a time is slow, and it misses things.</Caption>
    </FadeScene>
  );
};

const SNIP = ['func NewPullRequest(ctx, opts) error {', '  issue.LoadPoster(ctx)', '  ReviewRequest(ctx, ...)', '  AddAssigneeIfNotAssigned(...)', '  notify.IssueChangeProjects(...)', '}'];
const Idea: React.FC = () => {
  const f = useCurrentFrame();
  const nodes = [[300, 90], [120, 220], [480, 230], [200, 400], [430, 430], [90, 560], [330, 600]];
  const edges = [[0, 1], [0, 2], [1, 3], [2, 4], [3, 5], [3, 6], [4, 6], [2, 3]];
  const scan = ramp(f, 40, 150);
  return (
    <FadeScene dur={D.idea}>
      <Mark />
      <div style={{position: 'absolute', left: 90, top: 70, width: 1200}}>
        <Label f={f}>The idea</Label>
        <Head f={f}>Draw a map of the code.</Head>
      </div>
      <div style={{position: 'absolute', left: 90, top: 290, width: 800, background: C.panel, border: `2px solid ${C.line}`, borderRadius: 26, padding: '30px 34px', fontFamily: MONO, fontSize: 26, lineHeight: '56px', opacity: ramp(f, 10, 30)}}>
        {SNIP.map((l, i) => <div key={i} style={{whiteSpace: 'pre', color: scan * 6 > i ? C.ink : C.dim, background: Math.abs(scan * 6 - i - 0.5) < 0.6 ? 'rgba(255,90,31,0.18)' : 'transparent', borderRadius: 8, padding: '0 8px'}}>{l}</div>)}
      </div>
      <div style={{position: 'absolute', left: 940, top: 230, width: 880, height: 640}}>
        <svg viewBox="0 0 600 700" style={{width: '100%', height: '100%', overflow: 'visible'}}>
          {edges.map(([a, b], i) => { const p = ramp(f, 90 + i * 8, 120 + i * 8); const A = nodes[a], B = nodes[b]; return <line key={i} x1={A[0]} y1={A[1]} x2={A[0] + (B[0] - A[0]) * p} y2={A[1] + (B[1] - A[1]) * p} stroke={C.mid} strokeWidth={3} opacity={0.7} strokeLinecap="round" />; })}
          {nodes.map(([x, y], i) => { const p = ramp(f, 60 + i * 9, 84 + i * 9); return <circle key={i} cx={x} cy={y} r={(i === 0 ? 22 : 15) * p} fill={i === 0 ? C.acc : C.panel2} stroke={C.ink} strokeWidth={3} />; })}
        </svg>
      </div>
      <div style={{position: 'absolute', left: 940, top: 810, display: 'flex', gap: 40, fontSize: 28, color: C.soft, opacity: ramp(f, 150, 175)}}>
        <span><b style={{color: C.ink}}>●</b> a function</span><span><b style={{color: C.ink}}>━</b> “who calls whom”</span>
      </div>
      <Caption f={f} at={110}>GraphRelay reads your code once and turns it into a map: every function is a dot, every “this uses that” is a line.</Caption>
    </FadeScene>
  );
};

const Setup: React.FC = () => {
  const f = useCurrentFrame();
  const bar = ramp(f, 100, 190);
  const lines: TLine[] = [
    {at: 8, type: true, spans: [['$ ', C.acc], ['gr init', C.ink]], cps: 0.9},
    {at: 50, spans: [['  # registers this folder as a project', C.dim]]},
    {at: 62, gap: true, spans: []},
    {at: 70, type: true, spans: [['$ ', C.acc], ['gr start', C.ink]], cps: 0.9},
    {at: 100, spans: [["  Indexing 'gitea'…  ", C.soft], [`${'█'.repeat(Math.round(bar * 24))}${'░'.repeat(24 - Math.round(bar * 24))}`, C.acc2]]},
    {at: 196, spans: [['  Done: 3536 files, 103303 symbols', C.ink]]},
    {at: 206, gap: true, spans: []},
    {at: 214, type: true, spans: [['$ ', C.acc], ['gr setup', C.ink]], cps: 0.9},
    {at: 236, spans: [['  # auto-detects Claude Code, Cursor, OpenCode, Amp, Codex', C.dim]]},
  ];
  const steps = ['Register the project', 'Build the map, keep it fresh', 'Connect your AI agent'];
  const act = f < 66 ? 0 : f < 210 ? 1 : 2;
  return (
    <FadeScene dur={D.setup}>
      <Mark />
      <div style={{position: 'absolute', left: 90, top: 70}}><Label f={f}>Setup</Label><Head f={f}>Three commands.</Head></div>
      <div style={{position: 'absolute', left: 90, top: 250, opacity: ramp(f, 0, 20)}}><Term lines={lines} f={f} w={1120} h={560} font={26} title="~/code/gitea" /></div>
      <div style={{position: 'absolute', left: 1280, top: 290, width: 560}}>
        {steps.map((s, i) => (
          <div key={i} style={{display: 'flex', gap: 22, alignItems: 'center', margin: '0 0 56px', opacity: act === i ? 1 : 0.35, transform: `translateX(${act === i ? 0 : -6}px)`, transition: 'none'}}>
            <div style={{width: 58, height: 58, borderRadius: 29, border: `2px solid ${act === i ? C.acc : C.line2}`, color: act === i ? C.acc : C.mid, display: 'grid', placeItems: 'center', fontFamily: MONO, fontSize: 26}}>{i + 1}</div>
            <div style={{fontSize: 32, fontWeight: 600}}>{s}</div>
          </div>
        ))}
      </div>
      <Caption f={f} at={40}>Everything runs on your own computer. After the first index, the map updates itself when you save a file.</Caption>
    </FadeScene>
  );
};

// ---- Claude Code session: terminal + graph in sync ----
const G = {
  c: [440, 300], prod: [['CreatePullRequest', 90, 120], ['CompareAndPullRequestPost', 70, 300], ['ProcReceive', 110, 480]] as [string, number, number][],
  hop2: ['HookProcReceive', 40, 600] as [string, number, number], callees: [['LoadHeadRepo', 780, 110], ['AddAssigneeIfNotAssigned', 800, 230], ['PullRequestCodeOwnersReview', 790, 350], ['ReviewRequest', 780, 470]] as [string, number, number][],
};
const Session: React.FC = () => {
  const f = useCurrentFrame();
  const T = {s: 130, i: 290, t: 450, a: 600};
  const lines: TLine[] = [
    {at: 12, type: true, cps: 1.5, spans: [['> ', C.acc], ['Where do we create a pull request, and what breaks if I change it?', C.ink]]},
    {at: 90, gap: true, spans: []},
    {at: T.s - 30, type: true, cps: 2.4, spans: [['● ', C.acc], ['graphrelay', C.ink], [' · search_code ', C.soft], ['(intent: "create a new pull request")', C.mid]]},
    ...SEARCH.map((r, i): TLine => ({at: T.s + i * 8, spans: [['  ⎿ ', C.dim], [pad(r[0], 20), C.ink], [pad(r[1], 38), C.mid], [r[2], C.acc2]]})),
    {at: T.i - 40, gap: true, spans: []},
    {at: T.i - 30, type: true, cps: 2.4, spans: [['● ', C.acc], ['graphrelay', C.ink], [' · inspect ', C.soft], ['(symbol: "NewPullRequest")', C.mid]]},
    {at: T.i, spans: [['  ⎿ ', C.dim], ['services/pull/pull.go:57-200', C.ink]]},
    {at: T.i + 8, spans: [['     callers  CreatePullRequest, CompareAndPullRequestPost, ProcReceive', C.mid]]},
    {at: T.i + 16, spans: [['     callees  LoadHeadRepo, AddAssigneeIfNotAssigned, …', C.mid]]},
    {at: T.t - 40, gap: true, spans: []},
    {at: T.t - 30, type: true, cps: 2.4, spans: [['● ', C.acc], ['graphrelay', C.ink], [' · trace_impact ', C.soft], ['(symbol: "NewPullRequest", depth: 3)', C.mid]]},
    {at: T.t, spans: [['  ⎿ ', C.dim], ['14 functions affected: ', C.ink], ['4 in production code', C.acc2], [', 10 in tests', C.mid]]},
    {at: T.a - 30, gap: true, spans: []},
    {at: T.a - 20, type: true, cps: 2.6, spans: [['● ', C.acc], ['NewPullRequest (services/pull/pull.go) creates it.', C.ink]]},
    {at: T.a + 14, type: true, cps: 2.6, spans: [['  Called by the API, the web UI and agit. A change reaches', C.soft]]},
    {at: T.a + 40, type: true, cps: 2.6, spans: [['  4 production functions and 10 tests.', C.soft]]},
  ];
  const ph = f < T.i ? 0 : f < T.t ? 1 : 2;
  const lit = (a: number) => ramp(f, a, a + 30);
  const sx = (x: number) => x, W = 900;
  const nodeP = (name: string, x: number, y: number, kind: 'prod' | 'callee' | 'hop2', i: number) => {
    const on = kind === 'callee' ? lit(T.i + 10) : kind === 'prod' ? Math.max(lit(T.i + 4 + i * 4), 0) : lit(T.t + 24);
    const hot = ramp(f, T.t + 4 + (kind === 'hop2' ? 22 : i * 5), T.t + 24 + (kind === 'hop2' ? 22 : i * 5));
    const fill = kind === 'callee' ? (on > 0.5 ? C.ink : C.panel2) : hot > 0.5 ? C.acc2 : on > 0.5 ? C.ink : C.panel2;
    return {name, x, y, fill, glow: kind === 'callee' ? on : Math.max(on * 0.4, hot)};
  };
  const items = [
    ...G.prod.map(([n, x, y], i) => nodeP(n, x, y, 'prod', i)),
    nodeP(G.hop2[0], G.hop2[1], G.hop2[2], 'hop2', 0),
    ...G.callees.map(([n, x, y], i) => nodeP(n, x, y, 'callee', i)),
  ];
  const [cx, cy] = G.c;
  const centerOn = ramp(f, T.s, T.s + 24);
  return (
    <FadeScene dur={D.session}>
      <Mark />
      <div style={{position: 'absolute', left: 90, top: 50}}><Label f={f}>In Claude Code</Label></div>
      <div style={{position: 'absolute', left: 70, top: 130, opacity: ramp(f, 0, 20)}}>
        <Term lines={lines} f={f} w={1080} h={760} font={21} title="claude · ~/code/gitea" />
      </div>
      <div style={{position: 'absolute', left: 1170, top: 150, width: 700, height: 700}}>
        <svg viewBox="0 0 900 720" style={{width: '100%', height: '100%', overflow: 'visible'}}>
          <defs><filter id="g2" x="-100%" y="-100%" width="300%" height="300%"><feGaussianBlur stdDeviation="10" /></filter></defs>
          {[...G.prod.map(([, x, y]) => [x, y, cx, cy, 0] as number[]), [G.hop2[1], G.hop2[2], G.prod[2][1], G.prod[2][2], 1], ...G.callees.map(([, x, y]) => [cx, cy, x, y, 2] as number[])].map(([x1, y1, x2, y2, k], i) => (
            <line key={i} x1={x1} y1={y1} x2={x2} y2={y2} stroke={k === 2 ? (ph >= 1 ? C.ink : C.line2) : ph >= 2 ? C.acc2 : ph >= 1 ? C.ink : C.line2} strokeWidth={k === 1 ? 2.5 : 3} opacity={0.35 + 0.6 * (k === 2 ? lit(T.i + 10) : ph >= 1 ? lit(k === 1 ? T.t + 20 : T.i + 4) : 0)} />
          ))}
          {Array.from({length: 10}).map((_, i) => { const a = (i / 10) * Math.PI * 2; const x = 450 + Math.cos(a) * 400, y = 640 + Math.sin(a) * 0; const px = 120 + i * 70, py = 660; return <circle key={i} cx={px} cy={py} r={9} fill={ramp(f, T.t + 40 + i * 3, T.t + 60 + i * 3) > 0.5 ? C.acc : C.panel2} stroke={C.line2} strokeWidth={2} opacity={0.5 + 0.5 * ramp(f, T.t + 30, T.t + 50)} />; })}
          <text x={120} y={700} fill={C.dim} fontFamily={MONO} fontSize={20} opacity={ramp(f, T.t + 40, T.t + 70)}>10 tests</text>
          {items.map((n, i) => (
            <g key={i}>
              {n.glow > 0.3 ? <circle cx={n.x} cy={n.y} r={26} fill={n.fill} filter="url(#g2)" opacity={0.5} /> : null}
              <circle cx={n.x} cy={n.y} r={15} fill={n.fill} stroke={C.ink} strokeWidth={3} />
              <text x={n.x} y={n.y + 38} fill={C.soft} fontFamily={MONO} fontSize={19} textAnchor={n.x > 600 ? 'end' : 'start'} stroke={C.bg} strokeWidth={6} paintOrder="stroke" dx={n.x > 600 ? 12 : -12} opacity={0.4 + 0.6 * Math.min(1, n.glow + 0.2)}>{n.name}</text>
            </g>
          ))}
          <circle cx={cx} cy={cy} r={(26 + 6 * Math.sin(f / 10) * centerOn) * centerOn} fill={C.acc} stroke={C.ink} strokeWidth={4} />
          <circle cx={cx} cy={cy} r={50 * ramp(f, T.s, T.s + 30)} fill="none" stroke={C.acc} strokeWidth={2} opacity={0.5 * (1 - ramp(f, T.s, T.s + 30))} />
          <text x={cx} y={cy - 44} fill={C.ink} fontFamily={MONO} fontSize={22} textAnchor="middle" stroke={C.bg} strokeWidth={6} paintOrder="stroke" opacity={centerOn}>NewPullRequest</text>
        </svg>
      </div>
      <div style={{position: 'absolute', left: 1170, top: 880, width: 700, fontSize: 26, color: C.mid, opacity: ramp(f, 20, 40), fontFamily: MONO}}>
        {ph === 0 ? '① find by words' : ph === 1 ? '② follow callers & callees' : '③ trace what breaks'}
      </div>
      <div style={{position: 'absolute', left: 70, bottom: 36, fontSize: 20, color: C.dim, fontFamily: MONO}}>real output from Gitea · session layout illustrative</div>
    </FadeScene>
  );
};

const TOOLS: [string, string, string][] = [
  ['search_code', 'Find it', 'Ask in plain language, get the relevant code with its callers and callees.'],
  ['inspect', 'Look closer', 'Open one function or file: definition, who uses it, what it uses.'],
  ['trace_impact', 'What breaks?', 'Before you change something, see which functions call it, directly and indirectly.'],
  ['watch_status', 'Is it fresh?', 'Check the map matches your latest edits.'],
];
const Tools: React.FC = () => {
  const f = useCurrentFrame();
  return (
    <FadeScene dur={D.tools}>
      <Mark />
      <div style={{position: 'absolute', left: 90, top: 70}}><Label f={f}>What the agent gets</Label><Head f={f}>Four tools. One connection.</Head></div>
      <div style={{position: 'absolute', left: 90, right: 90, top: 280}}>
        {TOOLS.map(([n, t, d], i) => {
          const p = ramp(f, 30 + i * 28, 56 + i * 28);
          return (
            <div key={n} style={{display: 'flex', alignItems: 'center', gap: 36, padding: '22px 0', borderBottom: `1.5px solid ${C.line}`, opacity: p, transform: `translateX(${(1 - p) * 60}px)`}}>
              <div style={{fontFamily: MONO, color: C.acc, fontSize: 28, width: 70}}>0{i + 1}</div>
              <div style={{fontFamily: MONO, fontSize: 36, width: 380}}>{n}</div>
              <div style={{fontSize: 40, fontWeight: 700, width: 330}}>{t}</div>
              <div style={{fontSize: 28, color: C.soft, flex: 1, lineHeight: 1.3}}>{d}</div>
            </div>
          );
        })}
      </div>
      <Caption f={f} at={150}>The agent connects through a standard plug called MCP. Think of four buttons on a remote.</Caption>
    </FadeScene>
  );
};

const Trust: React.FC = () => {
  const f = useCurrentFrame();
  const items = ['One small program.', 'Stays on your machine.', 'Always up to date.'];
  return (
    <FadeScene dur={D.trust}>
      <Mark />
      <div style={{position: 'absolute', left: 90, top: 70}}><Label f={f}>Why it feels safe</Label></div>
      <div style={{position: 'absolute', left: 90, top: 190}}>
        {items.map((t, i) => {
          const p = ramp(f, 14 + i * 26, 40 + i * 26);
          return <div key={t} style={{fontSize: 96, fontWeight: 800, letterSpacing: -2, lineHeight: 1.15, opacity: p, transform: `translateY(${(1 - p) * 40}px)`, color: i === 1 ? C.ink : C.soft}}>{t}</div>;
        })}
      </div>
      <div style={{position: 'absolute', left: 90, right: 90, bottom: 70, opacity: ramp(f, 120, 150), borderTop: `1.5px solid ${C.line}`, paddingTop: 26, fontSize: 30, color: C.mid, lineHeight: 1.4}}>
        <b style={{color: C.acc2}}>Honest status.</b> On a 30-question benchmark, an agent with GraphRelay scores about the same as one without — the gap is inside the noise. We publish the numbers, good or not.
      </div>
    </FadeScene>
  );
};

const Outro: React.FC = () => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const s = spring({frame: f - 4, fps, config: {damping: 16}});
  const cmds = ['gr init', 'gr start', 'gr setup'];
  return (
    <FadeScene dur={D.outro}>
      <div style={{position: 'absolute', top: 150, width: '100%', display: 'flex', justifyContent: 'center', alignItems: 'center', gap: 30, transform: `scale(${0.85 + 0.15 * s})`, opacity: s}}>
        <Img src={logo} style={{width: 150, height: 150}} />
        <div style={{fontSize: 130, fontWeight: 900, letterSpacing: -3}}>Graph<span style={{color: C.mid}}>Relay</span></div>
      </div>
      <div style={{position: 'absolute', top: 400, width: '100%', textAlign: 'center', fontSize: 44, color: C.soft, opacity: ramp(f, 24, 48)}}>Give your AI agent a map.</div>
      <div style={{position: 'absolute', top: 580, width: '100%', display: 'flex', justifyContent: 'center', gap: 28}}>
        {cmds.map((c, i) => { const p = ramp(f, 50 + i * 16, 76 + i * 16); return <div key={c} style={{opacity: p, transform: `translateY(${(1 - p) * 28}px)`, background: C.panel, border: `2px solid ${C.line}`, borderRadius: 18, padding: '22px 40px', fontFamily: MONO, fontSize: 46}}><span style={{color: C.acc}}>$ </span>{c}</div>; })}
      </div>
    </FadeScene>
  );
};

const SCENES: Record<string, React.FC> = {title: Title, problem: Problem, idea: Idea, setup: Setup, session: Session, tools: Tools, trust: Trust, outro: Outro};
export const IntroV2: React.FC = () => (
  <AbsoluteFill style={{background: C.bg}}>
    <Backdrop />
    {order.map((k) => { const S = SCENES[k]; return <Sequence key={k} from={starts[k]} durationInFrames={D[k]}><S /></Sequence>; })}
  </AbsoluteFill>
);
