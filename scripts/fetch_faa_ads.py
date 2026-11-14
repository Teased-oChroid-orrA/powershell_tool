#!/usr/bin/env python3
"""Download FAA Airworthiness Directive final rules from the Federal Register API (US federal works, public domain).

  python3 scripts/fetch_faa_ads.py <out-dir> [manufacturer-term] [count]

Writes one <document_number>.txt per rule plus manifest.tsv (number, date, title). Polite: one request at a time.
"""
import json, sys, time, urllib.parse, urllib.request
out = sys.argv[1]
term = sys.argv[2] if len(sys.argv) > 2 else "Airbus Helicopters"
want = int(sys.argv[3]) if len(sys.argv) > 3 else 400
import os
os.makedirs(out, exist_ok=True)
def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": "toolbench-eval"})
    return urllib.request.urlopen(req, timeout=60).read()
rows, page = [], 1
while len(rows) < want:
    q = [("per_page", "100"), ("page", str(page)), ("order", "newest"),
         ("conditions[agencies][]", "federal-aviation-administration"), ("conditions[type][]", "RULE"),
         ("conditions[term]", f'"Airworthiness Directives; {term}"')] + [("fields[]", f) for f in ("document_number", "title", "raw_text_url", "publication_date")]
    d = json.loads(get("https://www.federalregister.gov/api/v1/documents.json?" + urllib.parse.urlencode(q)))
    if not d["results"]: break
    rows += [r for r in d["results"] if term.lower() in r["title"].lower()]
    page += 1
    if page > d.get("total_pages", 1): break
    time.sleep(0.5)
rows = rows[:want]
with open(os.path.join(out, "manifest.tsv"), "w") as m:
    for r in rows:
        p = os.path.join(out, r["document_number"] + ".txt")
        if not os.path.exists(p):
            open(p, "wb").write(get(r["raw_text_url"])); time.sleep(0.3)
        m.write(f'{r["document_number"]}\t{r["publication_date"]}\t{r["title"]}\n')
print(len(rows), "rules in", out)
