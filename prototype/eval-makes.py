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


# The harder set (--set hard): what people bring once the first thing is made. Each may start from files
# already in the project, may go on in the same conversation, and where it can, its check RUNS what was
# made rather than reading it. Ten small single-file makes stopped telling anything apart once they
# passed ten times out of ten.
QUOTES_PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Quotes</title>
<style>body{font-family:sans-serif;margin:3em;background:#fff;color:#222}button{padding:.6em 1.2em}</style></head>
<body><h1>Quotes</h1><p id="q">Press the button.</p><button id="next">Another one</button>
<script>
const quotes=["Simplicity is prerequisite for reliability.","Make it work, make it right, make it fast.","Premature optimization is the root of all evil."];
document.getElementById('next').addEventListener('click',()=>{document.getElementById('q').textContent=quotes[Math.floor(Math.random()*quotes.length)]});
</script></body></html>
"""
BUGGY_WC = """import sys

def count(path):
    text = open(path).read()
    return len(text)   # the number of words

if __name__ == "__main__":
    print(count(sys.argv[1]))
"""
TODO_CLI = """import json, os, sys

FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "todo.json")

def load():
    return json.load(open(FILE)) if os.path.exists(FILE) else []

def save(items):
    json.dump(items, open(FILE, "w"))

def main(argv):
    if not argv or argv[0] == "list":
        for i, t in enumerate(load(), 1):
            print(f"{i}. {t}")
    elif argv[0] == "add":
        items = load(); items.append(" ".join(argv[1:])); save(items)
    else:
        print("usage: todo.py add TEXT | list"); return 2
    return 0

