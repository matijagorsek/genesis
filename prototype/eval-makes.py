#!/usr/bin/python3
"""eval-makes: ten small makes against the maker on this machine, scored. The weekly evaluation runs
this inside a fresh VM on the tiny pack (CPU only, the 2B model), so a change to the prompts, the
tools or the compact script shows up as a number rather than a feeling.

  eval-makes.py [--out results.json] [--only N] [--shard I --of N] [--timeout 900]

--shard I --of N runs every Nth make starting at I, so the ten can be split over parallel machines and
the whole run takes as long as its slowest shard rather than the sum of all ten. Round robin, not
contiguous blocks: the makes differ by a factor of forty in how long they take.

A make counts as passed when the session ends in "done", at least one file was written or edited,
and no tool call ended in an error the model did not recover from. Permission prompts are answered
"allow" (this is a throw-away VM). Output: JSON with one row per make, and a Markdown table on stdout.
"""
import json, os, subprocess, sys, time, urllib.request

def _agentd_port():
    """This user's maker daemon (each account has its own port, written to the runtime directory)."""
    try:
        return int(open(os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid()), "genesis", "agentd.port")).read().strip())
    except (OSError, ValueError):
        return 11520

AGENTD = os.environ.get("GENESIS_AGENTD") or "http://127.0.0.1:%d" % _agentd_port()
def _token():
    try:
        return open(os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid()), "genesis", "agentd.token")).read().strip()
    except OSError:
        return ""

TOKEN = ""
try:
    TOKEN = open(os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid()), "genesis", "agentd.token")).read().strip()
except OSError:
    pass

# What each make must actually DO. Until now a make "passed" when the session ended and a file was
# written, which says nothing about whether the thing works: two runs of the same tree scored 9 and 5.
# And a web make passed when any .html was there -- which the template always is: the converter and the
# club site "passed" as the untouched template, run clean (decision 227). A web make now has to hold what
# its request names, in its own pages and scripts.
def text_of(project, exts=(".html", ".js")):
    out = []
    for root, _dirs, files in os.walk(project):
        for f in files:
            if f.endswith(exts):
                try:
                    out.append(open(os.path.join(root, f), errors="replace").read())
                except OSError:
                    pass
    return "\n".join(out).lower()

def html_pages(project):
    # anywhere in the project: the club site that made pages/home.html, pages/about.html and
    # pages/contact.html beside its index is a three-page site
    pages = []
    for root, dirs, files in os.walk(project):
        dirs[:] = [d for d in dirs if d not in ("node_modules", ".git")]
        for f in files:
            if f.endswith(".html"):
                try:
                    pages.append(open(os.path.join(root, f), errors="replace").read())
                except OSError:
                    pass
    return pages

