"""genesis-firstuser, run for real: the script that decides whether a stranger at the console gets an
administrator account. It is run as bash with a passwd file of the test's own and fake useradd, passwd,
getent, userdel and restorecon on the PATH, each of which writes down that it was called.

What is pinned: it fails closed (a passwd file it cannot read, or with no root, or where NSS and the file
disagree, means nothing happens); it does nothing when anyone can already log in; on an empty machine it
makes exactly one wheel account with the name given; it gives up when nobody answers, after five bad
names, and after three failed passwords -- removing the half-made account.
"""
import os, pathlib, stat, subprocess, sys, pytest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "system_files/usr/bin/genesis-firstuser"
pytestmark = pytest.mark.skipif(sys.platform != "linux" and not os.environ.get("GENESIS_WRAPPER_TESTS"), reason="bash, for the image; run where it is real")

ROOT_LINE = "root:x:0:0:root:/root:/bin/bash\n"
SYSTEM = "genesis-ai:x:957:957::/var/lib/genesis:/usr/sbin/nologin\nnobody:x:65534:65534::/:/usr/sbin/nologin\n"
HUMAN = "matija:x:1000:1000:Matija:/home/matija:/bin/bash\n"


def bin_dir(tmp_path, nss_humans="", passwd_fails=0):
    """Fakes for everything the script calls. `log` records the calls."""
    d = tmp_path / "bin"; d.mkdir(exist_ok=True); log = tmp_path / "calls"
    def fake(name, body):
        f = d / name; f.write_text("#!/bin/bash\n" + body); f.chmod(f.stat().st_mode | stat.S_IEXEC)
    fake("getent", f'echo "getent $*" >> {log}\nif [ "$1" = passwd ] && [ -z "$2" ]; then printf "%s" "{ROOT_LINE}{SYSTEM}{nss_humans}"; exit 0; fi\nexit 2\n')
    fake("useradd", f'echo "useradd $*" >> {log}\nexit 0\n')
    fake("userdel", f'echo "userdel $*" >> {log}\nexit 0\n')
    fake("restorecon", f'echo "restorecon $*" >> {log}\nexit 0\n')
    fake("passwd", f'n=$(grep -c "^passwd" {log} 2>/dev/null); n=${{n:-0}}; echo "passwd $*" >> {log}\n[ "$n" -lt {passwd_fails} ] && exit 1\nexit 0\n')
    fake("sleep", 'exit 0\n')  # the script sleeps to let a person read; the tests do not wait
    return d, log


def run(tmp_path, passwd_text, stdin="", nss_humans="", passwd_fails=0, unreadable=False):
    d, log = bin_dir(tmp_path, nss_humans, passwd_fails)
    pw = tmp_path / "passwd"
    if pw.exists(): pw.chmod(0o600); pw.unlink()   # each run starts from its own file, or none
    if log.exists(): log.unlink()
    if passwd_text is not None:
        pw.write_text(passwd_text)
        if unreadable: pw.chmod(0)
    env = dict(os.environ, PATH=f"{d}:{os.environ['PATH']}", GENESIS_PASSWD=str(pw))
    r = subprocess.run(["bash", str(SCRIPT)], input=stdin, capture_output=True, text=True, env=env, timeout=30)
    calls = log.read_text() if log.exists() else ""
    return r, calls


def test_a_machine_someone_can_log_into_is_left_alone(tmp_path):
    r, calls = run(tmp_path, ROOT_LINE + SYSTEM + HUMAN, stdin="intruder\n")
    assert r.returncode == 0 and "useradd" not in calls and "Username" not in r.stdout


def test_it_fails_closed(tmp_path):
    # the file says nobody, NSS says somebody: do nothing
    r, calls = run(tmp_path, ROOT_LINE + SYSTEM, stdin="x\n", nss_humans=HUMAN)
    assert r.returncode == 0 and "useradd" not in calls, "when the two sources disagree, nothing is offered"
    # no root line: the file is not to be trusted
    r, calls = run(tmp_path, SYSTEM, stdin="x\n")
    assert r.returncode == 0 and "useradd" not in calls and "not to be trusted" in r.stderr
    # a file that is not there
    r, calls = run(tmp_path, None, stdin="x\n")
    assert r.returncode == 0 and "useradd" not in calls and "cannot be read" in r.stderr
    if os.geteuid() != 0:
        r, calls = run(tmp_path, ROOT_LINE + SYSTEM, stdin="x\n", unreadable=True)
        assert r.returncode == 0 and "useradd" not in calls


def test_an_empty_machine_gets_one_administrator(tmp_path):
    r, calls = run(tmp_path, ROOT_LINE + SYSTEM, stdin="matija\nMatija G\n")
    assert r.returncode == 0, r.stdout + r.stderr
    assert "useradd -m -G wheel -c Matija G matija" in calls
    assert calls.count("useradd") == 1 and "passwd matija" in calls and "restorecon" in calls
    assert "log in as matija" in r.stdout


def test_bad_names_are_refused_and_it_gives_up_after_five(tmp_path):
    r, calls = run(tmp_path, ROOT_LINE + SYSTEM, stdin="Bad Name\n-x\n9lives\nroot!\n\n")
    assert r.returncode == 1 and "useradd" not in calls and "No usable name" in r.stdout
    # a name already on the machine is refused, and the next one taken
    r, calls = run(tmp_path, ROOT_LINE + SYSTEM + "taken:x:1500:1500::/home/taken:/usr/sbin/nologin\n", stdin="taken\nfree\n\n")
    assert "already taken" in r.stdout and "useradd -m -G wheel -c free free" in calls


def test_nobody_at_the_console_means_nothing_changes(tmp_path):
    r, calls = run(tmp_path, ROOT_LINE + SYSTEM, stdin="")
    assert r.returncode == 1 and "useradd" not in calls and "nobody at this console" in r.stdout


def test_three_failed_passwords_take_the_account_away_again(tmp_path):
    r, calls = run(tmp_path, ROOT_LINE + SYSTEM, stdin="matija\n\n", passwd_fails=3)
    assert r.returncode == 1
    assert sum(1 for l in calls.splitlines() if l == "passwd matija") == 3 and "userdel -r matija" in calls
    assert "account was removed" in r.stdout
