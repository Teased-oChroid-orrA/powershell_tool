"""Parse + judge bake-off for local GGUF models on CPU (llama-cpp-python, n_gpu_layers=0).
usage: python llm_bakeoff.py model.gguf out.json [threads]
Data: search-ml/examples/data/condition_{parse,judge}_cases.json"""
import json, re, resource, sys, time
from llama_cpp import Llama

DATA = "/Users/nautilus/Claude/Projects/powerShell/powershell_tool/search-ml/examples/data/"
model, out = sys.argv[1], sys.argv[2]
threads = int(sys.argv[3]) if len(sys.argv) > 3 else 4
nothink = "/no_think\n" if "Qwen3-1.7B" in model else ""

t0 = time.time()
llm = Llama(model_path=model, n_ctx=2048, n_threads=threads, n_gpu_layers=0, verbose=False)
load_s = time.time() - t0

PARSE_SCHEMA = {"type": "object", "required": ["conditions"], "properties": {"conditions": {
    "type": "array", "minItems": 1, "maxItems": 6, "items": {"type": "object",
    "required": ["text", "terms", "negated"], "properties": {
        "text": {"type": "string"}, "terms": {"type": "array", "items": {"type": "string"}, "maxItems": 12},
        "negated": {"type": "boolean"}}}}}}
JUDGE_SCHEMA = {"type": "object", "required": ["quote", "verdict"], "properties": {
    "quote": {"type": "string"}, "verdict": {"type": "string", "enum": ["yes", "no"]}}}

PARSE_SYS = nothink + """Split the user's search sentence into the independent conditions a document must satisfy.
For each condition give: text (short phrase), terms (lowercase key words plus word-form variants, keep numbers and codes), negated (true only if the document must NOT satisfy it).
Never add a condition that is not in the sentence. Example:
Sentence: files where the skin was cracked and repaired but not painted
{"conditions":[{"text":"skin cracked","terms":["skin","crack","cracked"],"negated":false},{"text":"repaired","terms":["repair","repaired"],"negated":false},{"text":"painted","terms":["paint","painted"],"negated":true}]}"""
JUDGE_SYS = nothink + """Decide whether the passage itself asserts the condition. First copy the exact words from the passage that state it (empty string if none). Then answer yes only if the passage affirms the condition for the same item: a negation, a requirement that was not met, a different item, or a mere mention of related words is no."""

def chat(system, user, schema, max_tokens):
    t = time.time()
    r = llm.create_chat_completion(messages=[{"role": "system", "content": system}, {"role": "user", "content": user}],
        response_format={"type": "json_object", "schema": schema}, temperature=0, max_tokens=max_tokens)
    return r["choices"][0]["message"]["content"], time.time() - t, r["usage"]["completion_tokens"], r["usage"]["prompt_tokens"]

def norm(s): return re.sub(r"\s+", " ", re.sub(r"[^a-z0-9. ]", " ", s.lower())).strip()

import os
LIM = int(os.environ.get("LIM", "999"))
parse = json.load(open(DATA + "condition_parse_cases.json"))[:LIM]
valid = rec = neg_ok = cnt_exact = over = gold_total = neg_total = 0
ptime = ptok = pgen = 0
perr = []
for c in parse:
    txt, dt, gen, pt = chat(PARSE_SYS, "Sentence: " + c["q"], PARSE_SCHEMA, 400)
    ptime += dt; pgen += gen; ptok += pt
    try:
        conds = json.loads(txt)["conditions"]; valid += 1
    except Exception:
        perr.append(c["q"]); gold_total += len(c["gold"]); continue
    blobs = [(norm(" ".join([x["text"]] + x["terms"])), bool(x["negated"])) for x in conds]
    cnt_exact += len(conds) == len(c["gold"]); over += max(0, len(conds) - len(c["gold"]))
    for g in c["gold"]:
        gold_total += 1
        same = [b for b, n in blobs if n == g["negated"]]
        union = " ".join(same)
        if all(s in union for s in g["stems"]): rec += 1
        else: perr.append((c["q"], g["stems"]))
        if g["negated"]:
            neg_total += 1
            if any(n and all(s in b for s in g["stems"]) for b, n in blobs): neg_ok += 1
    # spurious negation
negfp = 0

judge = json.load(open(DATA + "condition_judge_cases.json"))[:LIM]
tp = fp = tn = fn = qvalid = yes_n = 0
jtime = jgen = 0
jerr = []
for c in judge:
    txt, dt, gen, pt = chat(JUDGE_SYS, f"Condition: {c['cond']}\nPassage: {c['passage']}", JUDGE_SCHEMA, 120)
    jtime += dt; jgen += gen
    try:
        j = json.loads(txt)
    except Exception:
        j = {"quote": "", "verdict": "no"}
    quote_ok = bool(j["quote"]) and norm(j["quote"]) in norm(c["passage"])
    said_yes = j["verdict"] == "yes"
    verdict = said_yes and quote_ok
    if said_yes:
        yes_n += 1
        qvalid += quote_ok
    if c["gold"] and verdict: tp += 1
    elif c["gold"]: fn += 1; jerr.append(("FN", c["cond"], c["passage"], j))
    elif verdict: fp += 1; jerr.append(("FP", c["cond"], c["passage"], j))
    else: tn += 1

res = {
 "model": model.split("/")[-1], "threads": threads, "load_s": round(load_s, 1),
 "peak_rss_mb": round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1e6) if sys.platform == "darwin" else round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1e3),
 "parse": {"n": len(parse), "valid_json": valid, "gold_conditions": gold_total, "condition_recall": rec,
           "count_exact": cnt_exact, "extra_conditions": over, "negation_found": f"{neg_ok}/{neg_total}",
           "sec_per_parse": round(ptime / len(parse), 2), "gen_tok_per_s": round(pgen / ptime, 1)},
 "judge": {"n": len(judge), "tp": tp, "fn": fn, "fp": fp, "tn": tn, "accuracy": round((tp + tn) / len(judge), 3),
           "yes_quotes_valid": f"{qvalid}/{yes_n}", "sec_per_judge": round(jtime / len(judge), 2)},
 "parse_misses": perr[:12], "judge_errors": jerr,
}
json.dump(res, open(out, "w"), indent=1)
print(json.dumps({k: v for k, v in res.items() if k not in ("parse_misses", "judge_errors")}, indent=1))
