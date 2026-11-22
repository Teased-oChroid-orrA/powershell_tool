#!/usr/bin/env python3
"""Score the detector against the designed Hole features of prepared Fusion 360 Gallery assemblies (see README.md).
Usage: score.py WORK_DIR PATH/TO/assembly-score [report.json]

Labels come only from the dataset's `holes` (a designer's Hole feature: diameter, origin, direction), never from the detector.
 * hole recall: a labelled hole is found when a detected hole on a body of the same index has the same axis (parallel within 1 degree,
   the label origin within 0.1 mm of it) and diameter (0.05 mm). Body index = position of the labelled body in the manifest, a body
   that appears in several instances matches any of them.
 * labelled sites: two labelled holes of different body instances on one axis with equal diameters (1 %), found as a site when a
   detected site names both bodies on that line. (Instances of one body share a label, so a pair needs two distinct body ids.)
 * near-closed: partial bores at or above NEAR_CLOSED_MIN_TURN; a candidate that lies on a labelled hole's axis with its diameter is a
   designed full hole the reader split short of a turn (a false candidate)."""
import json, math, subprocess, sys

ANG = math.cos(math.radians(1.0))


def dot(a, b): return sum(x * y for x, y in zip(a, b))
def sub(a, b): return [x - y for x, y in zip(a, b)]


def perp_dist(p, cp, ca):
    d = sub(p, cp)
    t = dot(d, ca)
    return math.sqrt(sum((d[i] - t * ca[i]) ** 2 for i in range(3)))


def same_axis(p1, a1, p2, a2, tol=0.1):
    return abs(dot(a1, a2)) >= ANG and perp_dist(p1, p2, a2) <= tol


def main():
    work, exe = sys.argv[1], sys.argv[2]
    rep = {'assemblies': 0, 'failed': [], 'unread_parts': 0, 'labelled': 0, 'found': 0, 'missed': [], 'sites': 0, 'site_pairs': 0, 'site_pairs_found': 0,
           'bridged': 0, 'bridged_on_label': 0, 'near': [], 'times': [], 'holes': 0}
    for a in json.load(open(work + '/manifest.json')):
        try:
            r = subprocess.run([exe] + a['files'], capture_output=True, text=True, timeout=180)
            res = json.loads(r.stdout)
        except Exception as e:
            rep['failed'].append((a['id'], type(e).__name__))
            continue
        if 'error' in res:
            rep['failed'].append((a['id'], res['error'][:80]))
            continue
        rep['assemblies'] += 1
        rep['unread_parts'] += len(res['unread'])
        rep['times'].append(res['seconds'])
        rep['holes'] += len(res['holes'])
        rep['sites'] += len(res['sites'])
        body_ids = a['bodies']
        # detected body index (argument position) -> manifest body id
        label = lambda occ: body_ids[occ]
        labelled = list(a['holes'])
        for h in labelled:
            rep['labelled'] += 1
            hit = [d for d in res['holes'] if label(d['body']) == h['body'] and same_axis(h['p'], h['a'], d['p'], d['a']) and abs(d['d'] - h['d']) <= 0.05]
            if hit:
                rep['found'] += 1
            else:
                rep['missed'].append({'asm': a['id'], 'type': h['type'], 'd': round(h['d'], 3)})
        # labelled sites: designed holes of two different bodies on one axis with the same diameter
        pairs = set()
        for i, h1 in enumerate(labelled):
            for h2 in labelled[i + 1:]:
                if h1['body'] != h2['body'] and abs(h1['d'] - h2['d']) <= 0.01 * h1['d'] and same_axis(h1['p'], h1['a'], h2['p'], h2['a']):
                    pairs.add((min(h1['body'], h2['body']), max(h1['body'], h2['body']), round(h1['p'][0], 1), round(h1['p'][1], 1), round(h1['p'][2], 1)))
        rep['site_pairs'] += len(pairs)
        for p in pairs:
            b1, b2 = p[0], p[1]
            hh = next(h for h in labelled if h['body'] == b1 and abs(round(h['p'][0], 1) - p[2]) < 0.2)
            found = [s for s in res['sites'] if {b1, b2} <= {label(x) for x in s['bodies']} and same_axis(hh['p'], hh['a'], s['p'], s['a'])]
            rep['site_pairs_found'] += bool(found)
        for s in res['sites']:
            if s['bridged_by']:
                rep['bridged'] += 1
                if any(same_axis(h['p'], h['a'], s['p'], s['a']) for h in labelled):
                    rep['bridged_on_label'] += 1
        for n in res['near']:
            on_label = any(same_axis(h['p'], h['a'], n['p'], n['a']) and abs(h['d'] - n['d']) <= 0.05 for h in labelled)
            rep['near'].append(dict(n, asm=a['id'], on_label=on_label))
    out = sys.argv[3] if len(sys.argv) > 3 else work + '/report.json'
    json.dump(rep, open(out, 'w'), indent=1)
    print(f"assemblies scored {rep['assemblies']} (failed {len(rep['failed'])}, unread parts {rep['unread_parts']}), holes {rep['holes']}, sites {rep['sites']}")
    print(f"designed holes {rep['labelled']}, found {rep['found']} ({100 * rep['found'] / max(1, rep['labelled']):.1f} %)")
    print(f"designed coaxial pairs on different bodies {rep['site_pairs']}, found as a site {rep['site_pairs_found']}")
    print(f"bridged sites {rep['bridged']}, on a designed hole axis {rep['bridged_on_label']}")
    edges = [0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.99, 1.0]
    for lo, hi in zip(edges, edges[1:]):
        b = [n for n in rep['near'] if lo <= n['covered'] < hi]
        print(f"partial bores covering [{lo}, {hi}): {len(b)}, on a designed hole {sum(n['on_label'] for n in b)}")
    print('max detect seconds', max(rep['times']) if rep['times'] else None)


if __name__ == '__main__':
    main()
