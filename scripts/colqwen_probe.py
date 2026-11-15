#!/usr/bin/env python3
"""Research probe (not shipped): ColQwen2 late-interaction page retrieval on the judged ViDoRe source PDFs.

Same queries and relevance as `search-core/examples/pdf_folder_eval.rs` (`folder_queries.jsonl`: a judged (document,
page) pair is relevant), restricted to the documents that hold judged pages, with the same metrics: document hit@1/5,
page hit@1/10, page nDCG@10 (binary, ideal = min(relevant, 10)) and page recall@100. Reports index size and time.

  python -I scripts/colqwen_probe.py <vidore-dir> <work-dir> [--max-side 896] [--limit-docs N]

Needs torch, transformers, colpali-engine, pypdfium2, peft, pillow and the Hugging Face models
vidore/colqwen2-base + vidore/colqwen2-v1.0 (cached under HF_HOME). Offline after the first download.
"""
import argparse
import json
import math
import os
import time
from pathlib import Path

import numpy as np
import pypdfium2 as pdfium
import torch
from colpali_engine.models import ColQwen2, ColQwen2Processor


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("vidore")
    ap.add_argument("work")
    ap.add_argument("--max-side", type=int, default=896)
    ap.add_argument("--limit-docs", type=int, default=0)
    ap.add_argument("--limit-pages", type=int, default=0, help="smoke test: only the first N pages of each document")
    ap.add_argument("--device", default="mps")
    a = ap.parse_args()
    vidore, work = Path(a.vidore), Path(a.work)
    (work / "emb").mkdir(parents=True, exist_ok=True)

    queries = [json.loads(l) for l in open(vidore / "folder_queries.jsonl")]
    judged = sorted({d for q in queries for d, _ in q["pages"]})
    if a.limit_docs:
        judged = judged[: a.limit_docs]
    present = set(judged)

    dtype = torch.bfloat16
    # The v1.0 repo is a LoRA adapter over colqwen2-base; from_pretrained resolves and merges the base itself.
    model = ColQwen2.from_pretrained("vidore/colqwen2-v1.0", torch_dtype=dtype, low_cpu_mem_usage=True).eval().to(a.device)
    proc = ColQwen2Processor.from_pretrained("vidore/colqwen2-base")

    # ---- index: one multi-vector embedding per page, no OCR, no text chunks ----
    pages = []  # (doc, page 1-based)
    t_index = time.time()
    vectors = {}
    for doc in judged:
        pdf = pdfium.PdfDocument(str(vidore / "pdfs" / f"{doc}.pdf"))
        for i in range(min(len(pdf), a.limit_pages or len(pdf))):
            key = (doc, i + 1)
            f = work / "emb" / f"{doc}_{i + 1}.npy"
            if f.exists():
                vectors[key] = np.load(f)
                pages.append(key)
                continue
            page = pdf[i]
            w, h = page.get_size()
            scale = a.max_side / max(w, h)
            img = page.render(scale=scale).to_pil().convert("RGB")
            with torch.no_grad():
                batch = proc.process_images([img]).to(model.device)
                emb = model(**batch)[0].to(torch.float32).cpu().numpy().astype(np.float16)
            np.save(f, emb)
            vectors[key] = emb
            pages.append(key)
            if len(pages) % 10 == 0:
                print(f"{len(pages)} pages, {time.time() - t_index:.0f}s", flush=True)
    index_seconds = time.time() - t_index
    index_bytes = sum(v.nbytes for v in vectors.values())
    print(f"indexed {len(pages)} pages in {index_seconds:.0f}s (cached pages are not re-timed), "
          f"{index_bytes / 1e6:.1f} MB of float16 vectors, {np.mean([v.shape[0] for v in vectors.values()]):.0f} vectors/page")

    # ---- queries ----
    rows = []
    for q in queries:
        rel = {(d, p) for d, p in q["pages"] if d in present}
        if rel:
            rows.append((q["text"], rel))
    t_q = time.time()
    with torch.no_grad():
        qemb = []
        for text, _ in rows:
            b = proc.process_queries([text]).to(model.device)
            qemb.append(model(**b)[0].to(torch.float32).cpu())
    query_embed_seconds = time.time() - t_q

    t_s = time.time()
    page_tensors = [torch.from_numpy(vectors[k].astype(np.float32)) for k in pages]
    n = hit1_doc = hit5_doc = ph1 = ph10 = 0.0
    ndcg = rec100 = 0.0
    for (text, rel), qe in zip(rows, qemb):
        scores = np.array([(qe @ pt.T).max(dim=1).values.sum().item() for pt in page_tensors])
        order = np.argsort(-scores)
        ranked = [pages[i] for i in order]
        rel_docs = {d for d, _ in rel}
        docs = []
        for d, _ in ranked:
            if d not in docs:
                docs.append(d)
        hit_at = lambda k: float(any(d in rel_docs for d in docs[:k]))
        gains = [1.0 if r in rel else 0.0 for r in ranked[:100]]
        dcg = sum(g / math.log2(i + 2) for i, g in enumerate(gains[:10]))
        ideal = sum(1 / math.log2(i + 2) for i in range(min(len(rel), 10)))
        n += 1
        hit1_doc += hit_at(1)
        hit5_doc += hit_at(5)
        ph1 += float(gains[0] > 0)
        ph10 += float(any(g > 0 for g in gains[:10]))
        ndcg += dcg / ideal
        rec100 += sum(gains) / len(rel)
    score_seconds = time.time() - t_s
    result = {
        "docs": len(judged), "pages": len(pages), "queries": int(n),
        "doc_hit@1": hit1_doc / n, "doc_hit@5": hit5_doc / n, "page_hit@1": ph1 / n, "page_hit@10": ph10 / n,
        "page_nDCG@10": ndcg / n, "page_recall@100": rec100 / n,
        "index_seconds": index_seconds, "index_MB_fp16": index_bytes / 1e6,
        "query_embed_seconds": query_embed_seconds, "score_seconds": score_seconds, "max_side": a.max_side,
    }
    print(json.dumps(result, indent=1))
    (work / "result.json").write_text(json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
