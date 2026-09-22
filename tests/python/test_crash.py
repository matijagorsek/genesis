"""genesis-crash: what it makes of a real crash record, and what it refuses to interrupt somebody for.

The parsing is the part that breaks when systemd changes its output, and the ignore list is the part that
decides whether a person is bothered. Both are worth pinning.
"""
import importlib.util, pathlib, subprocess, sys, types

SRC = pathlib.Path(__file__).resolve().parents[2] / "system_files/usr/bin/genesis-crash"

# what `coredumpctl info` actually prints, kept verbatim
REAL = """           PID: 4242 (okular)
           UID: 1000 (matija)
           GID: 1000 (matija)
        Signal: 11 (SEGV)
     Timestamp: Mon 2026-09-22 09:14:03 CEST (2min ago)
  Command Line: /usr/bin/okular /home/matija/Documents/a.pdf
    Executable: /usr/bin/okular
 Control Group: /user.slice/user-1000.slice/session-2.scope
          Unit: session-2.scope
         Slice: user-1000.slice
       Boot ID: 3f2b1a
    Machine ID: 9c1d2e
      Hostname: genesis
       Storage: /var/lib/systemd/coredump/core.okular.1000.zst (present)
       Message: Process 4242 (okular) of user 1000 dumped core.

                Stack trace of thread 4242:
                #0  0x00007f1 __pthread_kill_implementation (libc.so.6)
                #1  0x00007f2 raise (libc.so.6)
                #2  0x00007f3 abort (libc.so.6)
                #3  0x00007f4 qt_assert (libQt6Core.so.6)
"""


def load():
    m = types.ModuleType("gc")
    m.__dict__["__file__"] = str(SRC)
    exec(compile(SRC.read_text(), str(SRC), "exec"), m.__dict__)
    return m


def test_it_reads_a_real_crash_record(monkeypatch):
    gc = load()
    monkeypatch.setattr(gc.subprocess, "run", lambda *a, **k: subprocess.CompletedProcess(a, 0, REAL, ""))
    d, err = gc.crash_detail(4242)
    assert not err
    assert d["program"] == "okular"
    assert d["signal"].startswith("11")
    assert "a.pdf" in d["command"]
    assert "qt_assert" in d["stack"], "the stack is what makes the answer worth reading"
    assert d["stack"].count("#") <= 25, "a whole stack is not worth sending to a small model"


def test_no_record_is_not_a_crash_of_its_own(monkeypatch):
    gc = load()
    monkeypatch.setattr(gc.subprocess, "run", lambda *a, **k: subprocess.CompletedProcess(a, 0, "", ""))
    d, err = gc.crash_detail(1)
    assert d is None and "no record" in err

    def boom(*a, **k):
        raise FileNotFoundError("coredumpctl")
    monkeypatch.setattr(gc.subprocess, "run", boom)
    d, err = gc.crash_detail(1)
    assert d is None and "could not be run" in err


def test_the_desktop_falling_over_does_not_start_a_conversation():
    gc = load()
    # these crash on the way down, or report their own failures elsewhere; interrupting somebody about
    # plasmashell while their session is restarting helps nobody
    for name in ("plasmashell", "kwin_wayland", "pipewire"):
        assert name in gc.IGNORE
    assert "okular" not in gc.IGNORE and "firefox" not in gc.IGNORE


def test_the_question_says_what_it_knows_and_asks_for_no_more(monkeypatch):
    gc = load()
    sent = {}
    fake = types.ModuleType("genesis_local")
    fake.call = lambda method, path, body=None, timeout=60: (sent.update({path: body}) or {"id": "s1"})
    monkeypatch.setitem(sys.modules, "genesis_local", fake)
    opened = {}
    monkeypatch.setattr(gc.subprocess, "Popen", lambda cmd, **k: opened.update({"cmd": cmd}))
    ok, err = gc.ask_genesis({"program": "okular", "signal": "11 (SEGV)", "command": "/usr/bin/okular",
                              "when": "now", "stack": "#0 qt_assert"})
    assert ok, err
    q = sent["/api/sessions/s1/prompt"]["text"]
    assert "okular" in q and "SEGV" in q and "qt_assert" in q
    assert "plain words" in q
    assert "Do not guess" in q, "a small model asked about a crash will invent a cause if it is not told not to"
    # the window takes a URL and has never taken a --session flag; inventing one opened nothing at all
    assert opened["cmd"][0] == "genesis-window"
    assert opened["cmd"][1].startswith("http://127.0.0.1:11520/?session=")
    assert not any(a.startswith("--session") for a in opened["cmd"])
