#!/usr/bin/python3
"""How much of each turn the model reads again. Run against a llama-server that is already up.

  prefill-bench.py --url http://127.0.0.1:18400 [--turns 6]

A maker job is a conversation that only grows: system prompt and tools, the task, then per turn the
model's reply (usually a tool call) and the tool's result. Every turn sends all of it. The server keeps
what it already read -- unless something in the part it read has changed, or, on the hybrid Qwen3.5 and
later models, its recurrent layers cannot be wound back to where the new text starts. The server's own
timings say which: cache_n tokens came from the cache, prompt_n were read again. On a laptop that reads 17
tokens a second, every token in prompt_n that was already there is time a person waits for nothing.
"""
import argparse, json, sys, time, urllib.request

SYSTEM = ("You are Genesis, the maker built into this computer. Build exactly what the user asks, step by step, "
          "using tools. Write complete files with write_file, run them with shell, fix what the run shows, then "
          "reply with a short summary. ") * 6
TOOLS = [
    {"type": "function", "function": {"name": "write_file", "description": "Write a whole file.", "parameters": {"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}, "required": ["path", "content"]}}},
    {"type": "function", "function": {"name": "read_file", "description": "Read a file.", "parameters": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}}},
    {"type": "function", "function": {"name": "shell", "description": "Run a shell command in the project directory.", "parameters": {"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}}},
]


def call(url, messages, key):
    body = {"messages": messages, "tools": TOOLS, "tool_choice": "auto", "max_tokens": 2500, "temperature": 0.6, "seed": 7}
    req = urllib.request.Request(url + "/v1/chat/completions", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json", "Authorization": "Bearer " + key})
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=1800) as r:
        d = json.load(r)
    return d, time.time() - t0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://127.0.0.1:18400")
    ap.add_argument("--turns", type=int, default=6)
    ap.add_argument("--key", default="local")
    a = ap.parse_args()
    messages = [{"role": "system", "content": SYSTEM},
                {"role": "user", "content": "Project directory: /tmp/p\n\nTask: a Python command-line word counter that prints lines, words and characters of a file, with a unittest."}]
    rows = []
    for turn in range(a.turns):
        d, wall = call(a.url, messages, a.key)
        t = d.get("timings", {})
        msg = d["choices"][0]["message"]
        messages.append(msg)
        calls = msg.get("tool_calls") or []
        row = {"turn": turn + 1, "cache_n": t.get("cache_n", 0), "prompt_n": t.get("prompt_n", 0),
               "prompt_ms": round(t.get("prompt_ms", 0)), "gen_n": t.get("predicted_n", 0), "wall_s": round(wall, 1),
               "call": calls[0]["function"]["name"] if calls else "(answer)"}
        rows.append(row)
        print(json.dumps(row), flush=True)
        if not calls:
            messages.append({"role": "user", "content": "Now add a --json flag that prints the counts as JSON."})
            continue
        for c in calls:
            name = c["function"]["name"]
            out = {"write_file": "wrote 1840 bytes", "read_file": "import sys\n" * 30,
                   "shell": "exit=0\n..\n----------------------------------------------------------------------\nRan 2 tests in 0.001s\n\nOK"}.get(name, "ok")
            messages.append({"role": "tool", "tool_call_id": c.get("id", ""), "content": out})
    total_prompt = sum(r["prompt_n"] for r in rows)
    total_cached = sum(r["cache_n"] for r in rows)
    print(f"\nread again: {total_prompt} tokens; from the cache: {total_cached}; "
          f"{total_prompt / max(1, total_prompt + total_cached):.0%} of all prompt tokens were read (again)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
