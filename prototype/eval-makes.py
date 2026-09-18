#!/usr/bin/python3
"""eval-makes: ten small makes against the maker on this machine, scored. The weekly evaluation runs
this inside a fresh VM on the tiny pack (CPU only, the 2B model), so a change to the prompts, the
tools or the compact script shows up as a number rather than a feeling.

  eval-makes.py [--out results.json] [--only N] [--timeout 900]

A make counts as passed when the session ends in "done", at least one file was written or edited,
and no tool call ended in an error the model did not recover from. Permission prompts are answered
"allow" (this is a throw-away VM). Output: JSON with one row per make, and a Markdown table on stdout.
"""
import json, os, subprocess, sys, time, urllib.request

AGENTD = os.environ.get("GENESIS_AGENTD", "http://127.0.0.1:11520")
TOKEN = ""
try:
    TOKEN = open(os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid()), "genesis", "agentd.token")).read().strip()
except OSError:
    pass

MAKES = [
    ("checklist", "a checklist app that saves to a file"),
    ("pomodoro", "a pomodoro timer web app with a big countdown and a bell"),
    ("wordcount", "a word-count tool for text files"),
    ("rename", "a script that renames photos in a folder by date taken"),
    ("club", "a simple website for a club with three pages"),
    ("json-flag", "a command-line tool that prints a JSON summary of a text file: lines, words, characters; with a --json flag"),
    ("temperature", "a converter between Celsius and Fahrenheit as a web page with two fields"),
    ("todo-cli", "a todo list command-line tool that stores items in a JSON file: add, list, done"),
    ("quotes", "a web page that shows a random quote from a list each time you press a button"),
    ("dice", "a dice roller script: number of dice and sides as arguments, prints the rolls and the sum"),
]


def api(path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(AGENTD + path, data=data, headers={"Content-Type": "application/json", "X-Genesis-Token": TOKEN})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def one(name, prompt, timeout):
    project = os.path.expanduser(f"~/Projects/eval/{name}")
    os.makedirs(project, exist_ok=True)
    t0 = time.time()
    s = api("/api/sessions", {"mode": "auto_edit", "project": project})
    sid = s["id"]
    api(f"/api/sessions/{sid}/prompt", {"text": prompt})
    row = {"name": name, "prompt": prompt, "state": "running", "turns": 0, "tool_calls": 0, "writes": 0, "tool_errors": 0, "preview": False, "seconds": 0, "summary": ""}
    while time.time() - t0 < timeout:
        time.sleep(5)
        st = api(f"/api/sessions/{sid}")
        for p in st.get("pending", []):
            try:
                api(f"/api/prompts/{p['request_id']}", {"allow": True})
            except Exception:
                pass  # already answered (a prompt that timed out between two polls is a 404, not a crash)
        if st["state"] in ("done", "error"):
            row["state"] = st["state"]
            break
    else:
        row["state"] = "timeout"
        st = api(f"/api/sessions/{sid}")
    ev = st.get("events", [])
    row["tool_calls"] = sum(1 for e in ev if e["kind"] == "tool_call")
    row["writes"] = sum(1 for e in ev if e["kind"] == "tool_call" and e["name"] in ("write_file", "edit_file"))
    row["tool_errors"] = sum(1 for e in ev if e["kind"] == "tool_result" and not e.get("ok", True))
    row["preview"] = any(e["kind"] == "tool_call" and e["name"] == "preview_start" for e in ev) or bool(st.get("preview_url"))
    row["turns"] = next((e["turns"] for e in ev if e["kind"] == "done"), 0)
    # the tool sequence, so a failed make can be read without the VM: name(first argument)
    def brief(e):
        a = e.get("args") or {}
        v = a.get("path") or a.get("command") or a.get("name") or a.get("url") or ""
        return f"{e['name']}({str(v)[:40]})"
    row["calls"] = [brief(e) for e in ev if e["kind"] == "tool_call"][:60]
    row["errors"] = [e.get("summary", "")[:160] for e in ev if e["kind"] == "tool_result" and not e.get("ok", True)][:10]
    row["errors"] += ["session: " + e.get("text", "")[:160] for e in ev if e["kind"] == "error"][:3]
    row["summary"] = next((e["text"] for e in reversed(ev) if e["kind"] == "assistant"), "")[:200].replace("\n", " ")
    row["seconds"] = int(time.time() - t0)
    # what the machine had left when this make ended: the two failures that say "the model service is gone"
    # need this to be answerable without guessing
    try:
        mem = {l.split(":")[0]: int(l.split()[1]) for l in open("/proc/meminfo") if l.startswith(("MemTotal", "MemAvailable"))}
        row["mem_available_mb"] = mem.get("MemAvailable", 0) // 1024
    except OSError:
        row["mem_available_mb"] = 0
    try:
        ps = subprocess.run(["ps", "-eo", "comm"], capture_output=True, text=True, timeout=10).stdout.lower()
        row["python_servers"] = ps.count("python3")
        row["router_alive"] = "llama-server" in ps or "llama-swap" in ps
    except Exception:
        pass
    row["passed"] = row["state"] == "done" and row["writes"] > 0
    return row


def main(argv):
    out = "results.json"; only = None; timeout = 1500
    if "--out" in argv: out = argv[argv.index("--out") + 1]
    if "--only" in argv: only = int(argv[argv.index("--only") + 1])
    if "--timeout" in argv: timeout = int(argv[argv.index("--timeout") + 1])
    health = api("/api/health")
    rows = []
    for name, prompt in MAKES[:only]:
        print(f"== {name}: {prompt}", file=sys.stderr, flush=True)
        try:
            rows.append(one(name, prompt, timeout))
        except Exception as e:
            rows.append({"name": name, "prompt": prompt, "state": "crash", "error": str(e), "passed": False, "writes": 0, "turns": 0, "seconds": 0, "tool_calls": 0, "tool_errors": 0, "preview": False, "summary": ""})
        print(f"   {rows[-1]['state']} writes={rows[-1]['writes']} turns={rows[-1]['turns']} {rows[-1]['seconds']}s", file=sys.stderr, flush=True)
    passed = sum(1 for r in rows if r.get("passed"))
    result = {"model": health.get("model"), "at": time.strftime("%Y-%m-%dT%H:%M:%S"), "passed": passed, "total": len(rows), "makes": rows}
    json.dump(result, open(out, "w"), indent=1)
    print(f"## Ten makes on `{health.get('model')}`: **{passed}/{len(rows)} passed**\n")
    print("| make | result | files written | turns | tool errors | preview | time | memory left |")
    print("|---|---|---|---|---|---|---|---|")
    for r in rows:
        mem = f"{r.get('mem_available_mb', 0)} MB" + ("" if r.get("router_alive", True) else " · model service gone")
        print(f"| {r['name']} | {'pass' if r.get('passed') else r['state']} | {r['writes']} | {r['turns']} | {r['tool_errors']} | {'yes' if r['preview'] else 'no'} | {r['seconds']}s | {mem} |")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
