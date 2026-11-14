"""Slice ViDoRe Industrial (English shard) retrieval misses by query attributes and judged-page answer coverage.

Usage: python3 scripts/analyze_vidore_misses.py <benchmarks-folder>   (needs pyarrow; analysis only, not an app dependency)
Inputs: vidore-industrial/queries parquet, vidore-industrial-shard0/{queries.jsonl,corpus.jsonl,qrels/test.tsv}, and a
`retrieval_eval --rank-experiments` per-query file at vidore-industrial-analysis/perquery.tsv (variant `default`).
"answer coverage" = share of the reference answer's words (length > 3) found on the best judged page's text.
"""
import sys
import pyarrow.parquet as pq, json, re, collections, statistics as st
B=sys.argv[1].rstrip('/') + '/'
Q={str(r['query_id']):r for r in pq.read_table(B+'vidore-industrial/queries/test-00000-of-00001.parquet').to_pylist()}
eng=[json.loads(l)['_id'] for l in open(B+'vidore-industrial-shard0/queries.jsonl')]
corp={}
for l in open(B+'vidore-industrial-shard0/corpus.jsonl'):
    j=json.loads(l); corp[j['_id']]=j.get('text') or j.get('markdown') or ''
rel=collections.defaultdict(dict)
for l in list(open(B+'vidore-industrial-shard0/qrels/test.tsv'))[1:]:
    a,b,c=l.split(); rel[a][b]=int(c)
m={}
for l in open(B+'vidore-industrial-analysis/perquery.tsv'):
    f=l.split('\t')
    if f[4]=='default': m[f[2]]=(float(f[5]),float(f[6]))
tok=lambda s:set(w for w in re.findall(r"[a-z0-9]+",s.lower()) if len(w)>3)
rows=[]
for qid in eng:
    r=Q[qid]; n,rc=m[qid]
    ans=tok(r['answer']); 
    cov=max((len(ans&tok(corp.get(c,'')))/max(1,len(ans)) for c in rel[qid]),default=0)
    qt=tok(r['query']); qcov=max((len(qt&tok(corp.get(c,'')))/max(1,len(qt)) for c in rel[qid]),default=0)
    ct=list(r['content_type']); 
    rows.append(dict(id=qid,n=n,rc=rc,gen=r['query_generator'],ct='Text' if ct==['Text'] else ('Table' if 'Table' in ct else ('Image' if any(x in ct for x in('Image','Infographic','Figure','Chart')) else '+'.join(ct))),cov=cov,qcov=qcov,types=list(r['query_types'])))
def agg(name,sel):
    x=[r for r in rows if sel(r)]
    if not x: return
    print(f"{name:28} n={len(x):3} nDCG10={st.mean(r['n'] for r in x):.3f} R100={st.mean(r['rc'] for r in x):.3f} top10miss={sum(r['n']==0 for r in x)} R100miss={sum(r['rc']==0 for r in x)} ansCov={st.mean(r['cov'] for r in x):.2f}")
agg('all',lambda r:1)
for g in ('human','sdg'): agg('gen='+g,lambda r:r['gen']==g)
for c in sorted(set(r['ct'] for r in rows)): agg('content='+c,lambda r:r['ct']==c)
agg('text & human',lambda r:r['ct']=='Text' and r['gen']=='human')
for t in sorted(set(t for r in rows for t in r['types'])): agg('type='+t,lambda r:t in r['types'])
agg('miss@10 & ansCov>=0.8',lambda r:r['n']==0 and r['cov']>=0.8)
agg('miss@10 & ansCov<0.5',lambda r:r['n']==0 and r['cov']<0.5)
agg('miss@100',lambda r:r['rc']==0)
agg('hit@10',lambda r:r['n']>0)
