#!/usr/bin/python3
"""How fast the models answer on a CPU, with and without speculative decoding. Run by speed.yml.

  speed-bench.py --server /path/llama-server --models DIR --out results.json

Each setup starts llama-server the way the router starts it on a machine with no GPU (-ngl 0 -dev none,
the pack's own sampling), asks it the same three kinds of thing, and reads the server's own timings:
generation tokens a second, and for a speculative setup how many drafted tokens were accepted.

  write   new code from a sentence: what a make starts with
  edit    a whole file handed back with one change: what the maker does most, and what an n-gram
          drafter should be best at, because nearly every token is already in the prompt
  answer  a plain-words explanation: what chat and the crash and update notes do

Sampling is the router's, with a fixed seed, because that is what a person gets. One greedy pair on
the edit checks the other thing that matters: speculation must not change what the model says.
"""
import argparse, json, os, pathlib, signal, subprocess, sys, time, urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
EDIT_SOURCE = (ROOT / "system_files/usr/bin/genesis-power").read_text()

TASKS = {
    "write": ("Write a Python command-line program that keeps a to-do list in a JSON file next to it, with "
              "`add TEXT`, `done N` and `list`, using argparse, followed by a unittest file for it. Code only.", 500),
    "edit": ("Here is a file. Rename the function `source` to `power_source` everywhere it is defined or "
             "called, change nothing else, and return the complete file with no commentary.\n\n```python\n"
             + EDIT_SOURCE + "\n```", 600),
    "answer": ("Explain in plain words, for someone who is not technical, what a swap file is, why a laptop "
               "with 8 GB of memory might slow down when it is used, and what they can do about it.", 400),
}

# the router's settings per role (packs.rs render_router_tuned), minus the port and the model path
ROLE_ARGS = {
    "code": "-c 8192 --temp 0.6 --top-p 0.95 --top-k 20 --min-p 0.0 --reasoning off --cache-type-k q8_0 --cache-type-v q8_0",
    "fast": "-c 16384 --temp 0.7 --top-p 0.8 --top-k 20 --reasoning off",
}


def ask(port, prompt, max_tokens, temperature=None):
    body = {"messages": [{"role": "user", "content": prompt}], "max_tokens": max_tokens, "seed": 42, "cache_prompt": False}
    if temperature is not None:
        body["temperature"] = temperature
    req = urllib.request.Request(f"http://127.0.0.1:{port}/v1/chat/completions", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=3600) as r:
        d = json.load(r)
    t = d.get("timings", {})
    return {
        "gen_tps": round(t.get("predicted_per_second", 0), 2),
        "prompt_tps": round(t.get("prompt_per_second", 0), 1),
        "gen_tokens": t.get("predicted_n", 0),
        "drafted": t.get("draft_n", 0),
        "accepted": t.get("draft_n_accepted", 0),
        "wall_s": round(time.time() - t0, 1),
        "text": d["choices"][0]["message"].get("content", ""),
    }


