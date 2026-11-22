#!/usr/bin/env python3
"""Place the bodies of every extracted Fusion 360 Gallery assembly in the root frame: one STEP file per body instance (every
CARTESIAN_POINT and DIRECTION rewritten by the composed occurrence transform, centimetres in and out, as the files are) plus a manifest
with the assembly's designed Hole features (`holes` in assembly.json: type, diameter, origin, direction; centimetres).
Usage: prepare.py DATA_DIR OUT_DIR [MAX_INSTANCES=80]. DATA_DIR holds one directory per assembly (assembly.json and one STEP per body)."""
import json, os, re, sys

NUM = r'[-+]?(?:\d+\.?\d*|\.\d+)(?:[Ee][-+]?\d+)?'
PT = re.compile(r"(CARTESIAN_POINT\('[^']*',\()(" + NUM + r'),(' + NUM + r'),(' + NUM + r')(\)\))')
DIR = re.compile(r"(DIRECTION\('[^']*',\()(" + NUM + r'),(' + NUM + r'),(' + NUM + r')(\)\))')
IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]


def mul(m, v, w):
    return [m[4 * i] * v[0] + m[4 * i + 1] * v[1] + m[4 * i + 2] * v[2] + m[4 * i + 3] * w for i in range(3)]


def transform(text, m):
    def point(g):
        p = mul(m, [float(g[1]), float(g[2]), float(g[3])], 1.0)
        return f'{g[0]}{p[0]!r},{p[1]!r},{p[2]!r}{g[4]}'

    def direction(g):
        d = mul(m, [float(g[1]), float(g[2]), float(g[3])], 0.0)
        return f'{g[0]}{d[0]!r},{d[1]!r},{d[2]!r}{g[4]}'

    return DIR.sub(lambda mo: direction(mo.groups()), PT.sub(lambda mo: point(mo.groups()), text))


def matmul(a, b):
    return [sum(a[4 * i + k] * b[4 * k + j] for k in range(4)) for i in range(4) for j in range(4)]


def matrix(t):
    """A Fusion transform (origin and three axes) as a row-major 4x4."""
    o, x, y, z = (t[k] for k in ('origin', 'x_axis', 'y_axis', 'z_axis'))
    return [x['x'], y['x'], z['x'], o['x'], x['y'], y['y'], z['y'], o['y'], x['z'], y['z'], z['z'], o['z'], 0, 0, 0, 1]


def instances(a):
    """(body id, composed transform) of every body instance: the root component's bodies, then each occurrence's component's."""
    out = []
    comps = a['components']
    for b in comps.get(a['root']['component'], {}).get('bodies', []):
        out.append((b, IDENTITY))

    def walk(tree, parent):
        for oid, sub in (tree or {}).items():
            o = a['occurrences'].get(oid)
            if o is None:
                continue
            m = matmul(parent, matrix(o['transform']))
            for b in comps.get(o['component'], {}).get('bodies', []):
                out.append((b, m))
            walk(sub, m)

    walk(a['tree'].get('root'), IDENTITY)
    return out


def main():
    data, out = sys.argv[1], sys.argv[2]
    cap = int(sys.argv[3]) if len(sys.argv) > 3 else 80
    os.makedirs(out, exist_ok=True)
    manifest, skipped = [], {'no json': 0, 'no bodies': 0, 'too many': 0, 'missing step': 0}
    for name in sorted(os.listdir(data)):
        try:
            a = json.load(open(f'{data}/{name}/assembly.json'))
            inst = instances(a)
        except Exception:
            skipped['no json'] += 1
            continue
        if len(inst) < 2:
            skipped['no bodies'] += 1
            continue
        if len(inst) > cap:
            skipped['too many'] += 1
            continue
        files = [f'{data}/{name}/{b}.step' for b, _ in inst]
        if not all(os.path.exists(f) for f in files):
            skipped['missing step'] += 1
            continue
        d = f'{out}/{name}'
        os.makedirs(d, exist_ok=True)
        placed, cache = [], {}
        for i, ((b, m), f) in enumerate(zip(inst, files)):
            cache.setdefault(f, open(f, errors='replace').read())
            path = f'{d}/inst_{i:03d}.step'
            open(path, 'w').write(transform(cache[f], m))
            placed.append(path)
        holes = [{'type': h['type'], 'd': h['diameter'] * 10, 'p': [h['origin'][k] * 10 for k in 'xyz'], 'a': [h['direction'][k] for k in 'xyz'], 'len': (h.get('length') or 0) * 10, 'body': h['body']}
                 for h in (a.get('holes') or []) if h.get('diameter') and h.get('origin') and h.get('direction')]
        manifest.append({'id': name, 'files': placed, 'bodies': [b for b, _ in inst], 'holes': holes})
    json.dump(manifest, open(out + '/manifest.json', 'w'))
    print(f'prepared {len(manifest)} assemblies; skipped {skipped}')


if __name__ == '__main__':
    main()