if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
"""

def ran(project, *cmd):
    try:
        r = subprocess.run(list(cmd), cwd=project, capture_output=True, text=True, timeout=30)
        return r.returncode, r.stdout + r.stderr
    except Exception as e:
        return -1, str(e)

def wc_fixed(project):
    open(os.path.join(project, "sample.txt"), "w").write("one two three four\nfive six\n")
    py = next((f for f in ("wc.py",) + tuple(sorted(os.listdir(project))) if f.endswith(".py") and os.path.exists(os.path.join(project, f))), None)
    if not py:
        return False
    code, out = ran(project, "python3", py, "sample.txt")
    return code == 0 and "6" in out.split() and "26" not in out

def todo_clear(project):
    ran(project, "python3", "todo.py", "add", "milk"); ran(project, "python3", "todo.py", "add", "bread")
    code, _ = ran(project, "python3", "todo.py", "clear")
    _, listed = ran(project, "python3", "todo.py", "list")
    return code == 0 and "milk" not in listed and "bread" not in listed

HARD = [
    # (name, prompts in one conversation, files there before it starts, check(files, project))
    ("fix-bug", ["The word count this prints is wrong: it counts characters, not words. Fix it."],
     {"wc.py": BUGGY_WC}, lambda d, p: wc_fixed(p)),
    ("add-command", ["Add a clear command to this todo tool that removes every item."],
     {"todo.py": TODO_CLI}, lambda d, p: todo_clear(p)),
    ("dark-mode", ["Add a button to this page that switches between a light and a dark look."],
     {"index.html": QUOTES_PAGE}, lambda d, p: "dark" in text_of(p) and any(k in text_of(p) for k in ("classlist", "toggle", "style.background", "data-theme")) and "quotes" in text_of(p)),
    ("follow-up", ["a todo list web page where you can add items", "Now also let me mark an item as done by clicking it."],
     {}, lambda d, p: any(k in text_of(p) for k in ("line-through", "done", "completed")) and "click" in text_of(p)),
    ("notes-app", ["a small notes web app: a Python backend that keeps notes in a JSON file, and a page to add a note and see them all"],
     {}, lambda d, p: any(f.endswith(".py") for _r, _d, fs in os.walk(p) for f in fs) and any(k in text_of(p) for k in ("fetch(", "fetch (", "xmlhttprequest", "<form")) and "json" in text_of(p, (".py",))),
]


# The chat set (--set chat): questions, not makes -- chat, documents and the machine go through their own
# path in the agent, and nothing checked it. Every answer here can be checked by a program.
NOTES = "Team notes\n\nThe planning meeting is on Thursday at 14:00 in room B12. Bring the budget figures.\n"
PARAGRAPH = ("The library will close for renovation from the first of March until the end of April. During that time "
             "books can be returned at the town hall, and the reading room moves to the school on Hill Street.")

def mem_gb():
    try:
        kb = int(next(l for l in open("/proc/meminfo") if l.startswith("MemTotal")).split()[1])
        return kb / 1024 / 1024
    except Exception:
        return 0

def says_memory(a):
    import re
    g = mem_gb()
    nums = [float(x) for x in re.findall(r"(\d+(?:\.\d+)?)\s*(?:gb|gib|gigabyte)", a)]
    return any(abs(n - g) <= 1.5 for n in nums)

CHATS = [
    # (name, question, files placed in ~/Documents first, check(answer lowercased))
    ("sum", "What is 17 times 23? Answer with the number.", {}, lambda a: "391" in a),
    ("weekday", "Which day of the week comes after Tuesday? One word.", {}, lambda a: "wednesday" in a),
    ("document", "When and where is the planning meeting? It is in ~/Documents/team-notes.txt", {"team-notes.txt": NOTES},
     lambda a: "thursday" in a and "b12" in a),
    ("summary", "Say in one sentence what this means for someone who wants to return a book in March: " + PARAGRAPH, {},
     lambda a: "town hall" in a),
    ("memory", "How much memory (RAM) does this computer have?", {}, says_memory),
    ("error", "I tried to save /etc/hosts and got 'Permission denied'. What does that mean, in plain words?", {},
     lambda a: any(k in a for k in ("administrator", "sudo", "root", "admin", "permission to change", "system file"))),
    ("not-there", "What did I write in my notes about the trip to Lisbon?", {},
     # right when it says there is nothing, however it says it ("I don't see any notes about Lisbon" is right)
     lambda a: any(k in a for k in ("could not find", "couldn't find", "can't find", "cannot find", "did not find", "didn't find", "no notes", "not find", "no information", "don't have", "do not have", "nothing about", "don't see", "do not see", "not about", "no mention", "doesn't mention", "does not mention")) and "sunny" not in a),
]

def one_chat(name, question, docs, check, timeout):
    home_docs = os.path.expanduser("~/Documents")
    os.makedirs(home_docs, exist_ok=True)
    for f, body in docs.items():
        open(os.path.join(home_docs, f), "w").write(body)
    t0 = time.time()
    sid = api("/api/sessions", {"mode": "auto_edit", "project": "", "kind": "chat"})["id"]
    api(f"/api/sessions/{sid}/prompt", {"text": question})
    row = {"name": name, "prompt": question, "state": "running", "turns": 0, "tool_calls": 0, "writes": 0, "tool_errors": 0, "preview": False, "seconds": 0, "summary": ""}
    st = {}
    time.sleep(3)
    while time.time() - t0 < timeout:
        time.sleep(3)
        st = api(f"/api/sessions/{sid}")
        for p in st.get("pending", []):
            try:
                api(f"/api/prompts/{p['request_id']}", {"allow": True})
            except Exception:
                pass
        if st["state"] in ("done", "error"):
            row["state"] = st["state"]
            break
    else:
        row["state"] = "timeout"
    ev = st.get("events", [])
    answer = next((e["text"] for e in reversed(ev) if e["kind"] == "assistant"), "")
    row["summary"] = answer[:300].replace("\n", " ")
    row["tool_calls"] = sum(1 for e in ev if e["kind"] == "tool_call")
    row["calls"] = [e.get("name", "") for e in ev if e["kind"] == "tool_call"][:20]
    row["errors"] = ["session: " + e.get("text", "")[:300] for e in ev if e["kind"] == "error"][:3]
    row["seconds"] = int(time.time() - t0)
    row["finished"] = row["state"] == "done" and bool(answer.strip())
    row["looks_right"] = bool(check(answer.lower()))
    row["passed"] = row["finished"] and row["looks_right"]
    return row


def api(path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    # the port file is read on every call: the daemon under test writes it when it starts, which can be
    # after this script did (a shard went to the fallback port and never found the maker)
    base = os.environ.get("GENESIS_AGENTD") or "http://127.0.0.1:%d" % _agentd_port()
    token = TOKEN or _token()
    req = urllib.request.Request(base + path, data=data, headers={"Content-Type": "application/json", "X-Genesis-Token": token})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def one(name, prompt, timeout, setup=None, check=None):
    project = os.path.expanduser(f"~/Projects/eval/{name}")
    os.makedirs(project, exist_ok=True)
    for f, body in (setup or {}).items():
        open(os.path.join(project, f), "w").write(body)
    prompts = prompt if isinstance(prompt, list) else [prompt]
    t0 = time.time()
    s = api("/api/sessions", {"mode": "auto_edit", "project": project})
    sid = s["id"]
    row = {"name": name, "prompt": " / ".join(prompts), "state": "running", "turns": 0, "tool_calls": 0, "writes": 0, "tool_errors": 0, "preview": False, "seconds": 0, "summary": ""}
    st = {}
    for text in prompts:  # a follow-up goes into the same conversation once the one before has finished
        api(f"/api/sessions/{sid}/prompt", {"text": text})
        time.sleep(3)
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
        if row["state"] != "done":
            break
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
    row["looks_right"] = bool((check or CHECKS.get(name, lambda d, p: True))(made, project))
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
    which = argv[argv.index("--set") + 1] if "--set" in argv else "basic"
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
    plan = [(n, pr, None, None) for n, pr in MAKES] if which == "basic" else list(CHATS) if which == "chat" else list(HARD)
    mine = plan[:only][shard::of]
    if of > 1: print(f"shard {shard} of {of}: {', '.join(m[0] for m in mine)}", file=sys.stderr, flush=True)

    def save():
        """Write what is known so far. A run of ten makes on a slow machine takes hours, and until now it
        wrote nothing until the last one finished — so a crash, a timeout or an interrupted run left no
        record of the nine that had worked, and nobody could see how it was going while it went."""
        p = sum(1 for r in rows if r.get("passed"))
        json.dump({"model": health.get("model"), "at": time.strftime("%Y-%m-%dT%H:%M:%S"),
                   "passed": p, "total": len(rows), "of_planned": len(mine), "makes": rows},
                  open(out, "w"), indent=1)

    save()
    for name, prompt, setup, check in mine:
        print(f"== {name}: {prompt}", file=sys.stderr, flush=True)
        try:
            rows.append(one_chat(name, prompt, setup or {}, check, min(timeout, 900)) if which == "chat" else one(name, prompt, timeout, setup, check))
        except Exception as e:
            rows.append({"name": name, "prompt": prompt, "state": "crash", "error": str(e), "passed": False, "writes": 0, "turns": 0, "seconds": 0, "tool_calls": 0, "tool_errors": 0, "preview": False, "summary": ""})
        print(f"   {rows[-1]['state']} writes={rows[-1]['writes']} turns={rows[-1]['turns']} {rows[-1]['seconds']}s", file=sys.stderr, flush=True)
        save()  # after every make, so an interrupted run still says what it learned
    passed = sum(1 for r in rows if r.get("passed"))
    save()
    finished = sum(1 for r in rows if r.get("finished"))
    print(f"## {'Ten makes' if which == 'basic' else 'Seven questions' if which == 'chat' else 'The harder makes'} on `{health.get('model')}`: **{passed}/{len(rows)} passed** ({finished} finished, {passed} of those look right)\n")
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
