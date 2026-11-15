#!/usr/bin/env python3
"""Score catalog SAME_ENTITY edges on FAA ADs against labels the ADs carry themselves.

  python3 scripts/score_faa_ad_edges.py <ad-folder> <edges.tsv from the edges_dump example>

Labels (no hand work): own AD number from the header; "supersedes AD x" / "AD x is superseded" cites; the helicopter
or aircraft models named in the summary. A pair is `chain` when one AD supersedes or cites the other (or both sit in
one citation component) and `model` when they share a model designation. Prints recall on supersede pairs and
precision per confidence band against each label.
"""
import re, sys, os, html, collections
d, tsv = sys.argv[1], sys.argv[2]
docs = {}
for f in os.listdir(d):
    if f.endswith(".txt"):
        docs[f] = html.unescape(re.sub(r"<[^>]+>", " ", open(os.path.join(d, f), errors="replace").read()))
AD = re.compile(r"\bAD\s+(\d{4}-\d{2}-\d{2})\b")
own, text = {}, {}
for f, t in docs.items():
    m = re.search(r"Amendment\s+39-\d+;\s*AD\s+(\d{4}-\d{2}-\d{2})", t) or AD.search(t)
    if m: own[f] = m.group(1)
    text[f] = re.sub(r"\s+", " ", t)
by_num = {v: k for k, v in own.items()}
sup, cites = set(), collections.defaultdict(set)
for f, t in text.items():
    for m in re.finditer(r"supersedes\s+(?:\w+\s+){0,6}?AD\s+(\d{4}-\d{2}-\d{2})", t, re.I):
        n = m.group(1)
        if n in by_num and by_num[n] != f: sup.add(frozenset((f, by_num[n])))
    for n in set(AD.findall(t)):
        if n in by_num and by_num[n] != f: cites[f].add(by_num[n])
parent = {f: f for f in docs}
def find(x):
    while parent[x] != x: parent[x] = parent[parent[x]]; x = parent[x]
    return x
for f, cs in cites.items():
    for c in cs: parent[find(f)] = find(c)
MODEL = re.compile(r"\b(?:EC|AS|SA|AS3|H|MBB|BK|BO)[ -]?\d{2,3}[A-Z]?\d?[A-Z]?\d?\b")
def models(t):
    s = re.search(r"SUMMARY:(.{0,900})", t); s = s.group(1) if s else t[:2000]
    return {re.sub(r"[ -]", "", m) for m in MODEL.findall(s)}
mod = {f: models(t) for f, t in text.items()}
edges = {}
for line in open(tsv):
    parts = line.rstrip("\n").split("\t")
    if len(parts) != 5: continue
    s, t, k, c, _ = parts
    if k != "SAME_ENTITY": continue
    s, t = os.path.basename(s), os.path.basename(t)
    edges[frozenset((s, t))] = max(float(c), edges.get(frozenset((s, t)), 0))
print(f"{len(docs)} docs, {len(own)} with AD number, {len(sup)} supersede pairs in set, {len(edges)} SAME_ENTITY edges")
hit = [p for p in sup if p in edges]
print(f"supersede recall: {len(hit)}/{len(sup)} = {len(hit)/max(1,len(sup)):.3f}")
def chain(p):
    a, b = tuple(p); return b in cites[a] or a in cites[b] or find(a) == find(b)
def model(p):
    a, b = tuple(p); return bool(mod[a] & mod[b])
print("band\tedges\tchain\tmodel\tchain_or_model\tneither")
bands = [(0.0, 0.2), (0.2, 0.3), (0.3, 0.5), (0.5, 0.7), (0.7, 1.01)]
for lo, hi in bands:
    es = [p for p, c in edges.items() if lo <= c < hi]
    if not es: continue
    ch = sum(chain(p) for p in es); mo = sum(model(p) for p in es); either = sum(chain(p) or model(p) for p in es)
    print(f"{lo:.1f}-{hi:.1f}\t{len(es)}\t{ch/len(es):.3f}\t{mo/len(es):.3f}\t{either/len(es):.3f}\t{len(es)-either}")
print("sample of edges with neither label (judge by hand):")
import random; random.seed(1)
neither = [p for p in edges if not (chain(p) or model(p))]
for p in random.sample(neither, min(8, len(neither))):
    a, b = sorted(p); print(f"  {edges[p]:.2f}  {a} [{','.join(sorted(mod[a]))}]  {b} [{','.join(sorted(mod[b]))}]")
