/* Toolbench file viewer. One static script over the JSON in #cfg (built by search-core/src/viewer/mod.rs).
   Text is drawn by one virtual-row engine (only the rows on screen exist), whether the lines are embedded in the page or
   read in blocks from the local reader. Highlighting is the report's own: Rust sends the match spans, this draws them. */
(function () {
  'use strict';
  var $ = function (id) { return document.getElementById(id); };
  var CFG = JSON.parse($('cfg').textContent);
  var EMB = CFG.embedded || null, RD = CFG.reader || null, PDF = CFG.kind === 'pdf';
  var esc = function (s) { return s.replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; }); };
  var ws = function (s) { return esc(s.replace(/\s+/g, ' ')); };
  var fmt = function (n) { return Math.round(n).toLocaleString('en-US'); };
  var root = document.documentElement;
  var N = EMB ? EMB.lines.length : (CFG.total || 0);
  var NF = 6, FILT = (CFG.filters || []).slice(0, 12);
  var SD = CFG.structure || null;   // the Office structure of the file, when it has one
  var ST = { on: false, el: null, inner: null, units: [], starts: [], shown: 0, key: '', sls: null, dirty: false, ref: new Map(), boxEl: null };

  /* ---------- preferences (localStorage may be refused on file:// pages) ---------- */
  var pref = { on: [true, true, true, true, true, true], railw: 272, collapsed: false, fs: 1, mo: false, wrap: false, str: true, theme: '' };
  try {
    var sv = JSON.parse(localStorage.getItem('toolbench-viewer') || 'null');
    if (sv && typeof sv === 'object') Object.keys(pref).forEach(function (k) { if (k in sv && typeof sv[k] === typeof pref[k]) pref[k] = sv[k]; });
  } catch (e) { /* no storage: defaults */ }
  if (!Array.isArray(pref.on)) pref.on = [];
  pref.on = [0, 1, 2, 3, 4, 5].map(function (k) { return pref.on[k] !== false; });
  if (pref.theme !== 'light' && pref.theme !== 'dark') pref.theme = '';
  var FS = [[11.5, 19], [12.5, 21], [14, 24], [15.5, 27]];
  if (!(pref.fs >= 0 && pref.fs < FS.length)) pref.fs = 1;
  function save() { try { localStorage.setItem('toolbench-viewer', JSON.stringify(pref)); } catch (e) { /* ignore */ } }
  var on = pref.on, onMask = 0;
  var LH = FS[pref.fs][1], LCAP = 10000, CTX = 2, EXSTEP = 20, RH = 30, MAXH = 8000000, BS = 25;

  /* ---------- small helpers ---------- */
  var toastT = 0;
  function toast(m) { var t = $('toast'); t.textContent = m; t.classList.add('show'); clearTimeout(toastT); toastT = setTimeout(function () { t.classList.remove('show'); }, 1800); }
  function copyText(t) {
    var done = function () { return true; };
    try { if (navigator.clipboard && window.isSecureContext) { navigator.clipboard.writeText(t).catch(function () { legacyCopy(t); }); return done(); } } catch (e) { /* fall through */ }
    return legacyCopy(t);
  }
  function legacyCopy(t) {
    var a = document.createElement('textarea'); a.value = t; a.style.cssText = 'position:fixed;top:0;left:0;opacity:0'; document.body.appendChild(a); a.select();
    var ok = false; try { ok = document.execCommand('copy'); } catch (e) { /* ignore */ } a.remove(); return ok;
  }
  function lower(a, v) { var lo = 0, hi = a.length; while (lo < hi) { var m = (lo + hi) >> 1; if (a[m] < v) lo = m + 1; else hi = m; } return lo; }   // first index with a[i] >= v
  function upper(a, v) { var lo = 0, hi = a.length; while (lo < hi) { var m = (lo + hi) >> 1; if (a[m] <= v) lo = m + 1; else hi = m; } return lo; }   // first index with a[i] > v

  /* ---------- hits ---------- */
  var HITS = [], HMASK = [], SPANS = new Map(), ACT = [], LIST = [], qHits = null, LV = 0;
  function maskOf(n) { var i = lower(HITS, n); return HITS[i] === n ? HMASK[i] : 0; }
  function isHit(n) { return (maskOf(n) & onMask) !== 0; }
  function recomputeOn() { onMask = 0; for (var k = 0; k < NF; k++) if (on[k]) onMask |= 1 << k; for (k = 0; k < NF; k++) root.classList.toggle('off' + (k + 1), !on[k]); }
  function rebuildAct() { ACT = []; for (var i = 0; i < HITS.length; i++) if (HMASK[i] & onMask) ACT.push(HITS[i]); }
  function slotOf(n) { var m = maskOf(n) & onMask; for (var k = 0; k < NF; k++) if (m >> k & 1) return k; return 0; }
  var hitColor = function (n) { return 'var(--c' + (slotOf(n) + 1) + ')'; };

  /* ---------- line data: embedded array, or blocks read from the local reader ---------- */
  var blocks = new Map(), inflight = new Set(), readerUp = true, maxLen = 0, fetchT = 0, snipT = 0, snipCache = new Map(), snipWant = new Set();
  function lineData(n) {
    if (n < 1 || n > N) return null;
    if (EMB) return { t: EMB.lines[n - 1], sp: SPANS.get(n) };
    var b = Math.floor((n - 1) / BS), blk = blocks.get(b), r = blk && blk[n - 1 - b * BS];
    return r ? { t: r[1], sp: r[3] } : null;
  }
  function rget(path) {
    return fetch(RD.base + path, { cache: 'no-store' }).then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json(); });
  }
  function readerFailed() { if (!readerUp) return; readerUp = false; paintReader(); }
  function paintReader() {
    if (!RD) return;
    $('readerDown').hidden = readerUp; $('rdPill').classList.toggle('down', !readerUp);
    $('rdTxt').textContent = readerUp ? 'Local reader ' + location.host : 'Local reader stopped';
  }
  function storeRows(rows) {
    rows.forEach(function (r) {
      var b = Math.floor((r[0] - 1) / BS), blk = blocks.get(b);
      if (!blk) { blk = new Array(BS); blocks.set(b, blk); }
      blk[r[0] - 1 - b * BS] = r; if (r[1].length > maxLen) maxLen = Math.min(LCAP, r[1].length);
    });
    var cap = Math.max(160, Math.ceil(box ? 4 * box.clientHeight / LH : 0) + 40);
    while (blocks.size > cap) blocks.delete(blocks.keys().next().value);
    setWidth();
  }
  function schedFetch(want) {
    clearTimeout(fetchT);
    fetchT = setTimeout(function () {
      var bs = Array.from(want).filter(function (b) { return !blocks.has(b) && !inflight.has(b); }).sort(function (a, b) { return a - b; });
      if (!bs.length || !readerUp) return;
      var i = 0;
      while (i < bs.length) {
        var j = i; while (j + 1 < bs.length && bs[j + 1] === bs[j] + 1 && j - i < 39) j++;
        (function (b0, b1) {
          var ids = []; for (var b = b0; b <= b1; b++) { inflight.add(b); ids.push(b); }
          rget('/lines?from=' + (b0 * BS + 1) + '&count=' + ((b1 - b0 + 1) * BS)).then(function (d) {
            ids.forEach(function (b) { inflight.delete(b); }); storeRows(d.l); readerUp = true; paintReader(); paint(); listPaint();
          }).catch(function () { ids.forEach(function (b) { inflight.delete(b); }); readerFailed(); });
        })(bs[i], bs[j]);
        i = j + 1;
      }
    }, 30);
  }
  /* the lines n..n+count-1 as [n, text, mask, spans] rows */
  function ensureLines(from, count) {
    if (EMB) {
      var out = []; for (var n = Math.max(1, from); n < from + count && n <= N; n++) out.push([n, EMB.lines[n - 1], 0, SPANS.get(n)]);
      return Promise.resolve(out);
    }
    return rget('/lines?from=' + Math.max(1, from) + '&count=' + count).then(function (d) { return d.l; });
  }

  /* ---------- text of a row: spans from Rust, find matches from the find box ---------- */
  var fre = null;
  function findRanges(t) {
    var r = [], m; if (!fre) return r; fre.lastIndex = 0;
    while ((m = fre.exec(t)) && r.length < 500) { if (!m[0].length) { fre.lastIndex++; continue; } r.push([m.index, m.index + m[0].length]); }
    return r;
  }
  function renderText(t, sp, nofind) {
    var fr = nofind ? [] : findRanges(t), mk = sp ? sp.filter(function (s) { return s[0] < t.length; }) : [];
    if (!mk.length && !fr.length) return esc(t);
    var pts = [0, t.length];
    mk.forEach(function (s) { pts.push(s[0], Math.min(s[1], t.length)); }); fr.forEach(function (r) { pts.push(r[0], Math.min(r[1], t.length)); });
    pts = pts.filter(function (v, i, a) { return a.indexOf(v) === i; }).sort(function (a, b) { return a - b; });
    var out = '';
    for (var i = 0; i + 1 < pts.length; i++) {
      var a = pts[i], b = pts[i + 1], seg = esc(t.slice(a, b));
      var m = mk.find(function (s) { return s[0] <= a && s[1] >= b; });
      if (m) seg = '<mark' + (m[2] ? ' class="m' + (m[2] + 1) + '"' : '') + '>' + seg + '</mark>';
      if (fr.some(function (r) { return r[0] <= a && r[1] >= b; })) seg = '<span class="fm">' + seg + '</span>';
      out += seg;
    }
    return out;
  }
  function lineHtml(n, d) {
    var t = d.t, more = '';
    if (t.length > LCAP) { more = '<button class="more" type="button" data-n="' + n + '" title="Show the whole line">⤢ ' + fmt(t.length) + ' chars</button>'; t = t.slice(0, LCAP); }
    return '<span class="no" title="Click to select this line and copy a link to it">' + fmt(n) + '</span><span>' + more + renderText(t, d.sp) + '</span>';
  }

  /* ---------- virtual scrolling: only the rows on screen exist; a list taller than the browser allows is mapped onto a shorter scrollbar.
     Rows are one height (rh), or, with P (prefix sums of the row heights, used by wrap), each their own; P is never scaled. ---------- */
  function pfx(P, y) {   // the fractional row at offset y
    var n = P.length - 1; if (y <= 0) return 0; if (y >= P[n]) return n;
    var lo = 0, hi = n - 1; while (lo < hi) { var m = (lo + hi + 1) >> 1; if (P[m] <= y) lo = m; else hi = m - 1; }
    return lo + (y - P[lo]) / (P[lo + 1] - P[lo] || 1);
  }
  function vgeom(el, total, rh, P) {
    var vh = el.clientHeight;
    if (P) { var st = el.scrollTop, k = pfx(P, st); return { vh: vh, rh: rh, total: total, H: Math.max(P[total], vh), scaled: false, P: P, vis: Math.max(1, pfx(P, st + vh) - k), kTop: k }; }
    var nat = total * rh, H = Math.min(nat, MAXH), scaled = nat > MAXH;
    return { vh: vh, rh: rh, total: total, H: Math.max(H, vh), scaled: scaled, vis: vh / rh, kTop: scaled ? el.scrollTop / Math.max(1, H - vh) * Math.max(0, total - vh / rh) : el.scrollTop / rh };
  }
  function vscroll(el, g, want) {
    if (g.P) {
      var n = g.total, w = Math.max(0, Math.min(want, n)), i = Math.min(Math.floor(w), n), j = Math.min(i + 1, n);
      el.scrollTop = Math.max(0, Math.min(g.P[i] + (w - i) * (g.P[j] - g.P[i]), Math.max(0, g.H - g.vh))); return;
    }
    var maxK = Math.max(0, g.total - g.vis); want = Math.max(0, Math.min(want, maxK));
    el.scrollTop = g.scaled ? (maxK ? want / maxK * (g.H - g.vh) : 0) : want * g.rh;
  }
  function vlayer(sp, vr, el, total, rh, P) { sp.style.height = Math.max(P ? P[total] : Math.min(total * rh, MAXH), el.clientHeight) + 'px'; vr.style.height = el.clientHeight + 'px'; return vgeom(el, total, rh, P); }

  /* ---------- text pane: rows are all lines, or only the hit lines with context and folded gaps ---------- */
  var box = null, sp = null, vr = null, nodes = new Map(), RGEN = 0, curLine = 0, sel = null, charW = 7.5;
  var MO = { on: false, gen: 0, ex: {}, GS: [], GE: [], pref: null, J: 0, R: 0 };
  var rowCount = function () { return MO.on ? MO.R : N; };
  var rowAt = function (r) { return MO.on ? moRow(r) : { n: r + 1 }; };
  var rowOfLine = function (n) { return MO.on ? moRowOfLine(n) : n - 1; };
  function moGap(j) { return MO.GS[j] - (j ? MO.GE[j - 1] : 0) - 1; }
  function moGapRows(j) { var g = moGap(j); if (g <= 0) return 0; var x = MO.ex[j] || 0; return 2 * x >= g ? g : 2 * x + 1; }
  function moPrefix(from) { var P = MO.pref; for (var j = from; j <= MO.J; j++) P[j + 1] = P[j] + moGapRows(j) + (MO.GE[j] - MO.GS[j] + 1); MO.R = P[MO.J + 1]; }
  function moBuild() {
    var GS = [], GE = [];
    LIST.forEach(function (n) {
      var s = Math.max(1, n - CTX), e = Math.min(N, n + CTX), j = GS.length - 1;
      if (j >= 0 && s <= GE[j] + 1) GE[j] = Math.max(GE[j], e); else { GS.push(s); GE.push(e); }
    });
    MO.J = GS.length; GS.push(N + 1); GE.push(N);   // the extra unit is the gap after the last hit
    MO.GS = GS; MO.GE = GE; MO.ex = {}; MO.pref = new Float64Array(MO.J + 2); moPrefix(0); MO.gen++;
  }
  function moUnitOf(r) { var P = MO.pref, lo = 0, hi = MO.J; while (lo < hi) { var m = (lo + hi + 1) >> 1; if (P[m] <= r) lo = m; else hi = m - 1; } return lo; }
  function moRow(r) {
    var j = moUnitOf(r), o = r - MO.pref[j], g = moGap(j), x = MO.ex[j] || 0, gr = moGapRows(j), base = j ? MO.GE[j - 1] : 0;
    if (o < gr) {
      if (2 * x >= g || o < x) return { n: base + 1 + o };
      if (o === x) return { fold: j, hidden: g - 2 * x };
      return { n: MO.GS[j] - x + (o - x - 1) };
    }
    return { n: MO.GS[j] + (o - gr) };
  }
  function moRowOfLine(n) {
    var GS = MO.GS, lo = 0, hi = MO.J; while (lo < hi) { var m = (lo + hi) >> 1; if (GS[m] > n) hi = m; else lo = m + 1; }
    var u = lo;   // first unit starting after n
    if (u > 0 && n <= MO.GE[u - 1]) return MO.pref[u - 1] + moGapRows(u - 1) + (n - MO.GS[u - 1]);
    var base = u ? MO.GE[u - 1] : 0, g = moGap(u), top = n - base, bot = MO.GS[u] - n, x = Math.min(top, bot);   // n sits in a fold: open just enough of it
    if ((MO.ex[u] || 0) < x) { MO.ex[u] = x; moPrefix(u); MO.gen++; }
    var e = MO.ex[u] || 0;
    return MO.pref[u] + (2 * e >= g || top <= e ? top - 1 : e + 1 + (e - bot));
  }
  var bgeom = function () { return vgeom(box, rowCount(), LH, wrapP()); };
  var topLine = function () {
    if (ST.on) return strTop();
    var d = rowAt(Math.max(0, Math.min(rowCount() - 1, Math.floor(bgeom().kTop)))); return d.n || (MO.GS[d.fold] || 1);
  };
  /* the first and last line in view */
  function viewLines() {
    if (ST.on) { var r = ST.el.getBoundingClientRect(), a = strTop(), b = 0; for (var y = r.bottom - 12, k = 0; !b && k < 8; k++, y -= 14) b = strAt(y); return [a, Math.max(a, b || a)]; }
    if (!box) return [1, 1];
    var g = bgeom(), a2 = topLine(); return [a2, Math.min(N, a2 + Math.ceil(g.vis) - 1)];
  }

  /* the line a view change (wrap, text size, matches only, structured) keeps in place: the selected line while it is on screen, else the first line in view */
  function anchorLine() { var vl = viewLines(); return curLine >= vl[0] && curLine <= vl[1] ? curLine : vl[0]; }

  /* ---------- wrap: rows of different heights (embedded text only). A row's height is its estimate (lines of the monospace width) until it has been drawn and measured. ---------- */
  var WR = { on: false, P: null, key: '', mh: null, cpl: 80, v: 0, fix: 0 };
  var canWrap = function () { return !!EMB && !PDF && N > 0 && !ST.on; };
  var wrapOn = function () { return WR.on && canWrap(); };
  var cplNow = function () { return Math.max(8, Math.floor((box.clientWidth - 76 - 16) / charW)); };
  function wrapReset() { WR.mh = new Float32Array(N + 2); WR.cpl = cplNow(); WR.P = null; WR.v++; }
  function rowH(r) {
    var d = rowAt(r); if (d.fold !== undefined) return LH;
    var m = WR.mh[d.n]; if (m) return m;
    return Math.max(1, Math.ceil(Math.min(EMB.lines[d.n - 1].length, LCAP) / WR.cpl)) * LH;
  }
  function wrapP() {
    if (!box || !wrapOn()) return null;
    var R = rowCount(), key = [R, MO.on, MO.gen, LH, WR.cpl, WR.v].join(':');
    if (WR.P && WR.key === key) return WR.P;
    var P = new Float64Array(R + 1); for (var r = 0; r < R; r++) P[r + 1] = P[r] + rowH(r);
    WR.P = P; WR.key = key; return P;
  }
  function wrapTooTall() { var P = wrapP(); return !!P && P[P.length - 1] > MAXH; }
  function wrapRefuse() { WR.on = false; if (box) box.classList.remove('wrapon'); toast('This file is too long to wrap'); syncTools(); }
  /* after drawing: correct the estimates with the real heights, keeping the row at the top of the window where it is */
  function wrapMeasure() {
    if (!wrapOn() || WR.fix) return;
    var changed = false;
    nodes.forEach(function (nd, r) {
      var d = rowAt(r); if (d.fold !== undefined || !nd._k || nd.classList.contains('skel')) return;
      var h = nd.offsetHeight; if (h && Math.abs(h - rowH(r)) > 0.5) { WR.mh[d.n] = h; changed = true; }
    });
    if (!changed) return;
    var k = pfx(WR.P, box.scrollTop), r0 = Math.floor(k), fr = k - r0;
    WR.v++; var P = wrapP();
    if (P[P.length - 1] > MAXH) { wrapRefuse(); paint(); return; }
    box.scrollTop = P[r0] + fr * (P[Math.min(r0 + 1, P.length - 1)] - P[r0]);
    WR.fix = requestAnimationFrame(function () { WR.fix = 0; paint(); afterScroll(); });
  }
  function wrapResize() {   // the width changed: lines wrap differently, so every measured height is stale
    if (!wrapOn() || !box || cplNow() === WR.cpl) return;
    var at = anchorLine(); wrapReset(); RGEN++; revealRow(rowOfLine(at), true); paint();
  }
  function setWrap(v) {
    if (v && !canWrap()) return;
    var at = box ? anchorLine() : 1;
    WR.on = v; pref.wrap = v; save();
    if (box) {
      box.classList.toggle('wrapon', wrapOn()); if (wrapOn()) wrapReset();
      if (wrapOn() && wrapTooTall()) { wrapRefuse(); setWidth(); return; }
      setWidth(); RGEN++; revealRow(rowOfLine(at), true); paint(); afterScroll();
    }
    syncTools();
  }


  function measureChar() {
    var s = document.createElement('span'); s.style.cssText = 'position:absolute;visibility:hidden;white-space:pre;font:inherit'; s.textContent = new Array(101).join('M');
    box.appendChild(s); charW = s.getBoundingClientRect().width / 100 || 7.5; s.remove();
  }
  function setWidth() { if (sp) sp.style.minWidth = wrapOn() ? '0' : Math.ceil(76 + 24 + maxLen * charW + (maxLen >= LCAP ? 110 : 0)) + 'px'; }

  var paintQ = 0;
  function schedPaint() { if (!paintQ) paintQ = requestAnimationFrame(function () { paintQ = 0; if (ST.on) strMore(); else paint(); afterScroll(); }); }
  function rowKey(d, ready) {
    return d.fold !== undefined ? 'f' + d.fold + ':' + d.hidden : 'n' + d.n + ':' + (ready ? 1 : 0) + ':' + (d.n === curLine ? 1 : 0) + ':' + (inSel(d.n) ? 1 : 0) + ':' + (isHit(d.n) ? 1 : 0) + ':' + RGEN + ':' + MO.gen;
  }
  function paint() {
    if (ST.on) { strPaint(); return; }
    if (!box) return;
    var R = rowCount(), g = vlayer(sp, vr, box, R, LH, wrapP());
    var first = Math.max(0, Math.floor(g.kTop) - 2), last = Math.min(R - 1, Math.ceil(g.kTop + g.vis) + 2), want = new Set(), seen = new Set();
    nodes.forEach(function (nd, r) { if (r < first || r > last) { nd.remove(); nodes.delete(r); } });
    for (var r = first; r <= last; r++) {
      var d = rowAt(r), dd = d.fold === undefined ? lineData(d.n) : null, nd = nodes.get(r), k = rowKey(d, !!dd);
      if (d.fold === undefined) { var b = Math.floor((d.n - 1) / BS); seen.add(b); if (!dd) want.add(b); }
      if (!nd) { nd = document.createElement('div'); vr.appendChild(nd); nodes.set(r, nd); }
      if (nd._k !== k) {
        nd._k = k;
        if (d.fold !== undefined) {
          nd.className = 'foldrow'; nd.removeAttribute('data-n');
          nd.innerHTML = '<button type="button" class="fold" data-j="' + d.fold + '">⋯ ' + fmt(d.hidden) + (d.hidden === 1 ? ' line' : ' lines') + ' hidden <em>click to show ' + EXSTEP + ' more each side, shift-click for all</em></button>';
        } else {
          nd.setAttribute('data-n', d.n);
          if (dd) { nd.className = 'ln' + (isHit(d.n) ? ' hit' : '') + (d.n === curLine ? ' cur' : '') + (inSel(d.n) ? ' sel' : ''); nd.innerHTML = lineHtml(d.n, dd); }
          else { nd.className = 'ln skel'; nd.innerHTML = '<span class="no">' + fmt(d.n) + '</span><span><i class="skb" style="width:' + (28 + (d.n * 37) % 55) + '%"></i></span>'; }
        }
      }
      nd.style.transform = 'translateY(' + (g.P ? g.P[r] - box.scrollTop : (r - g.kTop) * LH) + 'px)';
    }
    if (g.P) wrapMeasure();
    if (RD) {
      seen.forEach(function (b) { var blk = blocks.get(b); if (blk) { blocks.delete(b); blocks.set(b, blk); } });   // keep what is on screen newest
      var pf = Math.ceil(g.vis);   // one screen either side, so a short scroll finds the rows loaded
      for (var q = Math.max(0, first - pf); q < first; q++) { var e1 = rowAt(q); if (e1.n) want.add(Math.floor((e1.n - 1) / BS)); }
      for (q = last + 1; q <= Math.min(R - 1, last + pf); q++) { var e2 = rowAt(q); if (e2.n) want.add(Math.floor((e2.n - 1) / BS)); }
      var miss = Array.from(want).filter(function (b) { return !blocks.has(b); });
      if (miss.length) schedFetch(new Set(miss));
    }
  }

  /* ---------- selection ---------- */
  var inSel = function (n) { return !!(sel && n >= sel.a && n <= sel.b); };
  function selHash() { return sel ? (sel.a === sel.b ? '#L' + sel.a : '#L' + sel.a + '-L' + sel.b) : ''; }
  var pageUrl = function () { return location.href.split('#')[0]; };
  function setHash() { try { history.replaceState(null, '', selHash() || location.pathname + location.search); } catch (e) { /* ignore */ } }
  function pickLine(n, shift) {
    if (shift && sel) { sel = { anchor: sel.anchor, a: Math.min(sel.anchor, n), b: Math.max(sel.anchor, n) }; } else sel = { anchor: n, a: n, b: n };
    curLine = n; RGEN++; paint(); setHash(); copyText(pageUrl() + selHash()); syncMenu(); updateStatus();
    toast('Link to ' + (sel.a === sel.b ? 'line ' + fmt(sel.a) : 'lines ' + fmt(sel.a) + '–' + fmt(sel.b)) + ' copied');
  }
  function selText() {
    if (!sel) return Promise.resolve('');
    var count = Math.min(sel.b - sel.a + 1, 2000);
    return ensureLines(sel.a, count).then(function (rows) { return rows.map(function (r) { return r[1]; }).join('\n'); });
  }
  function openLine(n) {
    var show = function (t) { $('lineTitle').textContent = 'Line ' + fmt(n) + ' · ' + fmt(t.length) + ' characters'; $('lineText').textContent = t; $('linebox').hidden = false; $('lineClose').focus(); };
    var d = lineData(n); if (d) show(d.t); else ensureLines(n, 1).then(function (r) { if (r[0]) show(r[0][1]); });
  }

  /* ---------- navigation ---------- */
  function revealRow(r, force) {
    var g = bgeom();
    if (!force && r >= g.kTop && r + 1 <= g.kTop + g.vis) return;
    vscroll(box, g, r - g.vis / 3);
  }
  function goLine(n) {
    n = Math.max(1, Math.min(N || 1, n)); curLine = n;
    if (PDF) { pdfSearch(n); } else if (ST.on) { strGo(n); } else if (box) { revealRow(rowOfLine(n)); RGEN++; paint(); }
    listReveal(n); listPaint(); updateCount(); afterScroll();
  }
  var neighborIdx = function (d) {
    if (!LIST.length) return -1;
    var i = d > 0 ? upper(LIST, curLine) : lower(LIST, curLine) - 1;
    return i >= 0 && i < LIST.length ? i : -1;
  };
  function step(d) {
    if (!LIST.length) return; var i = neighborIdx(d);
    if (i < 0) i = d > 0 ? 0 : LIST.length - 1;   // wrap around
    goLine(LIST[i]);
  }
  function updateCount() {
    var m = LIST.length; $('prev').disabled = $('next').disabled = !m;
    $('count').textContent = m ? (curLine ? fmt(upper(LIST, curLine)) : '–') + ' of ' + fmt(m) : '0 of 0';
    var shown = LIST.length, tot = ACT.length;
    $('hitHead').innerHTML = '<span>' + (qHits ? fmt(shown) + ' of ' + fmt(tot) : fmt(tot)) + ' matches</span><span>' + (RD ? 'whole file' : '') + '</span>';
    var fm = $('fMatches'); if (fm) fm.textContent = fmt(ACT.length) + ' matches';
  }

  /* ---------- right rail: flags near the reading position, whole-file hit ruler, position marker ---------- */
  var flagKey = '', peekEl = $('peek'), peekTok = 0;
  var hidePeek = function () { peekEl.hidden = true; peekTok++; };
  window.addEventListener('scroll', hidePeek, true);
  function heat(curFrac, win) {
    var W2 = 120, x = function (f) { return Math.round(4 + Math.min(1, Math.max(0, f)) * (W2 - 8)); };
    var svg = '<svg class="heat" viewBox="0 0 ' + W2 + ' 22" aria-hidden="true"><rect width="' + W2 + '" height="22" rx="5" fill="var(--surface-2)" stroke="var(--line)"/>';
    if (win) svg += '<rect x="' + x(win[0]) + '" y="2" width="' + Math.max(3, x(win[1]) - x(win[0])) + '" height="18" rx="3" fill="var(--accent)" opacity=".28"/>';
    return svg + '<rect x="' + (x(curFrac) - 2) + '" y="2" width="4" height="18" rx="2" fill="var(--accent)"/></svg>';
  }
  function peekCard(n, rows) {
    var rowsH = rows.map(function (r) {
      var line = r[1].slice(0, 400), hit = r[0] === n;
      return '<div class="ln ' + (hit ? 'hit' : 'ctx') + '"><i>' + fmt(r[0]) + '</i><span>' + (hit ? renderText(line, r[3], true) : esc(line)) + '</span></div>';
    }).join('');
    var m = maskOf(n) & onMask, chips = FILT.filter(function (f) { return m >> f.slot & 1; }).map(function (f) { return '<span class="chipf"><i style="background:var(--c' + (f.slot + 1) + ')"></i>' + esc(f.name) + '</span>'; }).join('');
    var vl = viewLines();
    return '<div class="peek-h"><b>line ' + fmt(n) + '</b><span>match ' + fmt(upper(LIST, n)) + ' of ' + fmt(LIST.length) + '</span></div><div class="code">' + rowsH + '</div>' +
      '<div class="peek-f">' + chips + '<span class="sp"></span>' + heat(n / N, [(vl[0] - 1) / N, vl[1] / N]) + '<small>' + Math.round(n / N * 100) + '% into the file. Click to jump</small></div>';
  }
  function showPeek(flag, n) {
    var tok = ++peekTok;
    ensureLines(Math.max(1, n - 2), Math.min(n, 3) + 2).then(function (rows) {
      if (tok !== peekTok) return;
      peekEl.innerHTML = peekCard(n, rows.filter(function (r) { return r[0] <= n + 2; })); peekEl.hidden = false;
      var r = flag.getBoundingClientRect(), pw = peekEl.offsetWidth, ph = peekEl.offsetHeight;
      var left = r.left - pw - 12; if (left < 8) left = Math.min(innerWidth - pw - 8, r.right + 12);
      peekEl.style.left = left + 'px'; peekEl.style.top = Math.max(8, Math.min(innerHeight - ph - 8, r.top + r.height / 2 - ph / 2)) + 'px';
    }).catch(function () { /* reader gone: no card */ });
  }
  function mkFlag(g, top, n, cur) {
    var slot = slotOf(n), f = document.createElement('button');
    f.type = 'button'; f.tabIndex = cur ? 0 : -1; f.className = 'f s' + (slot % 2 + 1) + (cur ? ' cur' : '');
    f.style.top = top + 'px'; f.style.background = 'var(--c' + (slot + 1) + ')'; f.setAttribute('aria-label', 'line ' + fmt(n));
    f.addEventListener('click', function () { goLine(n); });
    var tm = 0;
    f.addEventListener('mouseenter', function () { tm = setTimeout(function () { showPeek(f, n); }, 120); });
    f.addEventListener('mouseleave', function () { clearTimeout(tm); hidePeek(); });
    f.addEventListener('focus', function () { showPeek(f, n); }); f.addEventListener('blur', hidePeek);
    f.addEventListener('keydown', function (e) {
      if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
      e.preventDefault(); var sib = e.key === 'ArrowDown' ? f.nextElementSibling : f.previousElementSibling;
      while (sib && !sib.classList.contains('f')) sib = e.key === 'ArrowDown' ? sib.nextElementSibling : sib.previousElementSibling;
      if (sib) { f.tabIndex = -1; sib.tabIndex = 0; sib.focus(); }
    });
    g.appendChild(f); return f;
  }
  function paintFlags() {
    var g = $('flagsDoc'); if (!box && !ST.on) return;
    if (ST.on) {   // the flags of the whole file (when there are few enough), and where the window is
      var ch2 = g.clientHeight || 400, key2 = ['S', LV, curLine, ch2].join(':');
      if (key2 !== flagKey) {
        flagKey = key2; g.innerHTML = '<div class="view"></div>';
        if (LIST.length <= 3000) for (var q = 0; q < LIST.length; q++) mkFlag(g, (LIST[q] - 0.5) / N * ch2, LIST[q], LIST[q] === curLine);
        var f1 = g.querySelector('.f'); if (f1) f1.tabIndex = 0;
      }
      var v2 = g.firstChild, vl = viewLines(); if (v2) { v2.style.display = ''; v2.style.top = ((vl[0] - 1) / N * ch2) + 'px'; v2.style.height = Math.max(6, (vl[1] - vl[0] + 1) / N * ch2) + 'px'; }
      return;
    }
    var R = rowCount(), vg = bgeom(), ch = g.clientHeight || vg.vh;
    // few hits: the rail is the whole file. Many hits: it is the window around the reading position, so the flags stay apart.
    var whole = LIST.length <= 3000 || R <= vg.vis * 12, w0 = 0, w1 = R;
    if (!whole) { w0 = Math.max(0, Math.floor(vg.kTop - 4 * vg.vis)); w1 = Math.min(R, Math.ceil(vg.kTop + 5 * vg.vis)); }
    var key = [MO.on, LV, curLine, ch, whole ? 'w' : Math.floor(vg.kTop / Math.max(1, vg.vis))].join(':');
    var had = g.contains(document.activeElement);
    if (key !== flagKey) {
      flagKey = key; g.innerHTML = '<div class="view"></div>';
      if (!MO.on) {
        var a = lower(LIST, w0 + 1), b = upper(LIST, w1);
        for (var i = a; i < b && i < a + 3500; i++) mkFlag(g, (LIST[i] - 1 - w0 + 0.5) / (w1 - w0) * ch, LIST[i], LIST[i] === curLine);
        if (!g.querySelector('.f[tabindex="0"]')) { var f0 = g.querySelector('.f'); if (f0) f0.tabIndex = 0; }
        if (had) { var c = g.querySelector('.f.cur'); if (c) c.focus({ preventScroll: true }); }
      }
    }
    var v = g.firstChild; if (!v) return;
    if (MO.on || R <= vg.vis) { v.style.display = 'none'; return; }
    v.style.display = ''; v.style.top = ((vg.kTop - w0) / (w1 - w0) * ch) + 'px'; v.style.height = Math.max(6, vg.vis / (w1 - w0) * ch) + 'px';
  }
  function setupCanvas(cv) { var w = cv.clientWidth || 18, h = cv.clientHeight || 400, r = window.devicePixelRatio || 1; cv.width = w * r; cv.height = h * r; var g = cv.getContext('2d'); g.scale(r, r); return { g: g, w: w, h: h }; }
  function drawRuler() {
    var cv = $('rcv'); if (!cv || !N) return;
    var c = setupCanvas(cv), g = c.g, nb = c.h, cs = getComputedStyle(root), cnt = [], s;
    for (s = 0; s < NF; s++) cnt.push(new Uint32Array(nb));
    LIST.forEach(function (n) { var m = maskOf(n) & onMask, b = Math.min(nb - 1, Math.floor((n - 1) / N * nb)); for (var k = 0; k < NF; k++) if (m >> k & 1) cnt[k][b]++; });
    var mx = 1; cnt.forEach(function (a) { for (var y = 0; y < nb; y++) if (a[y] > mx) mx = a[y]; });
    for (var lane = 0; lane < 2; lane++) {
      for (var y2 = 0; y2 < nb; y2++) {
        var best = -1, bv = 0;
        for (s = lane; s < NF; s += 2) if (cnt[s][y2] > bv) { bv = cnt[s][y2]; best = s; }
        if (best < 0) continue;
        g.fillStyle = cs.getPropertyValue('--c' + (best + 1)); g.globalAlpha = 0.25 + 0.7 * Math.min(1, bv / mx);
        g.fillRect(lane ? c.w / 2 + 1 : 1, y2, c.w / 2 - 2, 1);
      }
    }
  }
  function drawMark() {
    if (!box && !ST.on) return;
    var vl = viewLines(), w = $('rwin'), mk = $('rmk');
    w.style.top = ((vl[0] - 1) / N * 100) + '%'; w.style.height = ((vl[1] - vl[0] + 1) / N * 100) + '%';
    mk.style.top = ((curLine - 1) / N * 100) + '%'; mk.hidden = !curLine;
    if ($('histoWrap').hidden) return;
    var hw = $('hwin'), hc = $('hcur');
    hw.style.left = ((vl[0] - 1) / N * 100) + '%'; hw.style.width = ((vl[1] - vl[0] + 1) / N * 100) + '%';
    hc.style.left = ((curLine - 1) / N * 100) + '%'; hc.hidden = !curLine;
  }
  /* the same hit density, as stacked bars across the left panel; click jumps */
  function drawHisto() {
    var cv = $('hcv'); if (!cv || $('histoWrap').hidden || !N || !cv.clientWidth) return;
    var c = setupCanvas(cv), g = c.g, cs = getComputedStyle(root), nb = Math.max(12, Math.floor(c.w / 3)), cnt = [], i, k;
    for (k = 0; k < NF; k++) cnt.push(new Uint32Array(nb));
    LIST.forEach(function (n) { var b = Math.min(nb - 1, Math.floor((n - 1) / N * nb)), m = maskOf(n) & onMask; for (var s2 = 0; s2 < NF; s2++) if (m >> s2 & 1) cnt[s2][b]++; });
    var mx = 1; for (i = 0; i < nb; i++) { var t = 0; for (k = 0; k < NF; k++) t += cnt[k][i]; if (t > mx) mx = t; }
    var bw = c.w / nb; g.globalAlpha = 0.9;
    for (i = 0; i < nb; i++) {
      var y = c.h;
      for (k = 0; k < NF; k++) { var hh = cnt[k][i] / mx * (c.h - 4); if (!hh) continue; g.fillStyle = cs.getPropertyValue('--c' + (k + 1)); g.fillRect(i * bw + 0.5, y - hh, Math.max(1, bw - 1), hh); y -= hh; }
    }
  }
  function tipAt(e, a, b) {
    var hits = upper(LIST, b) - lower(LIST, a), t = $('tip');
    t.innerHTML = '<b>≈ line ' + fmt(Math.round((a + b) / 2)) + '</b> · ' + fmt(hits) + ' ' + (hits === 1 ? 'hit' : 'hits') + '<small>click to jump to the next hit here</small>';
    t.hidden = false; var w = t.offsetWidth; t.style.left = Math.max(8, Math.min(innerWidth - w - 8, e.clientX - w - 14)) + 'px'; t.style.top = Math.max(8, e.clientY - 20) + 'px';
  }
  /* the ruler (vertical) and the histogram (horizontal) both map a position to a line */
  function overviewEvents(el, horiz) {
    var frac = function (e) { var r = el.getBoundingClientRect(); return Math.max(0, Math.min(1, horiz ? (e.clientX - r.left) / r.width : (e.clientY - r.top) / r.height)); };
    var half = function () { return 2 / Math.max(1, horiz ? el.clientWidth : el.clientHeight); };
    el.addEventListener('mousemove', function (e) { var f = frac(e); tipAt(e, Math.max(1, Math.floor((f - half()) * N)), Math.min(N, Math.ceil((f + half()) * N))); });
    el.addEventListener('mouseleave', function () { $('tip').hidden = true; });
    el.addEventListener('click', function (e) {
      var from = Math.max(1, Math.floor((frac(e) - half()) * N)), i = lower(LIST, from), n = LIST[i] || LIST[LIST.length - 1];
      if (n) goLine(n); else goLine(Math.max(1, Math.round(frac(e) * N)));
    });
  }
  var rulerEvents = function () { overviewEvents($('ruler'), false); overviewEvents($('histo'), true); };
  function afterScroll() { paintFlags(); drawMark(); updateStatus(); }
  function redrawRail() { flagKey = ''; drawRuler(); drawHisto(); paintFlags(); drawMark(); }

  /* ---------- match list (virtual, one row per hit) ---------- */
  var hitList = $('hitList'), lnodes = new Map();
  /* the text around the first (switched on) match of a line, the match in bold */
  function snipHtml(t, spans, before) {
    var sps = spans || [], p = sps.find(function (x) { return on[x[2]]; }) || sps[0];
    if (!p) return ws(t.slice(0, 110));
    var a = Math.max(0, p[0] - (before === undefined ? 28 : before)), b = Math.min(t.length, p[1] + 110);
    return (a && before !== 0 ? '…' : '') + ws(t.slice(a, p[0])) + '<b>' + ws(t.slice(p[0], p[1])) + '</b>' + ws(t.slice(p[1], b)) + (b < t.length ? '…' : '');
  }
  function snipFor(n) {
    var s = snipCache.get(n); if (s !== undefined) return s;
    var d = lineData(n); if (!d) return undefined;
    s = snipHtml(d.t, d.sp);
    snipCache.set(n, s); if (snipCache.size > 2000) snipCache.delete(snipCache.keys().next().value);
    return s;
  }
  function wantSnips(first, last) {
    var pf = Math.ceil(hitList.clientHeight / RH) * 2, miss = [];
    for (var k = Math.max(0, first - pf); k <= Math.min(LIST.length - 1, last + pf); k++) { var n = LIST[k]; if (snipFor(n) === undefined) miss.push(n); }
    if (!miss.length || EMB) return;
    clearTimeout(snipT);
    snipT = setTimeout(function () {
      if (!readerUp) return;
      var batch = miss.filter(function (n) { return !snipWant.has(n); }).slice(0, 400); batch.forEach(function (n) { snipWant.add(n); });
      if (!batch.length) return;
      rget('/lines?l=' + batch.join(',')).then(function (d) { batch.forEach(function (n) { snipWant.delete(n); }); storeRows(d.l); d.l.forEach(function (r) { snipFor(r[0]); }); lnodes.forEach(function (nd) { nd._n = -1; }); listPaint(); })
        .catch(function () { batch.forEach(function (n) { snipWant.delete(n); }); readerFailed(); });
    }, 40);
  }
  function fillRow(nd, n) {
    var s = snipFor(n);
    nd.innerHTML = '<i class="d" style="background:' + hitColor(n) + '"></i><span class="no">' + fmt(n) + '</span><span class="snip">' + (s !== undefined ? s : '<span class="sk"></span>') + '</span>';
  }
  function listPaint() {
    var L = hitList;
    if (G.mode !== 'all') return;
    if (!LIST.length) {
      lnodes.clear();
      if (RD && !HITS.length && FILT.length && !SC.done) {
        L.innerHTML = SC.on ? '<div class="none">Scanning the file for the filters…</div>' : '<div class="gempty"><span>The search found no hit lines in this file, or did not list them. Scan it with the same filters to find them here.</span><button class="btn pri" type="button" id="scanGo">Scan the file for these filters</button></div>';
        return;
      }
      L.innerHTML = '<div class="none">' + (qHits ? 'No hit matches that.' : (HITS.length ? 'Every filter is off. Click a filter above to turn it on.' : 'No match lines to list.')) + '</div>'; return;
    }
    var spc = L.firstChild;
    if (!spc || !spc.classList || !spc.classList.contains('vsp')) { L.innerHTML = '<div class="vsp"><div class="vrows"></div></div>'; spc = L.firstChild; lnodes.clear(); }
    var vrw = spc.firstChild, total = LIST.length, g = vlayer(spc, vrw, L, total, RH);
    var first = Math.max(0, Math.floor(g.kTop) - 2), last = Math.min(total - 1, Math.ceil(g.kTop + g.vis) + 2);
    wantSnips(first, last);
    lnodes.forEach(function (nd, k) { if (k < first || k > last) { nd.remove(); lnodes.delete(k); } });
    for (var k = first; k <= last; k++) {
      var nd = lnodes.get(k), n = LIST[k];
      if (!nd) { nd = document.createElement('div'); nd.className = 'mrow v'; nd.setAttribute('role', 'option'); vrw.appendChild(nd); lnodes.set(k, nd); nd._n = -1; }
      var ready = snipCache.has(n);
      if (nd._n !== n || nd._r !== ready) { nd._n = n; nd._r = ready; nd.setAttribute('data-n', n); nd.id = 'opt-' + n; fillRow(nd, n); }
      nd.style.transform = 'translateY(' + ((k - g.kTop) * RH) + 'px)';
      var c = n === curLine; nd.classList.toggle('cur', c); nd.setAttribute('aria-selected', c ? 'true' : 'false'); if (c) L.setAttribute('aria-activedescendant', nd.id);
    }
  }
  function listReveal(n) {
    if (G.mode !== 'all') return;
    var k = lower(LIST, n); if (LIST[k] !== n) return;
    var g = vgeom(hitList, LIST.length, RH); if (k >= g.kTop && k + 1 <= g.kTop + g.vis) return;
    vscroll(hitList, g, k - g.vis / 3);
  }
  hitList.addEventListener('scroll', function () { listPaint(); });
  hitList.addEventListener('click', function (e) {
    var b = e.target.closest('.mrow'); if (b) { goLine(+b.getAttribute('data-n')); return; }
    var g = e.target.closest('.grow');
    if (g) { if (g.hasAttribute('data-t')) { G.tpl = +g.getAttribute('data-t'); setListMode('all'); } else goLine(+g.getAttribute('data-from')); return; }
    if (e.target.closest('#scanGo')) startScan();
  });
  hitList.addEventListener('keydown', function (e) {
    if (e.key === 'ArrowDown') { e.preventDefault(); step(1); } else if (e.key === 'ArrowUp') { e.preventDefault(); step(-1); }
    else if (e.key === 'Home') { e.preventDefault(); if (LIST.length) goLine(LIST[0]); } else if (e.key === 'End') { e.preventDefault(); if (LIST.length) goLine(LIST[LIST.length - 1]); }
  });

  /* The list filter: the typed text against the line number and the line text. Embedded pages test the lines directly; the reader scans the file. */
  var qTok = 0, qTimer = 0;
  function refreshList() {
    var t = $('q').value.trim().toLowerCase(), tok = ++qTok;
    var finish = function (arr) {
      var L = arr;
      if (G.tpl >= 0) L = (arr || ACT).filter(function (n) { return G.of.get(n) === G.tpl; });
      qHits = L; LIST = L || ACT; LV++; lnodes.clear(); hitList.innerHTML = ''; hitList.scrollTop = 0; $('q').classList.remove('busy');
      if (MO.on) { moBuild(); if (curLine) revealRow(moRowOfLine(curLine), true); }
      if (box || ST.on) { RGEN++; paint(); }
      if (curLine) listReveal(curLine);
      pillPaint(); listPaint(); updateCount(); redrawRail(); updateStatus(); groupsRefresh();
    };
    if (!t) { finish(null); return; }
    if (EMB) { finish(ACT.filter(function (n) { return String(n).indexOf(t) >= 0 || EMB.lines[n - 1].toLowerCase().indexOf(t) >= 0; })); return; }
    $('q').classList.add('busy');
    var set = new Set();
    scan({ q: t, cs: false, ww: false, rx: false }, function () { return tok === qTok; }, function (lines, frac, done) {
      lines.forEach(function (l) { set.add(l); });
      if (done) finish(ACT.filter(function (n) { return String(n).indexOf(t) >= 0 || set.has(n); }));
    });
  }
  $('q').addEventListener('input', function () { clearTimeout(qTimer); qTimer = setTimeout(refreshList, 150); });

  /* ---------- filter switches ---------- */
  function chipsPaint() {
    $('chips').innerHTML = FILT.map(function (f, k) {
      return '<button type="button" class="chipf" data-k="' + k + '" aria-pressed="' + on[f.slot] + '" title="' + (on[f.slot] ? 'Click to turn this filter off. Alt-click for this filter only.' : 'Click to turn this filter on') + '"><i style="background:var(--c' + (f.slot + 1) + ')"></i>' + esc(f.name) + ' ' + fmt(f.count) + '</button>';
    }).join('');
    var all = FILT.every(function (f) { return on[f.slot]; }), none = FILT.every(function (f) { return !on[f.slot]; });
    $('chipctl').innerHTML = FILT.length > 1 ? '<button type="button" class="lnk" data-act="all"' + (all ? ' disabled' : '') + '>All</button><button type="button" class="lnk" data-act="none"' + (none ? ' disabled' : '') + '>None</button>' : '';
  }
  function setFilters(next, focusK) {
    if (next.every(function (v, i) { return v === on[i]; })) return;
    for (var i = 0; i < NF; i++) on[i] = next[i];
    save(); hidePeek(); snipCache.clear(); recomputeOn(); rebuildAct();
    if (curLine && !isHit(curLine) && ACT.length && !MO.on) { /* stay where the reader is */ }
    G.ready = false; if (G.tpl >= 0 || G.mode === 'msg') buildTpl(refreshList); else refreshList();
    chipsPaint();
    if (focusK !== undefined) { var again = $('chips').querySelector('[data-k="' + focusK + '"]'); if (again) again.focus(); }
  }
  function toggleFilter(k, solo) {
    var f = FILT[k]; if (!f) return; var a = on.slice();
    if (solo) { var only = on[f.slot] && FILT.every(function (g) { return g.slot === f.slot || !on[g.slot]; }); FILT.forEach(function (g) { a[g.slot] = only ? true : g.slot === f.slot; }); } else a[f.slot] = !a[f.slot];
    setFilters(a, k);
  }
  $('chips').addEventListener('click', function (e) { var b = e.target.closest('.chipf'); if (b) toggleFilter(+b.getAttribute('data-k'), e.altKey || e.shiftKey); });
  $('chipctl').addEventListener('click', function (e) {
    var b = e.target.closest('.lnk'); if (!b || b.disabled) return; var a = on.slice(), v = b.getAttribute('data-act') === 'all';
    FILT.forEach(function (f) { a[f.slot] = v; }); setFilters(a);
  });

  /* ---------- matches only ---------- */
  function setMO(v) {
    var at = box ? anchorLine() : 1;
    pref.mo = v; save(); syncTools(); if (!box) return;
    MO.on = v; if (v) moBuild(); else MO.gen++;
    flagKey = ''; RGEN++; paint(); revealRow(rowOfLine(at), true); paint(); afterScroll();
  }

  /* ---------- find in the whole file ---------- */
  var F = { open: false, q: '', cs: false, ww: false, rx: false, res: [], i: -1, tok: 0, scan: 1, done: true, capped: false, pending: 0, bad: false, err: '', timer: 0, timer2: 0 };
  function makeRe(p) {
    if (!p.q) return null;
    var src = p.rx ? p.q : p.q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); if (p.ww) src = '\\b(?:' + src + ')\\b';
    try { return new RegExp(src, p.cs ? 'g' : 'gi'); } catch (e) { return 'bad'; }
  }
  /* Scan every line for a pattern; sink(lines, fraction, done, error, capped). The page stays responsive: 12 ms slices, or one reader request at a time. */
  function scan(p, alive, sink) {
    if (EMB) {
      var re = makeRe(p); if (re === 'bad') { sink([], 1, true, 'Invalid pattern'); return; }
      var one = new RegExp(re.source, p.cs ? '' : 'i'), n = 1, found = [];
      (function step() {
        if (!alive()) return;
        var t0 = performance.now(), got = [];
        while (n <= N && (n & 255 || performance.now() - t0 < 12)) { if (one.test(EMB.lines[n - 1])) { got.push(n); if (found.length + got.length >= 200000) { n = N + 1; break; } } n++; }
        found = found.concat(got);
        var done = n > N; sink(got, Math.min(1, n / N), done, null, found.length >= 200000);
        if (!done) setTimeout(step, 0);
      })();
      return;
    }
    var from = 1, total = 0;
    (function next() {
      if (!alive() || !from) return;
      rget('/find?q=' + encodeURIComponent(p.q) + '&cs=' + (p.cs ? 1 : 0) + '&ww=' + (p.ww ? 1 : 0) + '&rx=' + (p.rx ? 1 : 0) + '&from=' + from + '&max=5000').then(function (r) {
        if (!alive()) return;
        if (r.err) { sink([], 1, true, 'Invalid pattern'); return; }
        total += r.m.length; var capped = total >= 200000; from = capped ? 0 : r.next;
        sink(r.m, from ? Math.min(1, from / N) : 1, !from, null, capped);
        if (from) next();
      }).catch(function () { readerFailed(); });
    })();
  }
  function findCount() {
    var el = $('findCount');
    if (!F.q) { el.textContent = ''; return; }
    if (F.bad) { el.textContent = F.err || 'Invalid pattern'; return; }
    var n = F.res.length;
    el.textContent = (n ? (F.i >= 0 ? F.i + 1 : 0) : 0) + ' of ' + fmt(n) + (F.capped ? '+' : '') + (F.done ? '' : ' · scanning ' + Math.round(F.scan * 100) + '%');
  }
  function runFind() {
    var tok = ++F.tok; F.res = []; F.i = -1; F.done = true; F.capped = false; F.pending = 0; F.bad = false; F.err = ''; fre = null;
    $('findbox').classList.remove('bad');
    if (F.q) {
      var m = makeRe(F); fre = m && m !== 'bad' ? m : null;
      F.done = false; F.scan = 0;
      scan(F, function () { return tok === F.tok; }, function (lines, frac, done, err, capped) {
        for (var i = 0; i < lines.length; i++) F.res.push(lines[i]);
        F.scan = frac; if (done) F.done = true; if (capped) F.capped = true;
        if (err) { F.bad = true; F.err = err; F.done = true; fre = null; $('findbox').classList.add('bad'); RGEN++; paint(); }
        var front = Math.floor(frac * N);
        if (F.pending) { var pd = F.pending; if (F.done || (pd > 0 ? F.res.length : front >= (curLine || 0))) { F.pending = 0; findNext(pd); } }
        findCount();
      });
    }
    RGEN++; paint(); findCount();
  }
  function findNext(d) {
    if (!F.q || F.bad) return;
    var R = F.res, at = curLine || 0, i;
    if (!R.length) { if (!F.done) F.pending = d; return; }
    var front = Math.floor(F.scan * N);
    if (d < 0 && !F.done && front < at) { F.pending = d; return; }
    if (d > 0) { i = upper(R, at); if (i >= R.length) { if (!F.done) { F.pending = d; return; } i = 0; } }
    else { i = lower(R, at) - 1; if (i < 0) { if (!F.done) { F.pending = d; return; } i = R.length - 1; } }
    F.i = i; goLine(R[i]); findCount();
  }
  function findAgain() { runFind(); findNext(1); }
  function openFind() {
    if (PDF) return;
    F.open = true; $('findbar').hidden = false; $('findBtn').setAttribute('aria-pressed', 'true');
    $('findHint').textContent = (RD ? 'Searches the whole file through the local reader.' : 'Searches every line of this page.') + ' The browser’s own Ctrl+F only sees the rows drawn right now.';
    $('findbox').focus(); $('findbox').select(); if (F.q) runFind();
  }
  function closeFind() {
    F.open = false; $('findbar').hidden = true; $('findBtn').setAttribute('aria-pressed', 'false');
    F.tok++; F.res = []; fre = null; RGEN++; paint();
  }
  $('findbox').addEventListener('input', function () { clearTimeout(F.timer2); F.timer2 = setTimeout(function () { F.q = $('findbox').value; findAgain(); }, 120); });
  $('findbox').addEventListener('keydown', function (e) { if (e.key === 'Enter') { e.preventDefault(); findNext(e.shiftKey ? -1 : 1); } else if (e.key === 'Escape') { e.preventDefault(); closeFind(); } });
  [['fCase', 'cs'], ['fWord', 'ww'], ['fRe', 'rx']].forEach(function (p) { $(p[0]).addEventListener('click', function () { F[p[1]] = !F[p[1]]; this.setAttribute('aria-pressed', String(F[p[1]])); findAgain(); }); });
  $('fPrev').addEventListener('click', function () { findNext(-1); });
  $('fNext').addEventListener('click', function () { findNext(1); });
  $('fClose').addEventListener('click', closeFind);
  $('findBtn').addEventListener('click', function () { if (F.open) closeFind(); else openFind(); });

  /* ---------- go to line ---------- */
  function gotoLine(v) { if (!v) return; if (v > N) { toast('This file has ' + fmt(N) + ' lines'); return; } goLine(v); }
  var openGoto = function () { if (PDF) return; $('gotoWrap').hidden = false; $('gotoIn').focus(); $('gotoIn').select(); };
  var closeGoto = function () { $('gotoWrap').hidden = true; };
  $('gotoBtn').addEventListener('click', function () { if ($('gotoWrap').hidden) openGoto(); else closeGoto(); });
  $('gotoIn').addEventListener('keydown', function (e) { if (e.key === 'Enter') { gotoLine(parseInt(this.value.replace(/[^\d]/g, ''), 10)); closeGoto(); } else if (e.key === 'Escape') closeGoto(); });

  /* ---------- text pane events ---------- */
  function onBoxClick(e) {
    var more = e.target.closest('.more'); if (more) { openLine(+more.getAttribute('data-n')); return; }
    var fold = e.target.closest('.fold');
    if (fold) { var j = +fold.getAttribute('data-j'), g = moGap(j); MO.ex[j] = e.shiftKey ? Math.ceil(g / 2) : (MO.ex[j] || 0) + EXSTEP; moPrefix(j); MO.gen++; paint(); afterScroll(); return; }
    var no = e.target.closest('.ln .no'); if (no && !no.closest('.skel')) pickLine(+no.closest('.ln').getAttribute('data-n'), e.shiftKey);
  }

  /* ---------- toolbar, rail splitter, menu ---------- */
  function syncTools() {
    var text = !PDF && N > 0, vb = $('vbody');
    $('moBtn').disabled = !text || ST.on; $('moBtn').setAttribute('aria-pressed', String(!!pref.mo && text && !ST.on));
    $('moBtn').title = ST.on ? 'Matches only works in the text view' : text ? 'Show only the lines that match, with ' + CTX + ' lines of context. The gaps fold away.  (M)' : 'Matches only needs a text view';
    var wb = $('wrapBtn'); wb.disabled = !canWrap(); wb.setAttribute('aria-pressed', String(wrapOn()));
    wb.title = ST.on ? 'The structured view wraps by itself' : RD ? 'Windowed files keep one line per row; long lines are cut at 10,000 characters' : canWrap() ? 'Wrap long lines  ( W )' : 'Wrap applies to text and code';
    $('viewSeg').hidden = !(SD && N > 0);
    Array.prototype.forEach.call($('viewSeg').querySelectorAll('button'), function (b) { b.setAttribute('aria-pressed', String((b.getAttribute('data-v') === 'struct') === ST.on)); });
    var lists = HITS.length > 0 && (!!RD || N >= 2000); $('listSeg').hidden = !lists; $('histoWrap').hidden = !lists;
    if (lists) drawHisto();
    $('fsDown').disabled = $('fsUp').disabled = !text; $('findBtn').disabled = $('gotoBtn').disabled = !text;
    $('railBtn').setAttribute('aria-pressed', String(!pref.collapsed)); vb.classList.toggle('collapsed', !!pref.collapsed);
    $('app').style.setProperty('--fs', FS[pref.fs][0] + 'px'); $('app').style.setProperty('--lh', FS[pref.fs][1] + 'px');
  }
  function setFs(d) {
    var f = Math.max(0, Math.min(FS.length - 1, pref.fs + d)); if (f === pref.fs || PDF) return;
    var at = anchorLine(); pref.fs = f; save(); LH = FS[f][1]; syncTools();
    if (box) { measureChar(); if (wrapOn()) wrapReset(); setWidth(); RGEN++; revealRow(rowOfLine(at), true); paint(); redrawRail(); }
    else if (ST.on) { redrawRail(); afterScroll(); }
  }
  function toggleRail() { pref.collapsed = !pref.collapsed; save(); syncTools(); setTimeout(function () { wrapResize(); listPaint(); paint(); redrawRail(); }, 0); }
  $('railBtn').addEventListener('click', toggleRail);
  $('moBtn').addEventListener('click', function () { setMO(!pref.mo); });
  $('wrapBtn').addEventListener('click', function () { setWrap(!WR.on); });
  $('viewSeg').addEventListener('click', function (e) { var b = e.target.closest('button'); if (b && SD) setStruct(b.getAttribute('data-v') === 'struct'); });
  $('fsDown').addEventListener('click', function () { setFs(-1); });
  $('fsUp').addEventListener('click', function () { setFs(1); });
  (function () {
    var spl = $('split'), vb = $('vbody'), drag = false;
    var apply = function (w) { pref.railw = Math.max(200, Math.min(520, Math.round(w))); vb.style.setProperty('--railw', pref.railw + 'px'); };
    apply(pref.railw);
    spl.addEventListener('pointerdown', function (e) { if (pref.collapsed) return; drag = true; spl.setPointerCapture(e.pointerId); spl.classList.add('drag'); });
    spl.addEventListener('pointermove', function (e) { if (drag) apply(e.clientX - vb.getBoundingClientRect().left); });
    var end = function () { if (!drag) return; drag = false; spl.classList.remove('drag'); save(); wrapResize(); listPaint(); paint(); redrawRail(); };
    spl.addEventListener('pointerup', end); spl.addEventListener('pointercancel', end);
    spl.addEventListener('dblclick', toggleRail);
    spl.addEventListener('keydown', function (e) { if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') { e.preventDefault(); apply(pref.railw + (e.key === 'ArrowRight' ? 16 : -16)); save(); } });
  })();

  var menu = $('menu'), menuBtn = $('menuBtn');
  function syncMenu() {
    $('mCopySel').disabled = !sel; $('mExport').disabled = !HITS.length;
    $('mScan').hidden = !RD; $('mScan').disabled = SC.on || SC.done || (HITS.length > 0 && SC.from <= 1);
  }
  var closeMenu = function () { if (!menu.hidden) { menu.hidden = true; menuBtn.setAttribute('aria-expanded', 'false'); } };
  menuBtn.addEventListener('click', function (e) { e.stopPropagation(); syncMenu(); menu.hidden = !menu.hidden; menuBtn.setAttribute('aria-expanded', String(!menu.hidden)); if (!menu.hidden) menu.querySelector('button:not(:disabled)').focus(); });
  document.addEventListener('click', function (e) { if (!menu.contains(e.target) && e.target !== menuBtn) closeMenu(); });
  menu.addEventListener('keydown', function (e) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return; e.preventDefault();
    var items = Array.prototype.slice.call(menu.querySelectorAll('button:not(:disabled)')), i = items.indexOf(document.activeElement);
    items[(i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length].focus();
  });
  menu.addEventListener('click', function (e) { var b = e.target.closest('button'); if (b && !b.disabled) closeMenu(); });
  $('mCopyPath').addEventListener('click', function () { copyText(CFG.path); toast('Path copied'); });
  $('mCopyLink').addEventListener('click', function () { copyText(pageUrl() + (sel ? selHash() : curLine ? '#L' + curLine : '')); toast('Link copied'); });
  $('mCopySel').addEventListener('click', function () { selText().then(function (t) { copyText(t); toast('Selected lines copied'); }); });
  $('mTheme').addEventListener('click', function () {
    var dark = root.dataset.theme ? root.dataset.theme === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
    root.dataset.theme = dark ? 'light' : 'dark'; pref.theme = root.dataset.theme; save(); redrawRail();
  });
  $('mKeys').addEventListener('click', function () { $('kbd').hidden = false; $('kbdClose').focus(); });
  $('kbdClose').addEventListener('click', function () { $('kbd').hidden = true; });
  $('native').addEventListener('click', function () {
    if (RD) { rget('/native').then(function () { toast('Opened in the default app'); }).catch(function () { toast('The local reader is not running. Open the file from the workbench.'); }); }
    else { window.open(CFG.uri); }
  });
  ['kbd', 'linebox'].forEach(function (id) { $(id).addEventListener('click', function (e) { if (e.target === this) this.hidden = true; }); });
  $('lineClose').addEventListener('click', function () { $('linebox').hidden = true; });
  $('lineCopy').addEventListener('click', function () { copyText($('lineText').textContent); toast('Line copied'); });
  $('staleGo').addEventListener('click', function () { toast('Run the search again in the workbench to refresh this view'); });
  $('staleX').addEventListener('click', function () { $('stale').hidden = true; });
  $('readerRetry').addEventListener('click', function () { readerUp = true; inflight.clear(); paintReader(); paint(); listPaint(); });

  function exportHits() {
    var N2 = LIST.length, cap = RD ? 50000 : N2, take = Math.min(N2, cap), parts = [], i = 0;
    var finish = function () {
      var a = document.createElement('a'), url = URL.createObjectURL(new Blob([parts.join('\n') + '\n'], { type: 'text/plain' }));
      a.href = url; a.download = CFG.name.replace(/[^\w.-]+/g, '-') + '-hits.txt'; document.body.appendChild(a); a.click(); a.remove(); setTimeout(function () { URL.revokeObjectURL(url); }, 1000);
      toast('Exported ' + fmt(parts.length) + ' hits' + (take < N2 ? ' (the first ' + fmt(take) + ')' : ''));
    };
    if (EMB) { for (; i < take; i++) parts.push(LIST[i] + ':' + EMB.lines[LIST[i] - 1]); finish(); return; }
    toast('Collecting hits…');
    (function batch() {
      if (i >= take) { finish(); return; }
      var ids = LIST.slice(i, i + 400); i += 400;
      rget('/lines?l=' + ids.join(',')).then(function (d) { d.l.forEach(function (r) { parts.push(r[0] + ':' + r[1]); }); batch(); }).catch(function () { readerFailed(); toast('The local reader stopped; export cancelled'); });
    })();
  }
  $('mExport').addEventListener('click', exportHits);
  $('mScan').addEventListener('click', function () { startScan(); });

  /* ---------- status bar ---------- */
  function updateStatus() {
    var pos = '';
    if ((box || ST.on) && N) { var a = topLine(); pos = 'Ln ' + fmt(a) + ' of ' + fmt(N) + ' · ' + Math.round(a / N * 100) + '%'; }
    if (sel) pos += (pos ? ' · ' : '') + fmt(sel.b - sel.a + 1) + ' selected';
    $('stPos').textContent = pos; $('stEnc').textContent = CFG.enc || '';
  }

  /* ---------- PDF: the browser's own viewer beside the match list ---------- */
  var pdfFrame = null;
  function pdfSearch(n) {
    if (!pdfFrame) return;
    var d = lineData(n), sps = (d && d.sp) || [], p = sps.find(function (x) { return on[x[2]]; }) || sps[0];
    if (p) pdfFrame.src = CFG.pdf + '#search=' + encodeURIComponent(d.t.slice(p[0], p[1]));
  }

  /* ---------- keys ---------- */
  document.addEventListener('keydown', function (e) {
    var mod = e.ctrlKey || e.metaKey, k = e.key;
    if (mod && (k === 'f' || k === 'F') && !PDF) { e.preventDefault(); openFind(); return; }
    if (mod && (k === 'g' || k === 'G') && !PDF) { e.preventDefault(); openGoto(); return; }
    if (k === 'Escape') {
      if (!$('kbd').hidden) $('kbd').hidden = true; else if (!$('linebox').hidden) $('linebox').hidden = true; else if (!menu.hidden) { closeMenu(); menuBtn.focus(); }
      else if (!$('gotoWrap').hidden) closeGoto(); else if (F.open) closeFind(); else if ($('q').value) { $('q').value = ''; refreshList(); }
      return;
    }
    if (e.target.closest && e.target.closest('input,textarea')) return;
    if (mod || e.altKey) return;
    if (k === '/') { e.preventDefault(); $('q').focus(); }
    else if (k === 'n' || k === 'F3') { e.preventDefault(); step(1); }
    else if (k === 'p') { e.preventDefault(); step(-1); }
    else if (k >= '1' && k <= '6') toggleFilter(+k - 1, false);
    else if (k === 'm' || k === 'M') { if (!$('moBtn').disabled) setMO(!pref.mo); }
    else if (k === 'w' || k === 'W') { if (!$('wrapBtn').disabled) setWrap(!WR.on); }
    else if (k === '+' || k === '=') setFs(1);
    else if (k === '-') setFs(-1);
    else if (k === '[') toggleRail();
    else if (k === '?') { $('kbd').hidden = false; $('kbdClose').focus(); }
  });
  $('prev').addEventListener('click', function () { step(-1); });
  $('next').addEventListener('click', function () { step(1); });
  window.addEventListener('resize', function () { if (box) { wrapResize(); paint(); redrawRail(); } else if (ST.on) { redrawRail(); afterScroll(); } listPaint(); });

  /* ---------- list groups: all hits, by message (lines that differ only in digits and spacing), by range of lines ---------- */
  var GCAP = 100000;
  var G = { mode: 'all', tpl: -1, ids: new Map(), names: [], ex: [], rep: [], exOk: [], of: new Map(), tok: 0, ready: false, busy: false };
  var idxCancelFn = null;
  function idxShow(text, frac, cancel) {
    $('idx').hidden = false; $('idxT').textContent = text; $('idxB').style.width = Math.round(Math.max(0, Math.min(1, frac)) * 100) + '%'; idxCancelFn = cancel || null; $('idxCancel').hidden = !cancel;
  }
  var idxHide = function () { $('idx').hidden = true; idxCancelFn = null; };
  $('idxCancel').addEventListener('click', function () { if (idxCancelFn) idxCancelFn(); });
  var tplKey = function (t) { return t.slice(0, 400).replace(/\d+/g, '').replace(/\s+/g, ' ').trim(); };
  /* read the text of every active hit once (the page has them; the reader sends them 400 at a time) and give each its message id */
  function buildTpl(done) {
    var tok = ++G.tok, todo = ACT.slice(0, GCAP).filter(function (n) { return !G.of.has(n); }), i = 0;
    var absorb = function (n, t, sp) {
      var k = tplKey(t), id = G.ids.get(k);
      if (id === undefined) { id = G.names.length; G.ids.set(k, id); G.names.push(k); G.ex.push(snipHtml(t, sp, 0)); G.rep.push(n); G.exOk.push(EMB || !!sp); }
      G.of.set(n, id);
    };
    /* the reader grouped on plain text (no match positions: much cheaper); the labels shown, the 300 biggest groups, get their match in bold from one request */
    var end = function () {
      var fin = function () { if (tok !== G.tok) return; G.busy = false; G.ready = true; idxHide(); done(); };
      if (EMB) { fin(); return; }
      var cnt = new Map(), id;
      ACT.slice(0, GCAP).forEach(function (n) { id = G.of.get(n); if (id !== undefined && !G.exOk[id]) cnt.set(id, (cnt.get(id) || 0) + 1); });
      var need = Array.from(cnt.entries()).sort(function (a, b) { return b[1] - a[1]; }).slice(0, 300).map(function (r) { return r[0]; });
      if (!need.length) { fin(); return; }
      rget('/lines?l=' + need.map(function (x) { return G.rep[x]; }).join(',')).then(function (d) {
        if (tok !== G.tok) return;
        var at = new Map(); need.forEach(function (x) { at.set(G.rep[x], x); });
        d.l.forEach(function (r) { var x = at.get(r[0]); if (x !== undefined && r[3]) { G.ex[x] = snipHtml(r[1], r[3], 0); G.exOk[x] = true; } });
        fin();
      }).catch(function () { if (tok === G.tok) { G.busy = false; idxHide(); readerFailed(); } });
    };
    if (!todo.length) { end(); return; }
    G.busy = true; G.ready = false;
    (function next() {
      if (tok !== G.tok) return;
      if (i >= todo.length) { end(); return; }
      idxShow('Grouping ' + fmt(todo.length) + ' hit lines…', i / todo.length, function () { G.tok++; G.busy = false; idxHide(); if (G.mode === 'msg') setListMode('all'); });
      if (EMB) {
        var t0 = performance.now();
        while (i < todo.length && performance.now() - t0 < 12) { absorb(todo[i], EMB.lines[todo[i] - 1], SPANS.get(todo[i])); i++; }
        setTimeout(next, 0);
      } else {
        var ids = todo.slice(i, i + 400); i += 400;
        rget('/lines?l=' + ids.join(',') + '&spans=0').then(function (d) { if (tok !== G.tok) return; d.l.forEach(function (r) { absorb(r[0], r[1]); }); next(); })
          .catch(function () { if (tok === G.tok) { G.busy = false; idxHide(); readerFailed(); } });
      }
    })();
  }
  function pillPaint() {
    var on2 = G.tpl >= 0 && G.ex[G.tpl] !== undefined; $('tplPill').hidden = !on2;
    if (on2) $('tplTxt').innerHTML = esc('Message: ') + G.ex[G.tpl];
  }
  function groupsRefresh() { if (G.mode !== 'all') paintGroups(); }
  function rangeSize() { var sizes = [100, 1000, 10000, 100000, 1000000, 10000000], k = 0; while (k < sizes.length - 1 && Math.ceil(N / sizes[k]) > 60) k++; return sizes[k]; }
  function paintGroups() {
    var L = hitList, h = '';
    lnodes.clear(); L.scrollTop = 0;
    if (!ACT.length) { L.innerHTML = '<div class="none">' + (HITS.length ? 'Every filter is off. Click a filter above to turn it on.' : 'No match lines to group.') + '</div>'; return; }
    if (G.mode === 'msg') {
      if (!G.ready) { L.innerHTML = '<div class="gnote">Reading the hit lines…</div>'; return; }
      var cnt = new Map(), seen = Math.min(ACT.length, GCAP), i, id;
      for (i = 0; i < seen; i++) { id = G.of.get(ACT[i]); if (id !== undefined) cnt.set(id, (cnt.get(id) || 0) + 1); }
      var rows = Array.from(cnt.entries()).sort(function (a, b) { return b[1] - a[1] || a[0] - b[0]; }), top = rows.slice(0, 300), mx = top.length ? top[0][1] : 1;
      h = '<div class="gnote">' + fmt(rows.length) + (rows.length === 1 ? ' message' : ' different messages') + ' in ' + fmt(seen) + ' hits. Digits and spacing are ignored.' + (ACT.length > seen ? ' Grouped the first ' + fmt(GCAP) + ' hits.' : '') + (rows.length > top.length ? ' Showing the ' + top.length + ' most common.' : '') + '</div>';
      top.forEach(function (r) { h += '<button type="button" class="grow" data-t="' + r[0] + '" title="Show only these hits"><span class="gt">' + G.ex[r[0]] + '</span><span class="gc">' + fmt(r[1]) + '</span><i class="gbar" style="width:' + Math.max(2, r[1] / mx * 100) + '%"></i></button>'; });
    } else {
      var size = rangeSize(), buckets = [], mx2 = 1;
      for (var from = 1; from <= N; from += size) {
        var a = lower(ACT, from), b = upper(ACT, Math.min(N, from + size - 1));
        if (b > a) { buckets.push([from, Math.min(N, from + size - 1), b - a, ACT[a]]); if (b - a > mx2) mx2 = b - a; }
      }
      h = '<div class="gnote">Hits per block of ' + fmt(size) + ' lines. Click a block to go to its first hit.</div>';
      buckets.forEach(function (x) { h += '<button type="button" class="grow" data-from="' + x[3] + '"><span class="gt">lines ' + fmt(x[0]) + ' – ' + fmt(x[1]) + '</span><span class="gc">' + fmt(x[2]) + '</span><i class="gbar" style="width:' + Math.max(2, x[2] / mx2 * 100) + '%"></i></button>'; });
    }
    L.innerHTML = h;
  }
  function setListMode(m) {
    G.mode = m;
    Array.prototype.forEach.call($('listSeg').querySelectorAll('button'), function (b) { b.setAttribute('aria-pressed', String(b.getAttribute('data-m') === m)); });
    if (m === 'all') { if (G.tpl >= 0 && !G.ready) buildTpl(refreshList); else refreshList(); return; }
    if (m === 'msg' && !G.ready) { buildTpl(paintGroups); }
    paintGroups();
  }
  $('listSeg').addEventListener('click', function (e) { var b = e.target.closest('button'); if (b) setListMode(b.getAttribute('data-m')); });
  $('tplX').addEventListener('click', function () { G.tpl = -1; refreshList(); });

  /* ---------- reader: scan the file for the search's filters (when the search listed no hits, or only some) ---------- */
  var SC = { on: false, tok: 0, done: false, from: 1, t: 0 };
  function absorbScan(h) {
    for (var i = 0; i < h.length; i += 2) {
      var n = h[i], m = h[i + 1], at = lower(HITS, n), old = 0;
      if (HITS[at] === n) { old = HMASK[at]; HMASK[at] = old | m; } else { HITS.splice(at, 0, n); HMASK.splice(at, 0, m); }
      FILT.forEach(function (f) { if ((m >> f.slot & 1) && !(old >> f.slot & 1)) f.count++; });
      blocks.delete(Math.floor((n - 1) / BS)); snipCache.delete(n);
    }
  }
  function scanUpdate(final) {
    var now = performance.now(); if (!final && now - SC.t < 300) return; SC.t = now;
    rebuildAct(); G.ready = false; chipsPaint(); syncTools(); syncMenu();
    if (qHits) { if (final) refreshList(); return; }
    LIST = ACT; LV++; listPaint(); groupsRefresh(); updateCount(); paint(); redrawRail();
    if (G.mode === 'msg') buildTpl(paintGroups);
  }
  function startScan() {
    if (!RD || SC.on || SC.done) return;
    var tok = ++SC.tok; SC.on = true; syncMenu(); listPaint();
    var stop = function () { SC.tok++; SC.on = false; idxHide(); scanUpdate(true); listPaint(); };
    (function next() {
      if (tok !== SC.tok) return;
      idxShow('Scanning the file for the filters… ' + Math.round(Math.min(1, SC.from / N) * 100) + '%', SC.from / N, stop);
      rget('/scan?from=' + SC.from).then(function (d) {
        if (tok !== SC.tok) return;
        if (d.err) { SC.on = false; idxHide(); toast(d.err); syncMenu(); listPaint(); return; }
        absorbScan(d.h);
        if (d.next) { SC.from = d.next; scanUpdate(false); next(); return; }
        SC.on = false; SC.done = true; idxHide(); scanUpdate(true); listPaint(); toast('Scan finished: ' + fmt(ACT.length) + ' matching lines');
      }).catch(function () { if (tok === SC.tok) { SC.on = false; idxHide(); readerFailed(); syncMenu(); listPaint(); } });
    })();
  }

  /* ---------- structured view of Office files: the same lines, drawn as pages, slides and grids ---------- */
  var lt = function (n) { var t = EMB.lines[n - 1]; return t.length > LCAP ? t.slice(0, LCAP) : t; };
  var txt = function (n) { return renderText(lt(n), SPANS.get(n)); };
  function slAttr(n, cls) { return 'class="sl' + (cls ? ' ' + cls : '') + (isHit(n) ? ' hit' : '') + (n === curLine ? ' cur' : '') + (inSel(n) ? ' sel' : '') + '" data-n="' + n + '"'; }
  function renderSub(t, sp, off, len) {
    var shifted = sp ? sp.filter(function (x) { return x[1] > off && x[0] < off + len; }).map(function (x) { return [Math.max(0, x[0] - off), Math.min(len, x[1] - off), x[2]]; }) : null;
    return renderText(t.slice(off, off + len), shifted);
  }
  function figs(n) {
    var r = ST.ref.get(n), h = ''; if (!r) return '';
    r.forEach(function (ix) { if (SD.imgs[ix]) h += '<div class="fig"><img alt="" src="' + SD.imgs[ix] + '"></div>'; }); return h;
  }
  var metaOf = function (n) { return (SD.meta && SD.meta[n - 1]) || ''; };
  var cellOf = function (n) { var m = /^c(\d+)\.(\d+)\.(\d+)$/.exec(metaOf(n)); return m ? [+m[1], +m[2], +m[3]] : null; };
  function tableHtml(a, e) {
    var rows = [], row = null, lastKey = '', n;
    for (n = a; n <= e; n++) {
      var c = cellOf(n), key = c[1] + ':' + c[2];
      if (!row || c[1] !== row.r) { row = { r: c[1], cells: [] }; rows.push(row); lastKey = ''; }
      if (key !== lastKey) { row.cells.push({ lines: [] }); lastKey = key; }
      row.cells[row.cells.length - 1].lines.push(n);
    }
    var h = '<table class="tbl">';
    rows.forEach(function (r, ri) {
      var tag = ri === 0 && rows.length > 1 ? 'th' : 'td';
      h += '<tr>' + r.cells.map(function (cell) {
        return '<' + tag + '>' + cell.lines.map(function (ln) { return '<div ' + slAttr(ln, lt(ln).trim() ? '' : 'blank') + '>' + txt(ln) + '</div>' + figs(ln); }).join('') + '</' + tag + '>';
      }).join('') + '</tr>';
    });
    return h + '</table>';
  }
  function blockHtml(n) {
    var m = metaOf(n), t = lt(n), cls = /^(h[1-9]|title)$/.test(m) ? 'h ' + m : m === 'li' ? 'li' : t.trim() ? '' : 'blank';
    return '<' + (cls.charAt(0) === 'h' ? 'div' : 'p') + ' ' + slAttr(n, cls) + '>' + txt(n) + '</' + (cls.charAt(0) === 'h' ? 'div' : 'p') + '>' + figs(n);
  }
  function pageHtml(u) {
    var h = '<section class="page" data-s="' + u.a + '"><div class="pn"><span>Page ' + u.num + '</span><span>lines ' + fmt(u.a) + ' – ' + fmt(u.b) + '</span></div>';
    for (var n = u.a; n <= u.b;) {
      var c = cellOf(n);
      if (c) { var e = n; while (e < u.b && cellOf(e + 1) && cellOf(e + 1)[0] === c[0]) e++; h += tableHtml(n, e); n = e + 1; } else { h += blockHtml(n); n++; }
    }
    return h + '</section>';
  }
  function slideHtml(u) {
    var b0 = u.a + 1, b1 = u.notes ? u.notes - 1 : u.b, first = 0, n, h = '<div class="slide" data-s="' + u.a + '"><div ' + slAttr(u.a, 'sn') + '>' + (isHit(u.a) ? esc(lt(u.a)) : 'Slide ' + u.num) + '</div>';
    for (n = b0; n <= b1; n++) if (lt(n).trim()) { first = n; break; }
    if (first) h += '<h4 ' + slAttr(first, 'st') + '>' + txt(first) + '</h4>';
    h += figs(u.a);
    for (n = b0; n <= b1; n++) if (n !== first && lt(n).trim()) h += '<div ' + slAttr(n, 'sb') + '>' + txt(n) + '</div>';
    if (u.notes) {
      h += '<div class="snotes"><b ' + slAttr(u.notes) + '>Notes</b>';
      for (n = u.notes + 1; n <= u.b; n++) if (lt(n).trim()) h += '<div ' + slAttr(n, 'sb notes') + '>' + txt(n) + '</div>';
      h += '</div>';
    }
    return h + '</div>';
  }
  function diagramHtml(u) {
    var h = '<div class="slide" data-s="' + u.a + '"><h4 ' + slAttr(u.a, 'st') + '>' + (isHit(u.a) ? esc(lt(u.a)) : 'SmartArt diagram ' + u.num) + '</h4>';
    for (var n = u.a + 1; n <= u.b; n++) if (lt(n).trim()) h += '<div ' + slAttr(n, 'sb') + '>' + txt(n) + '</div>';
    return h + '</div>';
  }
  function sheetHtml(u) {
    var h = '<div data-s="' + u.a + '">';
    if (u.head) h += '<div class="sh"><span ' + slAttr(u.a) + '>' + (isHit(u.a) ? esc(lt(u.a)) : 'Sheet ' + u.num) + '</span><small>' + (u.rows ? fmt(u.rows) + ' rows' : 'empty') + '</small></div>';
    if (u.to >= u.from) {
      h += '<div class="grid-wrap"><table class="grid"><thead><tr><th class="rn">Line</th>';
      for (var c = 1; c <= u.cols; c++) h += '<th>' + c + '</th>';
      h += '</tr></thead><tbody>';
      for (var n = u.from; n <= u.to; n++) {
        var t = lt(n), sp = SPANS.get(n), off = 0, parts = t.split('\t');
        h += '<tr ' + slAttr(n) + '><th class="rn">' + fmt(n) + '</th>';
        for (c = 0; c < u.cols; c++) {
          var p = parts[c];
          if (p === undefined) { h += '<td></td>'; continue; }
          h += '<td' + (/^-?\d[\d,.]*%?$/.test(p.trim()) ? ' class="num"' : '') + '>' + renderSub(t, sp, off, p.length) + '</td>'; off += p.length + 1;
        }
        h += '</tr>';
      }
      h += '</tbody></table></div>';
    }
    return h + '</div>';
  }
  var strUnitHtml = function (u) { return u.kind === 'slide' ? slideHtml(u) : u.kind === 'diagram' ? diagramHtml(u) : u.kind === 'sheet' ? sheetHtml(u) : pageHtml(u); };
  function pageUnits(units, a, b) {   // plain pages of about 40 lines; a table is never split
    var i = a, num = units.filter(function (u) { return u.kind === 'page'; }).length;
    while (i <= b) {
      var s0 = i, cnt = 0;
      do { i++; cnt++; } while (i <= b && (cnt < 40 || (cellOf(i) && cellOf(i - 1) && cellOf(i)[0] === cellOf(i - 1)[0])));
      units.push({ kind: 'page', a: s0, b: i - 1, num: ++num });
    }
  }
  function strUnits() {
    var units = [], cur = null, m, n, t;
    if (SD.kind === 'docx') { pageUnits(units, 1, N); return units; }
    var plainFrom = 0, flush = function (to) { if (plainFrom) { pageUnits(units, plainFrom, to); plainFrom = 0; } };
    for (n = 1; n <= N; n++) {
      t = EMB.lines[n - 1];
      if (SD.kind === 'pptx' && (m = /^--- Slide (\d+) ---$/.exec(t))) { flush(n - 1); cur = { kind: 'slide', a: n, b: n, num: +m[1], notes: 0 }; units.push(cur); }
      else if (SD.kind === 'pptx' && cur && cur.kind === 'slide' && /^--- Slide \d+ notes ---$/.test(t)) { cur.notes = n; cur.b = n; }
      else if (SD.kind === 'pptx' && (m = /^--- SmartArt diagram (\d+) ---$/.exec(t))) { flush(n - 1); cur = { kind: 'diagram', a: n, b: n, num: +m[1] }; units.push(cur); }
      else if (SD.kind === 'xlsx' && (m = /^--- Sheet (\d+) ---$/.exec(t))) { flush(n - 1); cur = { kind: 'sheet', a: n, b: n, num: +m[1], rows: 0, cols: 1 }; units.push(cur); }
      else if (cur && cur.kind !== 'page') { cur.b = n; if (cur.kind === 'sheet') { cur.rows++; cur.cols = Math.max(cur.cols, t.split('\t').length); } }
      else if (!plainFrom) plainFrom = n;
    }
    flush(N);
    if (SD.kind !== 'xlsx') return units;
    var out = [];   // a sheet is drawn in chunks of 200 rows
    units.forEach(function (u) {
      if (u.kind !== 'sheet') { out.push(u); return; }
      var from = u.a + 1;
      do {
        var to = Math.min(u.b, from + 199);
        out.push({ kind: 'sheet', a: from === u.a + 1 ? u.a : from, b: to < u.b ? to : u.b, num: u.num, rows: u.rows, cols: u.cols, head: from === u.a + 1, from: from, to: to });
        from = to + 1;
      } while (from <= u.b);
    });
    return out;
  }
  var unitIdx = function (n) { return Math.max(0, upper(ST.starts, n) - 1); };
  var strList = function () { return ST.sls || (ST.sls = ST.inner.querySelectorAll('.sl')); };
  function strAt(y) {   // the line of the last element that starts at or above screen position y
    var L = strList(), lo = 0, hi = L.length - 1, best = -1;
    while (lo <= hi) { var m = (lo + hi) >> 1; if (L[m].getBoundingClientRect().top <= y) { best = m; lo = m + 1; } else hi = m - 1; }
    return best < 0 ? 0 : +L[best].getAttribute('data-n');
  }
  function strTop() { var L = strList(); return strAt(ST.el.getBoundingClientRect().top + 1) || (L.length ? +L[0].getAttribute('data-n') : 1); }
  function strMore() {
    var el = ST.el, k = 0;
    while (ST.shown < ST.units.length && k++ < 12 && el.scrollHeight - el.scrollTop - el.clientHeight < el.clientHeight * 2) { ST.inner.insertAdjacentHTML('beforeend', strUnitHtml(ST.units[ST.shown++])); ST.sls = null; }
  }
  function strMark() {
    Array.prototype.forEach.call(strList(), function (el) {
      var n = +el.getAttribute('data-n'); el.classList.toggle('hit', isHit(n)); el.classList.toggle('cur', n === curLine); el.classList.toggle('sel', inSel(n));
    });
  }
  function strRender() {
    var top = ST.el.scrollTop, h = ''; for (var i = 0; i < ST.shown; i++) h += strUnitHtml(ST.units[i]);
    ST.inner.innerHTML = h; ST.sls = null; ST.el.scrollTop = top;
  }
  function strPaint() {
    if (!ST.el) return;
    var key = fre ? fre.source + '/' + fre.flags : '';
    if (key !== ST.key || ST.dirty) { ST.key = key; ST.dirty = false; strRender(); } else strMark();
    strMore();
  }
  function strGo(n) {
    var ui = unitIdx(n);
    while (ST.shown <= ui && ST.shown < ST.units.length) { ST.inner.insertAdjacentHTML('beforeend', strUnitHtml(ST.units[ST.shown++])); ST.sls = null; }
    strMark();
    var el = ST.inner.querySelector('.sl[data-n="' + n + '"]');
    for (var k = n - 1; !el && k >= ST.units[ui].a && n - k < 60; k--) el = ST.inner.querySelector('.sl[data-n="' + k + '"]');
    if (!el) el = ST.inner.querySelector('[data-s="' + ST.units[ui].a + '"]');
    if (el) { var r = el.getBoundingClientRect(), c = ST.el.getBoundingClientRect(); if (r.top < c.top + 8 || r.bottom > c.bottom - 8) ST.el.scrollTop += r.top - c.top - c.height / 3; }
    strMore();
  }
  function strInit() {
    if (!N || N > 60000) { SD = null; return; }
    (SD.refs || []).forEach(function (r) { var a = ST.ref.get(r[0]); if (!a) ST.ref.set(r[0], a = []); a.push(r[1]); });
    ST.units = strUnits(); ST.starts = ST.units.map(function (u) { return u.a; });
    $('doc').insertAdjacentHTML('beforeend', '<div class="sview" id="sview" tabindex="0" aria-label="Structured view" hidden><div class="inner"></div></div>');
    ST.el = $('sview'); ST.inner = ST.el.firstChild; ST.boxEl = box;
    ST.el.addEventListener('scroll', schedPaint);
    ST.el.addEventListener('click', function (e) {
      if (String(window.getSelection()).length) return;
      var s = e.target.closest('.sl'); if (s) pickLine(+s.getAttribute('data-n'), e.shiftKey);
    });
    if (pref.str) setStruct(true);
  }
  function setStruct(v) {
    if (!SD || !ST.el || v === ST.on) return;
    var at = anchorLine();
    pref.str = v; save(); ST.on = v;
    if (v) {
      box = null; ST.boxEl.hidden = true; ST.el.hidden = false; ST.dirty = true; ST.shown = 0; ST.inner.innerHTML = ''; ST.sls = null;
      strPaint(); strGo(at);
    } else {
      box = ST.boxEl; ST.el.hidden = true; box.hidden = false;
      box.classList.toggle('wrapon', wrapOn()); if (wrapOn()) { measureChar(); wrapReset(); }
      setWidth(); RGEN++; revealRow(rowOfLine(at), true); paint();
    }
    syncTools(); redrawRail(); afterScroll();
  }

  /* ---------- start ---------- */
  function applyHash() {
    var m = /^#L(\d+)(?:-L(\d+))?$/.exec(location.hash); if (!m || !N) return;
    var a = Math.min(N, +m[1]), b = m[2] ? Math.min(N, +m[2]) : a;
    sel = { anchor: a, a: Math.min(a, b), b: Math.max(a, b) }; goLine(a); RGEN++; paint(); syncMenu(); updateStatus();
  }
  function start() {
    recomputeOn(); rebuildAct(); LIST = ACT; LV++;
    chipsPaint(); syncTools(); syncMenu(); updateCount(); listPaint();
    $('foot').innerHTML = '<span>' + esc(CFG.foot || '') + '</span><span>The original file is not changed</span><span id="fMatches">' + fmt(ACT.length) + ' matches</span>' +
      (CFG.capped ? '<span>Showing the first ' + fmt(N) + ' of ' + fmt(CFG.total) + ' lines</span>' : '');
    updateCount();
    if (PDF) {
      $('rail').hidden = true;
      $('doc').classList.add('flush');
      $('doc').innerHTML = '<div class="pdfwrap">' + (CFG.ocr ? '<div class="banner">' + esc(CFG.ocr) + '</div>' : '') + '<iframe class="pdfframe" title="PDF" src="' + esc(CFG.pdf) + '"></iframe></div>';
      pdfFrame = $('doc').querySelector('iframe');
      updateStatus(); return;
    }
    $('doc').classList.add('flush');
    if (!N) { $('doc').innerHTML = '<div class="empty">This file has no lines to show.</div>'; $('rail').hidden = true; updateStatus(); return; }
    $('doc').innerHTML = '<div class="logsrc" id="logsrc" tabindex="0" aria-label="File text"><div class="mo"><div class="mosp"><div class="movr"></div></div></div></div>';
    box = $('logsrc'); sp = box.querySelector('.mosp'); vr = box.querySelector('.movr');
    if (EMB) { for (var i = 0; i < N; i++) { var l = EMB.lines[i].length; if (l > maxLen) maxLen = l; } maxLen = Math.min(maxLen, LCAP); }
    measureChar();
    WR.on = !!pref.wrap && canWrap();
    if (WR.on) { box.classList.add('wrapon'); wrapReset(); if (wrapTooTall()) wrapRefuse(); }
    setWidth();
    box.addEventListener('scroll', schedPaint); box.addEventListener('click', onBoxClick);
    rulerEvents();
    if (pref.mo) { MO.on = true; moBuild(); }
    var first = LIST[0] || 0;
    if (first) { curLine = first; revealRow(rowOfLine(first), true); }
    paint(); redrawRail(); updateCount(); updateStatus(); listReveal(curLine); listPaint();
    if (SD) strInit();
    syncTools(); syncMenu(); applyHash();
  }

  if (pref.theme) root.dataset.theme = pref.theme;
  document.title = CFG.name + ' - Toolbench';
  $('vName').innerHTML = '<span class="badge' + (RD ? ' big' : '') + '">' + esc(RD ? 'LARGE' : CFG.badge) + '</span><span class="n">' + esc(CFG.name) + '</span>';
  $('vMeta').textContent = CFG.path + '  |  ' + CFG.size + '  |  modified ' + CFG.mod;
  if (CFG.stale) { $('stale').hidden = false; $('staleTxt').textContent = 'This file changed after the search ran (' + CFG.stale + '). Highlights and positions may not match what Search found.'; }
  $('rdPill').hidden = !RD;
  if (EMB) {
    EMB.hits.forEach(function (h) { HITS.push(h[0]); HMASK.push(h[1]); SPANS.set(h[0], h[2]); });
    start();
  } else if (RD) {
    paintReader();
    rget('/hits').then(function (d) { for (var i = 0; i < d.h.length; i += 2) { HITS.push(d.h[i]); HMASK.push(d.h[i + 1]); } start(); })
      .catch(function () { readerUp = false; start(); paintReader(); });
  } else start();
})();
