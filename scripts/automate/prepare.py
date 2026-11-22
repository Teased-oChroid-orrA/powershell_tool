#!/usr/bin/env python3
"""Place the parts of every fetched AutoMate assembly in the root frame: one STEP file per occurrence (every CARTESIAN_POINT and
DIRECTION of the part rewritten by the occurrence's rigid transform, metres in and out) plus a manifest with each mate's world frame.
Usage: prepare.py DATA_DIR OUT_DIR. Assemblies with a missing part file are skipped."""
import json, os, re, sys

NUM = r'[-+]?(?:\d+\.?\d*|\.\d+)(?:[Ee][-+]?\d+)?'
PT = re.compile(r"(CARTESIAN_POINT\('[^']*',\()(" + NUM + r'),(' + NUM + r'),(' + NUM + r')(\)\))')
DIR = re.compile(r"(DIRECTION\('[^']*',\()(" + NUM + r'),(' + NUM + r'),(' + NUM + r')(\)\))')


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


def main():
    data, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    manifest = []
    for n, (key, j) in enumerate(json.load(open(data + '/asm_sample.json'))):
        files = [f"{data}/parts/{p['id']}.step" for p in j['parts']]
        if not all(os.path.exists(f) for f in files):
            continue
        d = f'{out}/a{n:03d}'
        os.makedirs(d, exist_ok=True)
        occ_files, text_cache = [], {}
        for i, o in enumerate(j['occurrences']):
            f = files[o['part']]
            text_cache.setdefault(f, open(f, errors='replace').read())
            path = f'{d}/occ_{i:02d}.step'
            open(path, 'w').write(transform(text_cache[f], o['transform']))
            occ_files.append(path)
        mates = []
        for m in j['mates']:
            frames = []
            for oi, mcf in zip(m['occurrences'], m['mcfs']):
                w = matmul(j['occurrences'][oi]['transform'], mcf)
                frames.append({'p': [w[3] * 1000, w[7] * 1000, w[11] * 1000], 'z': [w[2], w[6], w[10]]})
            mates.append({'type': m['mateType'], 'occ': m['occurrences'], 'frames': frames})
        manifest.append({'id': j['assemblyId'], 'dir': d, 'files': occ_files, 'mates': mates, 'parts': [o['part'] for o in j['occurrences']]})
    json.dump(manifest, open(out + '/manifest.json', 'w'))
    print(f'prepared {len(manifest)} assemblies')


if __name__ == '__main__':
    main()
