"""Decompile every function in the current Ghidra program into re/decomp/<addr>.c (skips existing)."""
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
from gh import call  # noqa: E402

ROOT = os.path.join(os.path.dirname(__file__), "..", "re")
fs = json.load(open(os.path.join(ROOT, "functions.json")))
out = os.path.join(ROOT, "decomp")
os.makedirs(out, exist_ok=True)
todo = [f["address"] for f in fs if not f.get("isExternal") and not f.get("isThunk")
        and not os.path.exists(os.path.join(out, f["address"] + ".c"))]
print(f"{len(todo)} to decompile", flush=True)
t0 = time.time()
for i, a in enumerate(todo):
    try:
        r = call("decompile_function", {"address": a, "timeout": "60"}, timeout=120)
        txt = r if isinstance(r, str) else r.get("decompiled", json.dumps(r))
    except Exception as e:  # keep going; record failure
        txt = f"// DECOMPILE FAILED: {e}\n"
    with open(os.path.join(out, a + ".c"), "w", encoding="utf-8") as fh:
        fh.write(txt.replace("\r", ""))
    if i % 500 == 0:
        print(f"{i}/{len(todo)} {time.time() - t0:.0f}s", flush=True)
print("DONE", flush=True)
