#!/usr/bin/python3
"""eval-merge: join the results.json files the shards produced into one result and one table.

  eval-merge.py shard-*/results.json --out results.json

The makes are put back into the order eval-makes.py declares them, so two runs read the same way no
matter how the shards were split or which finished first.
"""
import json, sys

ORDER = ["checklist", "pomodoro", "wordcount", "rename", "club", "json-flag", "temperature", "todo-cli", "quotes", "dice"]


def main(argv):
    out = "results.json"
    if "--out" in argv:
        out = argv[argv.index("--out") + 1]; argv = [a for a in argv if a != out and a != "--out"]
    rows, model, at = [], None, None
    for f in argv:
        try:
            d = json.load(open(f))
        except (OSError, ValueError) as e:
            print(f"skipping {f}: {e}", file=sys.stderr); continue
        rows += d.get("makes", [])
        model = model or d.get("model"); at = at or d.get("at")
    rows.sort(key=lambda r: ORDER.index(r["name"]) if r["name"] in ORDER else 99)
    passed = sum(1 for r in rows if r.get("passed"))
    finished = sum(1 for r in rows if r.get("finished"))
    json.dump({"model": model, "at": at, "passed": passed, "total": len(rows), "makes": rows}, open(out, "w"), indent=1)
    print(f"## Ten makes on `{model}`: **{passed}/{len(rows)} passed** ({finished} finished, {passed} of those look right)\n")
    print("| make | result | files written | turns | tool errors | preview | time | memory left |")
    print("|---|---|---|---|---|---|---|---|")
    for r in rows:
        mem = f"{r.get('mem_available_mb', 0)} MB" + ("" if r.get("router_alive", True) else " · model service gone")
        print(f"| {r['name']} | {'pass' if r.get('passed') else r['state']} | {r.get('writes', 0)} | {r.get('turns', 0)} | {r.get('tool_errors', 0)} | {'yes' if r.get('preview') else 'no'} | {r.get('seconds', 0)}s | {mem} |")
    print("\nWhat was holding memory at the end of each make (largest resident sets):\n")
    for r in rows:
        print(f"- **{r['name']}** — {r.get('mem_available_mb', 0)} MB free, {r.get('proc_count', 0)} processes, {r.get('rss_total_mb', 0)} MB resident in all: " + "; ".join(r.get("top_rss", [])[:4]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
