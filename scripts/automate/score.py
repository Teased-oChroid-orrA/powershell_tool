#!/usr/bin/env python3
"""Score the detector against the user-authored mates of prepared AutoMate assemblies (see README.md).
Usage: score.py WORK_DIR PATH/TO/assembly-score [report.json]

Labels come only from the mates (mate type, mated occurrences, world mate frame: origin and z axis), never from the detector.
A weaker label, "a mate axis lies on a concave cylinder of one body and a convex one of the other = a pin in a hole", was tried and
dropped: about half of those pairs have a boss wider than the bore (standoffs, collars), so it does not discriminate.
 * bolted joints: one body C (the fastener) mated on one axis line to two other bodies A and B, each of which has a concave cylinder
   on that line (a bore), and C carries a convex cylinder on the line that fits the bores (no wider than the narrower one, within 0.05
   mm) and overlaps both bores along the axis (a real shank through both). A fastener site through A and B is expected. Mates whose
   frame merely lies on two bores' axis without a shank (a mate made on a circle's centre, a sleeve) are not counted as bolted.
"""
import json, math, subprocess, sys

ANG = math.cos(math.radians(1.0))
DIST = 0.01  # mm: a mate frame sits on the axis of the circle it was made on, so this is generous for float output


def dot(a, b): return sum(x * y for x, y in zip(a, b))
def sub(a, b): return [x - y for x, y in zip(a, b)]


def on_axis(p, z, cp, ca):
    """Does the mate axis (point p, direction z) coincide with the cylinder axis (cp, ca)?"""
    if abs(dot(z, ca)) < ANG:
        return False
    d = sub(p, cp)
    t = dot(d, ca)
    perp = [d[i] - t * ca[i] for i in range(3)]
    return math.sqrt(dot(perp, perp)) <= DIST


def same_line(p1, z1, p2, z2):
    return abs(dot(z1, z2)) >= ANG and on_axis(p1, z1, p2, z2)


def near_summary(near):
    """Partial bores (half a turn or more) by the turn they cover: where the near-closed threshold falls."""
    edges = [0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.99, 1.0]
    bins = [sum(1 for n in near if lo <= n['covered'] < hi) for lo, hi in zip(edges, edges[1:])]
    cand = [n for n in near if n['candidate']]
    return 'partial bores by covered turn ' + ', '.join(f'[{lo},{hi}): {b}' for (lo, hi), b in zip(zip(edges, edges[1:]), bins)) + f'; near-closed candidates {len(cand)}'


def main():
    work, exe = sys.argv[1], sys.argv[2]
    report = {'assemblies': 0, 'unread_parts': 0, 'bolted': [], 'times': [], 'hole_total': 0, 'site_total': 0, 'near': []}
    for a in json.load(open(work + '/manifest.json')):
        r = subprocess.run([exe] + a['files'], capture_output=True, text=True)
        try:
            res = json.loads(r.stdout)
        except Exception:
            report.setdefault('failed', []).append(a['id'])
            continue
        if 'error' in res:
            report.setdefault('failed', []).append((a['id'], res['error']))
            continue
        report['assemblies'] += 1
        report['unread_parts'] += len(res['unread'])
        report['times'].append(res['seconds'])
        report['hole_total'] += len(res['holes'])
        report['site_total'] += len(res['sites'])
        report['near'] += [dict(n, asm=a['id']) for n in res.get('near', [])]
        cyl = {}
        for c in res['cyl']:
            cyl.setdefault(c['body'], []).append(c)
        two_pi = 2 * math.pi - 1e-6
        # --- bolted joints: fastener body C mated on the same axis to bodies A and B that both have a bore on it
        mates = [m for m in a['mates'] if m['type'] in ('REVOLUTE', 'CYLINDRICAL', 'FASTENED')]
        seen = set()
        for i, m1 in enumerate(mates):
            for m2 in mates[i + 1:]:
                shared = set(m1['occ']) & set(m2['occ'])
                if len(shared) != 1:
                    continue
                c = shared.pop()
                ao = [o for o in m1['occ'] if o != c][0]
                bo = [o for o in m2['occ'] if o != c][0]
                if ao == bo:
                    continue
                f1 = m1['frames'][m1['occ'].index(ao)]
                f2 = m2['frames'][m2['occ'].index(bo)]
                if not same_line(f1['p'], f1['z'], f2['p'], f2['z']):
                    continue
                def bore(o, f):
                    return [x for x in cyl.get(o, []) if x['side'] == 'concave' and on_axis(f['p'], f['z'], x['p'], x['a'])]
                ba, bb = bore(ao, f1), bore(bo, f2)
                if not ba or not bb:
                    continue
                key = (a['id'], tuple(sorted((ao, bo))), c)
                if key in seen:
                    continue
                seen.add(key)
                z, p0 = f1['z'], f1['p']

                def interval(cs):
                    lo, hi = float('inf'), float('-inf')
                    for x in cs:
                        base, k = dot(sub(x['p'], p0), z), dot(x['a'], z)
                        e = [base + x['range'][0] * k, base + x['range'][1] * k]
                        lo, hi = min(lo, *e), max(hi, *e)
                    return lo, hi
                ia, ib = interval(ba), interval(bb)
                rb = min(x['r'] for x in ba + bb)
                pins = [x for x in cyl.get(c, []) if x['side'] == 'convex' and on_axis(p0, z, x['p'], x['a']) and x['r'] <= rb + 0.05]
                if not pins:
                    continue
                ip = interval(pins)
                if min(ip[1], ia[1]) - max(ip[0], ia[0]) < -0.05 or min(ip[1], ib[1]) - max(ip[0], ib[0]) < -0.05:
                    continue
                full = sum(x['wrap'] for x in ba) >= two_pi and sum(x['wrap'] for x in bb) >= two_pi
                found = [s for s in res['sites'] if {ao, bo} <= set(s['bodies']) and same_line(p0, z, s['p'], s['a'])]
                gap = max(ia[0], ib[0]) - min(ia[1], ib[1])
                report['bolted'].append({'asm': a['id'], 'bodies': sorted((ao, bo)), 'fastener': c, 'bores_full_turn': full, 'gap_mm': round(gap, 3),
                                         'site_found': bool(found), 'bridged_by': found[0].get('bridged_by') if found else None,
                                         'mismatch': found[0]['mismatch'] if found else None, 'size': found[0]['fastener'] if found else None})
    out = sys.argv[3] if len(sys.argv) > 3 else work + '/report.json'
    json.dump(report, open(out, 'w'), indent=1)
    print(f"assemblies scored {report['assemblies']} (unread parts {report['unread_parts']}), holes {report['hole_total']}, sites {report['site_total']}")
    bj = report['bolted']
    full = [b for b in bj if b['bores_full_turn']]
    print(f"labelled bolted joints (shank mated to two bored bodies): {len(bj)}; both bores a full turn: {len(full)}; site found among those: {sum(b['site_found'] for b in full)} (bridged across a gap: {sum(1 for b in full if b['bridged_by'])})")
    print(f"size table matched on found sites: {sum(1 for b in full if b['size'])}; diameter mismatch reported: {sum(1 for b in full if b['mismatch'])}")
    print(near_summary(report['near']))
    print('max detect seconds', max(report['times']) if report['times'] else None)


if __name__ == '__main__':
    main()
