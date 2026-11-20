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

  /* ---------- preferences (localStorage may be refused on file:// pages) ---------- */
  var pref = { on: [true, true, true, true, true, true], railw: 272, collapsed: false, fs: 1, mo: false, theme: '' };
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

  /* ---------- virtual scrolling: only the rows on screen exist; a list taller than the browser allows is mapped onto a shorter scrollbar ---------- */
  function vgeom(el, total, rh) {
    var vh = el.clientHeight, nat = total * rh, H = Math.min(nat, MAXH), scaled = nat > MAXH;
    return { vh: vh, rh: rh, total: total, H: Math.max(H, vh), scaled: scaled, vis: vh / rh, kTop: scaled ? el.scrollTop / Math.max(1, H - vh) * Math.max(0, total - vh / rh) : el.scrollTop / rh };
  }
  function vscroll(el, g, want) {
    var maxK = Math.max(0, g.total - g.vis); want = Math.max(0, Math.min(want, maxK));
    el.scrollTop = g.scaled ? (maxK ? want / maxK * (g.H - g.vh) : 0) : want * g.rh;
  }
  function vlayer(sp, vr, el, total, rh) { sp.style.height = Math.max(Math.min(total * rh, MAXH), el.clientHeight) + 'px'; vr.style.height = el.clientHeight + 'px'; return vgeom(el, total, rh); }

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
  var topLine = function () { var d = rowAt(Math.max(0, Math.min(rowCount() - 1, Math.floor(vgeom(box, rowCount(), LH).kTop)))); return d.n || (MO.GS[d.fold] || 1); };

  function measureChar() {
    var s = document.createElement('span'); s.style.cssText = 'position:absolute;visibility:hidden;white-space:pre;font:inherit'; s.textContent = new Array(101).join('M');
    box.appendChild(s); charW = s.getBoundingClientRect().width / 100 || 7.5; s.remove();
  }
  function setWidth() { if (sp) sp.style.minWidth = Math.ceil(76 + 24 + maxLen * charW + (maxLen >= LCAP ? 110 : 0)) + 'px'; }

  var paintQ = 0;
  function schedPaint() { if (!paintQ) paintQ = requestAnimationFrame(function () { paintQ = 0; paint(); afterScroll(); }); }
  function rowKey(d, ready) {
    return d.fold !== undefined ? 'f' + d.fold + ':' + d.hidden : 'n' + d.n + ':' + (ready ? 1 : 0) + ':' + (d.n === curLine ? 1 : 0) + ':' + (inSel(d.n) ? 1 : 0) + ':' + (isHit(d.n) ? 1 : 0) + ':' + RGEN + ':' + MO.gen;
  }
  function paint() {
    if (!box) return;
    var R = rowCount(), g = vlayer(sp, vr, box, R, LH);
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
      nd.style.transform = 'translateY(' + ((r - g.kTop) * LH) + 'px)';
    }
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
    var g = vgeom(box, rowCount(), LH);
    if (!force && r >= g.kTop && r + 1 <= g.kTop + g.vis) return;
    vscroll(box, g, r - g.vis / 3);
  }
  function goLine(n) {
    n = Math.max(1, Math.min(N || 1, n)); curLine = n;
    if (PDF) { pdfSearch(n); } else if (box) { revealRow(rowOfLine(n)); RGEN++; paint(); }
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
    var g = box ? vgeom(box, rowCount(), LH) : null, a = topLine();
    return '<div class="peek-h"><b>line ' + fmt(n) + '</b><span>match ' + fmt(upper(LIST, n)) + ' of ' + fmt(LIST.length) + '</span></div><div class="code">' + rowsH + '</div>' +
      '<div class="peek-f">' + chips + '<span class="sp"></span>' + heat(n / N, g ? [a / N, Math.min(N, a + g.vis) / N] : null) + '<small>' + Math.round(n / N * 100) + '% into the file. Click to jump</small></div>';
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
    var g = $('flagsDoc'); if (!box) return;
    var R = rowCount(), vg = vgeom(box, R, LH), ch = g.clientHeight || vg.vh;
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
    if (!box) return;
    var a = topLine() - 1, b = Math.min(N, a + Math.ceil(box.clientHeight / LH)), w = $('rwin'), mk = $('rmk');
    w.style.top = (a / N * 100) + '%'; w.style.height = ((b - a) / N * 100) + '%';
    mk.style.top = ((curLine - 1) / N * 100) + '%'; mk.hidden = !curLine;
  }
  function tipAt(e, a, b) {
    var hits = upper(LIST, b) - lower(LIST, a), t = $('tip');
    t.innerHTML = '<b>≈ line ' + fmt(Math.round((a + b) / 2)) + '</b> · ' + fmt(hits) + ' ' + (hits === 1 ? 'hit' : 'hits') + '<small>click to jump to the next hit here</small>';
    t.hidden = false; var w = t.offsetWidth; t.style.left = Math.max(8, Math.min(innerWidth - w - 8, e.clientX - w - 14)) + 'px'; t.style.top = Math.max(8, e.clientY - 20) + 'px';
  }
  function rulerEvents() {
    var el = $('ruler'), frac = function (e) { var r = el.getBoundingClientRect(); return Math.max(0, Math.min(1, (e.clientY - r.top) / r.height)); }, half = function () { return 2 / Math.max(1, el.clientHeight); };
    el.addEventListener('mousemove', function (e) { var f = frac(e); tipAt(e, Math.max(1, Math.floor((f - half()) * N)), Math.min(N, Math.ceil((f + half()) * N))); });
    el.addEventListener('mouseleave', function () { $('tip').hidden = true; });
    el.addEventListener('click', function (e) {
      var from = Math.max(1, Math.floor((frac(e) - half()) * N)), i = lower(LIST, from), n = LIST[i] || LIST[LIST.length - 1];
      if (n) goLine(n); else goLine(Math.max(1, Math.round(frac(e) * N)));
    });
  }
  function afterScroll() { paintFlags(); drawMark(); updateStatus(); }
  function redrawRail() { flagKey = ''; drawRuler(); paintFlags(); drawMark(); }

  /* ---------- match list (virtual, one row per hit) ---------- */
  var hitList = $('hitList'), lnodes = new Map();
  function snipFor(n) {
    var s = snipCache.get(n); if (s !== undefined) return s;
    var d = lineData(n); if (!d) return undefined;
    var sps = d.sp || [], p = sps.find(function (x) { return on[x[2]]; }) || sps[0], t = d.t;
    if (!p) s = ws(t.slice(0, 110));
    else {
      var a = Math.max(0, p[0] - 28), b = Math.min(t.length, p[1] + 80);
      s = (a ? '…' : '') + ws(t.slice(a, p[0])) + '<b>' + ws(t.slice(p[0], p[1])) + '</b>' + ws(t.slice(p[1], b)) + (b < t.length ? '…' : '');
    }
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
    if (!LIST.length) {
      lnodes.clear();
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
    var k = lower(LIST, n); if (LIST[k] !== n) return;
    var g = vgeom(hitList, LIST.length, RH); if (k >= g.kTop && k + 1 <= g.kTop + g.vis) return;
    vscroll(hitList, g, k - g.vis / 3);
  }
  hitList.addEventListener('scroll', function () { listPaint(); });
  hitList.addEventListener('click', function (e) { var b = e.target.closest('.mrow'); if (b) goLine(+b.getAttribute('data-n')); });
  hitList.addEventListener('keydown', function (e) {
    if (e.key === 'ArrowDown') { e.preventDefault(); step(1); } else if (e.key === 'ArrowUp') { e.preventDefault(); step(-1); }
    else if (e.key === 'Home') { e.preventDefault(); if (LIST.length) goLine(LIST[0]); } else if (e.key === 'End') { e.preventDefault(); if (LIST.length) goLine(LIST[LIST.length - 1]); }
  });

  /* The list filter: the typed text against the line number and the line text. Embedded pages test the lines directly; the reader scans the file. */
  var qTok = 0, qTimer = 0;
  function refreshList() {
    var t = $('q').value.trim().toLowerCase(), tok = ++qTok;
    var finish = function (arr) {
      qHits = arr; LIST = arr || ACT; LV++; lnodes.clear(); hitList.innerHTML = ''; hitList.scrollTop = 0; $('q').classList.remove('busy');
      if (MO.on) { moBuild(); if (curLine) revealRow(moRowOfLine(curLine), true); }
      if (box) { RGEN++; paint(); }
      if (curLine) listReveal(curLine);
      listPaint(); updateCount(); redrawRail(); updateStatus();
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
    refreshList(); chipsPaint();
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
    var at = curLine || (box ? topLine() : 1);
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
    $('moBtn').disabled = !text; $('moBtn').setAttribute('aria-pressed', String(!!pref.mo && text));
    $('moBtn').title = text ? 'Show only the lines that match, with ' + CTX + ' lines of context. The gaps fold away.  (M)' : 'Matches only needs a text view';
    $('fsDown').disabled = $('fsUp').disabled = !text; $('findBtn').disabled = $('gotoBtn').disabled = !text;
    $('railBtn').setAttribute('aria-pressed', String(!pref.collapsed)); vb.classList.toggle('collapsed', !!pref.collapsed);
    $('app').style.setProperty('--fs', FS[pref.fs][0] + 'px'); $('app').style.setProperty('--lh', FS[pref.fs][1] + 'px');
  }
  function setFs(d) {
    var f = Math.max(0, Math.min(FS.length - 1, pref.fs + d)); if (f === pref.fs || PDF) return;
    var at = curLine || topLine(); pref.fs = f; save(); LH = FS[f][1]; syncTools();
    if (box) { measureChar(); setWidth(); RGEN++; revealRow(rowOfLine(at), true); paint(); redrawRail(); }
  }
  function toggleRail() { pref.collapsed = !pref.collapsed; save(); syncTools(); setTimeout(function () { listPaint(); paint(); redrawRail(); }, 0); }
  $('railBtn').addEventListener('click', toggleRail);
  $('moBtn').addEventListener('click', function () { setMO(!pref.mo); });
  $('fsDown').addEventListener('click', function () { setFs(-1); });
  $('fsUp').addEventListener('click', function () { setFs(1); });
  (function () {
    var spl = $('split'), vb = $('vbody'), drag = false;
    var apply = function (w) { pref.railw = Math.max(200, Math.min(520, Math.round(w))); vb.style.setProperty('--railw', pref.railw + 'px'); };
    apply(pref.railw);
    spl.addEventListener('pointerdown', function (e) { if (pref.collapsed) return; drag = true; spl.setPointerCapture(e.pointerId); spl.classList.add('drag'); });
    spl.addEventListener('pointermove', function (e) { if (drag) apply(e.clientX - vb.getBoundingClientRect().left); });
    var end = function () { if (!drag) return; drag = false; spl.classList.remove('drag'); save(); listPaint(); paint(); redrawRail(); };
    spl.addEventListener('pointerup', end); spl.addEventListener('pointercancel', end);
    spl.addEventListener('dblclick', toggleRail);
    spl.addEventListener('keydown', function (e) { if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') { e.preventDefault(); apply(pref.railw + (e.key === 'ArrowRight' ? 16 : -16)); save(); } });
  })();

  var menu = $('menu'), menuBtn = $('menuBtn');
  function syncMenu() { $('mCopySel').disabled = !sel; $('mExport').disabled = !HITS.length; }
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

  /* ---------- status bar ---------- */
  function updateStatus() {
    var pos = '';
    if (box && N) { var a = topLine(); pos = 'Ln ' + fmt(a) + ' of ' + fmt(N) + ' · ' + Math.round(a / N * 100) + '%'; }
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
    else if (k === '+' || k === '=') setFs(1);
    else if (k === '-') setFs(-1);
    else if (k === '[') toggleRail();
    else if (k === '?') { $('kbd').hidden = false; $('kbdClose').focus(); }
  });
  $('prev').addEventListener('click', function () { step(-1); });
  $('next').addEventListener('click', function () { step(1); });
  window.addEventListener('resize', function () { if (box) { paint(); redrawRail(); } listPaint(); });

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
    measureChar(); setWidth();
    box.addEventListener('scroll', schedPaint); box.addEventListener('click', onBoxClick);
    rulerEvents();
    if (pref.mo) { MO.on = true; moBuild(); }
    var first = LIST[0] || 0;
    if (first) { curLine = first; revealRow(rowOfLine(first), true); }
    paint(); redrawRail(); updateCount(); updateStatus(); listReveal(curLine); listPaint();
    applyHash();
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
