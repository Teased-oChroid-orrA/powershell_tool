#!/usr/bin/env python3
"""Fetch a small random sample of AutoMate assemblies and their part STEP files by HTTP byte ranges (no 35 GB download).

AutoMate (Jones et al., ACM TOG 40(6), 2021; Zenodo 7776208, DOI 10.5061/dryad.2547d7wvw, CC0 1.0): public Onshape assemblies with
user-authored mates. Reads only the two zip directories (45.9 MB + 84.3 MB), the chosen assembly JSON files and their part STEP files.
Usage: fetch.py DATA_DIR [N_CANDIDATES=400] [SEED=7]. Nothing here is committed to the repository.
"""
import json, os, random, struct, sys, time, urllib.request, zlib
from concurrent.futures import ThreadPoolExecutor

REC = 'https://zenodo.org/api/records/7776208/files/%s/content'
ASM, STEP = 'assemblies.zip', 'step.zip'
SIZES = {ASM: 911443736, STEP: 13211245622}


def rng(url, a, b, tries=4):
    for k in range(tries):
        try:
            return urllib.request.urlopen(urllib.request.Request(url, headers={'Range': f'bytes={a}-{b}'}), timeout=120).read()
        except Exception:
            if k == tries - 1:
                raise
            time.sleep(2 * (k + 1))


def directory(name, cache):
    if os.path.exists(cache):
        return json.load(open(cache))
    url, size = REC % name, SIZES[name]
    tail = rng(url, size - 70000, size - 1)
    j = tail.rfind(b'PK\x06\x07')
    _, _, off64, _ = struct.unpack('<IIQI', tail[j:j + 20])
    _, _, _, _, _, _, _, n, cd_size, cd_off = struct.unpack('<IQHHIIQQQQ', rng(url, off64, off64 + 55))
    cd = rng(url, cd_off, cd_off + cd_size - 1)
    out, p = {}, 0
    for _ in range(n):
        (_, _, _, _, method, _, _, _, csize, usize, nlen, elen, clen, _, _, _, lho) = struct.unpack('<IHHHHHHIIIHHHHHII', cd[p:p + 46])
        entry_name = cd[p + 46:p + 46 + nlen].decode('utf8', 'replace')
        extra, q = cd[p + 46 + nlen:p + 46 + nlen + elen], 0
        while q + 4 <= len(extra):
            hid, hl = struct.unpack('<HH', extra[q:q + 4])
            if hid == 1:
                f, k = extra[q + 4:q + 4 + hl], 0
                if usize == 0xFFFFFFFF: usize = struct.unpack('<Q', f[k:k + 8])[0]; k += 8
                if csize == 0xFFFFFFFF: csize = struct.unpack('<Q', f[k:k + 8])[0]; k += 8
                if lho == 0xFFFFFFFF: lho = struct.unpack('<Q', f[k:k + 8])[0]; k += 8
            q += 4 + hl
        out[entry_name] = [method, csize, usize, lho]
        p += 46 + nlen + elen + clen
    json.dump(out, open(cache, 'w'))
    return out


def member(name, ent):
    method, csize, _, lho = ent
    url = REC % name
    nlen, elen = struct.unpack('<HH', rng(url, lho, lho + 29)[26:30])
    data = rng(url, lho + 30 + nlen + elen, lho + 30 + nlen + elen + csize - 1) if csize else b''
    return data if method == 0 else zlib.decompress(data, -15)


def main():
    data = sys.argv[1]
    n_cand = int(sys.argv[2]) if len(sys.argv) > 2 else 400
    seed = int(sys.argv[3]) if len(sys.argv) > 3 else 7
    os.makedirs(data + '/parts', exist_ok=True)
    adir = directory(ASM, data + '/assemblies.dir.json')
    cands = [k for k, v in adir.items() if k.endswith('.json') and 5000 <= v[2] <= 40000]
    random.Random(seed).shuffle(cands)

    def get(k):
        try:
            return k, json.loads(member(ASM, adir[k]))
        except Exception:
            return k, None

    keep = []
    with ThreadPoolExecutor(4) as ex:
        for k, j in ex.map(get, cands[:n_cand]):
            if j and j.get('has_all_step') and 2 <= len(j['parts']) <= 10 and j['mates']:
                keep.append((k, j))
    json.dump(keep, open(data + '/asm_sample.json', 'w'))
    sdir = directory(STEP, data + '/step.dir.json')
    parts = sorted({p['id'] for _, j in keep for p in j['parts']})

    def fetch(p):
        path = f'{data}/parts/{p}.step'
        if not os.path.exists(path):
            try:
                open(path, 'wb').write(member(STEP, sdir[f'step/{p}.step']))
                time.sleep(0.2)
            except Exception as e:
                return p, repr(e)[:80]
        return p, None

    with ThreadPoolExecutor(2) as ex:
        failed = [(p, e) for p, e in ex.map(fetch, parts) if e]
    print(f'assemblies {len(keep)}, parts {len(parts)}, failed {len(failed)}')
    for p, e in failed:
        print('  failed', p, e)


if __name__ == '__main__':
    main()
