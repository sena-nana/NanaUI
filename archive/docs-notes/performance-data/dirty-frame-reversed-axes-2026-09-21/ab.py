import json, subprocess, sys, os
S = os.path.dirname(os.path.abspath(__file__))
shapes = ["layout", "layout-rtl", "layout-reverse"]
positions = ["tail", "head"]
rows = [250, 500, 1000, 2000, 4000]
results = {}
def run(variant, shape, pos, n):
    out = f"{S}/ab-{variant}-{shape}-{pos}-{n}.json"
    subprocess.run([f"{S}/bench-{variant}", "--shape", shape, "--position", pos,
                    "--rows", str(n), "--dirty", "1", "--output", out],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    cell = json.load(open(out))["cells"][0]
    return cell
for round_ in range(2):
    order = ["before", "after"] if round_ == 0 else ["after", "before"]
    for shape in shapes:
        for pos in positions:
            for n in rows:
                for variant in order:
                    cell = run(variant, shape, pos, n)
                    key = (shape, pos, n, variant)
                    prev = results.get(key)
                    entry = {"min": cell["flush_ms"]["min"], "p50": cell["flush_ms"]["p50"],
                             "plan": cell["layout_plan"], "layout_nodes": cell["counters"]["layout_nodes"],
                             "nodes": cell["nodes"]}
                    if prev is None:
                        results[key] = entry
                    else:
                        prev["min"] = min(prev["min"], entry["min"])
                        prev["p50"] = min(prev["p50"], entry["p50"])
json.dump({"|".join(map(str, k)): v for k, v in results.items()}, open(f"{S}/ab-results.json", "w"), indent=1)
print("done")