def start(server, args, port, log):
    cmd = [server, "--port", str(port), "--host", "127.0.0.1", "-ngl", "0", "-dev", "none", "-t", str(os.cpu_count() or 4),
           "--jinja", "--flash-attn", "auto"] + args.split()
    print("  $", " ".join(cmd[1:]), flush=True)
    p = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
    for _ in range(600):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=2) as r:
                if r.status == 200:
                    return p
        except Exception:
            pass
        if p.poll() is not None:
            raise RuntimeError(f"llama-server exited ({p.returncode}); see the log")
        time.sleep(1)
    p.kill()
    raise RuntimeError("llama-server did not become healthy in ten minutes")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", required=True)
    ap.add_argument("--models", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    m = pathlib.Path(a.models)
    code, fast = m / "Qwen3.5-9B-Q4_K_M.gguf", m / "Qwen3.5-4B-Q4_K_M.gguf"
    d08, d2 = m / "Qwen3.5-0.8B-Q4_K_M.gguf", m / "Qwen3.5-2B-Q4_K_M.gguf"
    setups = [
        # (name, role, target, extra args, tasks)
        ("9B, as shipped", "code", code, "", ["write", "edit"]),
        ("9B + n-gram", "code", code, "--spec-type ngram-mod", ["write", "edit"]),
        ("9B + 0.8B draft, 8", "code", code, f"-md {d08} --draft-max 8 --draft-min 2", ["write", "edit"]),
        ("9B + 0.8B draft, 16", "code", code, f"-md {d08} --draft-max 16 --draft-min 4", ["write", "edit"]),
        ("9B + 0.8B draft + n-gram", "code", code, f"--spec-type ngram-mod,draft-simple -md {d08} --draft-max 8 --draft-min 2", ["write", "edit"]),
        ("9B + 2B draft, 8", "code", code, f"-md {d2} --draft-max 8 --draft-min 2", ["write", "edit"]),
        ("4B, as shipped", "fast", fast, "", ["answer", "edit"]),
        ("4B + n-gram", "fast", fast, "--spec-type ngram-mod", ["answer", "edit"]),
        ("4B + 0.8B draft, 8", "fast", fast, f"-md {d08} --draft-max 8 --draft-min 2", ["answer", "edit"]),
    ]
    results, port = [], 18200
    logdir = pathlib.Path(a.out).with_suffix(".logs"); logdir.mkdir(exist_ok=True)
    for name, role, target, extra, tasks in setups:
        port += 1
        print(f"== {name}", flush=True)
        with open(logdir / f"{port}.log", "w") as log:
            try:
                p = start(a.server, f"-m {target} {ROLE_ARGS[role]} {extra}", port, log)
            except RuntimeError as e:
                print("  could not start:", e, flush=True)
                results.append({"setup": name, "error": str(e)}); continue
            try:
                for task in tasks:
                    prompt, n = TASKS[task]
                    r = ask(port, prompt, n)
                    r.update(setup=name, role=role, task=task)
                    print(f"  {task:6} {r['gen_tps']:6.2f} tok/s  ({r['gen_tokens']} tokens, drafted {r['drafted']}, accepted {r['accepted']}, {r['wall_s']} s)", flush=True)
                    results.append(r)
                # speculation must not change the answer: the edit, greedy, once per setup of the coder
                if role == "code" and ("n-gram" in name or "shipped" in name or name.endswith("draft, 8") and "0.8B" in name):
                    g = ask(port, TASKS["edit"][0], 300, temperature=0)
                    results.append({"setup": name, "role": role, "task": "edit-greedy", "text": g["text"], "gen_tps": g["gen_tps"]})
            finally:
                p.send_signal(signal.SIGTERM)
                try:
                    p.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    p.kill()
    pathlib.Path(a.out).write_text(json.dumps(results, indent=1))
    # the table, for the job summary
    base = {(r["role"], r["task"]): r["gen_tps"] for r in results if r.get("setup", "").endswith("as shipped") and "gen_tps" in r}
    lines = ["| setup | task | tokens/s | vs as shipped | drafted | accepted |", "|---|---|---|---|---|---|"]
    for r in results:
        if "error" in r:
            lines.append(f"| {r['setup']} | — | could not start | | | |"); continue
        if r["task"] == "edit-greedy":
            continue
        b = base.get((r["role"], r["task"]))
        rate = f"{r['accepted'] / r['drafted']:.0%}" if r["drafted"] else ""
        lines.append(f"| {r['setup']} | {r['task']} | {r['gen_tps']:.2f} | {r['gen_tps'] / b:.2f}x | {r['drafted'] or ''} | {rate} |" if b else
                     f"| {r['setup']} | {r['task']} | {r['gen_tps']:.2f} | | | |")
    greedy = {r["setup"]: r["text"] for r in results if r.get("task") == "edit-greedy"}
    ref = greedy.get("9B, as shipped")
    if ref is not None:
        lines.append("")
        for s, t in greedy.items():
            if s != "9B, as shipped":
                same = t == ref
                lines.append(f"- greedy edit, {s}: {'the same answer as without speculation' if same else 'DIFFERENT from the answer without speculation (first difference at character %d)' % next((i for i, (x, y) in enumerate(zip(t, ref)) if x != y), min(len(t), len(ref)))}")
    table = "\n".join(lines)
    print(table)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write(f"### Speed on this runner's CPU ({os.cpu_count()} threads)\n\n" + table + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