CHECKS = {
    "checklist": lambda d, p: any(f.endswith((".py", ".html", ".js")) for f in d),
    "pomodoro": lambda d, p: any("html" in f for f in d) and "25" in text_of(p) and any(k in text_of(p) for k in ("setinterval", "settimeout", "countdown", "timer")),
    "wordcount": lambda d, p: True,
    "rename": lambda d, p: True,
    # three pages that are not three copies of one page
    "club": lambda d, p: len(html_pages(p)) >= 3 and len(set(html_pages(p))) >= 3,
    "json-flag": lambda d, p: True,
    "temperature": lambda d, p: any("html" in f for f in d) and ("celsius" in text_of(p) or "fahrenheit" in text_of(p)),
    "todo-cli": lambda d, p: True,
    "quotes": lambda d, p: any("html" in f for f in d) and "quote" in text_of(p) and any(k in text_of(p) for k in ("button", "onclick", "addeventlistener")),
    "dice": lambda d, p: True,
}
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
    # the port file is read on every call: the daemon under test writes it when it starts, which can be
    # after this script did (a shard went to the fallback port and never found the maker)
    base = os.environ.get("GENESIS_AGENTD") or "http://127.0.0.1:%d" % _agentd_port()
    token = TOKEN or _token()
    req = urllib.request.Request(base + path, data=data, headers={"Content-Type": "application/json", "X-Genesis-Token": token})
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
    row["errors"] += ["session: " + e.get("text", "")[:900] for e in ev if e["kind"] == "error"][:3]
    # the whole make in order, each call beside the start of what came back, and what Genesis said: a
    # write that "wrote nothing new" counts as a success above, and three of those end a make
    row["timeline"] = []
    for e in ev:
        k = e["kind"]
        if k == "tool_call":
            row["timeline"].append("> " + brief(e))
        elif k == "tool_result":
            row["timeline"].append(("  ok  " if e.get("ok", True) else "  ERR ") + e.get("summary", "")[:110].replace("\n", " "))
        elif k in ("assistant", "error"):
            row["timeline"].append(f"  [{k}] " + e.get("text", "")[:110].replace("\n", " "))
    row["timeline"] = row["timeline"][:120]
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
    # Memory available fell from 1.7 GB to 480 MB across one run of ten makes, and the slowest three makes
    # were all at the bottom of that slide. Who is holding it is a question the run should answer rather
    # than something to reason about from outside: the six largest resident sets, and the totals.
    try:
        out = subprocess.run(["ps", "-eo", "rss,comm,args"], capture_output=True, text=True, timeout=10).stdout.splitlines()[1:]
        procs = []
        for l in out:
            parts = l.split(None, 2)
            if len(parts) >= 2 and parts[0].isdigit():
                procs.append((int(parts[0]) // 1024, parts[1], (parts[2] if len(parts) > 2 else "")[:60]))
        procs.sort(reverse=True)
        row["top_rss"] = [f"{mb} MB {comm} {args}" for mb, comm, args in procs[:6]]
        row["rss_total_mb"] = sum(mb for mb, _, _ in procs)
        row["proc_count"] = len(procs)
    except Exception:
        pass
    # did it finish, and does what it made look like the thing that was asked for
    try:
        made = os.listdir(project)
    except OSError:
        made = []
    row["files"] = made
    row["looks_right"] = bool(CHECKS.get(name, lambda d, p: True)(made, project))
    row["finished"] = row["state"] == "done" and row["writes"] > 0
    row["passed"] = row["finished"] and row["looks_right"]
    return row


def main(argv):
    out = "results.json"; only = None; timeout = 1500; shard = 0; of = 1
    if "--out" in argv: out = argv[argv.index("--out") + 1]
    if "--only" in argv: only = int(argv[argv.index("--only") + 1])
    if "--timeout" in argv: timeout = int(argv[argv.index("--timeout") + 1])
    if "--shard" in argv: shard = int(argv[argv.index("--shard") + 1])
    if "--of" in argv: of = int(argv[argv.index("--of") + 1])
    # the daemon was started a moment ago: a shard asked before it was listening and lost both its makes
    for _ in range(60):
        try:
            health = api("/api/health")
            break
        except OSError:
            time.sleep(2)
    else:
        health = api("/api/health")
    rows = []
    mine = MAKES[:only][shard::of]
    if of > 1: print(f"shard {shard} of {of}: {', '.join(n for n, _ in mine)}", file=sys.stderr, flush=True)

    def save():
        """Write what is known so far. A run of ten makes on a slow machine takes hours, and until now it
        wrote nothing until the last one finished — so a crash, a timeout or an interrupted run left no
        record of the nine that had worked, and nobody could see how it was going while it went."""
        p = sum(1 for r in rows if r.get("passed"))
        json.dump({"model": health.get("model"), "at": time.strftime("%Y-%m-%dT%H:%M:%S"),
                   "passed": p, "total": len(rows), "of_planned": len(mine), "makes": rows},
                  open(out, "w"), indent=1)

    save()
    for name, prompt in mine:
        print(f"== {name}: {prompt}", file=sys.stderr, flush=True)
        try:
            rows.append(one(name, prompt, timeout))
        except Exception as e:
            rows.append({"name": name, "prompt": prompt, "state": "crash", "error": str(e), "passed": False, "writes": 0, "turns": 0, "seconds": 0, "tool_calls": 0, "tool_errors": 0, "preview": False, "summary": ""})
        print(f"   {rows[-1]['state']} writes={rows[-1]['writes']} turns={rows[-1]['turns']} {rows[-1]['seconds']}s", file=sys.stderr, flush=True)
        save()  # after every make, so an interrupted run still says what it learned
    passed = sum(1 for r in rows if r.get("passed"))
    save()
    finished = sum(1 for r in rows if r.get("finished"))
    print(f"## Ten makes on `{health.get('model')}`: **{passed}/{len(rows)} passed** ({finished} finished, {passed} of those look right)\n")
    print("| make | result | files written | turns | tool errors | preview | time | memory left |")
    print("|---|---|---|---|---|---|---|---|")
    for r in rows:
        mem = f"{r.get('mem_available_mb', 0)} MB" + ("" if r.get("router_alive", True) else " · model service gone")
        print(f"| {r['name']} | {'pass' if r.get('passed') else r['state']} | {r['writes']} | {r['turns']} | {r['tool_errors']} | {'yes' if r['preview'] else 'no'} | {r['seconds']}s | {mem} |")
    print("\nWhat was holding memory at the end of each make (largest resident sets):\n")
    for r in rows:
        print(f"- **{r['name']}** — {r.get('mem_available_mb', 0)} MB free, {r.get('proc_count', 0)} processes, {r.get('rss_total_mb', 0)} MB resident in all: " + "; ".join(r.get("top_rss", [])[:4]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
