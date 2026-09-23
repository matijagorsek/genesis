"""genesis-llama-server, the wrapper every model starts through, had no tests: it is the script that decides
whether a machine has an assistant at all. A fake llama-server stands in for the real one -- it prints, it
exits how it is told, it dies how it is told -- and the wrapper is run for real, as bash.

What is pinned: the child's last words reach the wrapper's stderr (the real image lost the exception a
crashing server died with, because the tailer was killed before it drained); a signal to the wrapper
reaches the child; an exit code comes back; a GPU failure is retried on the CPU.
"""
import os, pathlib, signal, stat, subprocess, sys, time, pytest

ROOT = pathlib.Path(__file__).resolve().parents[2]
WRAPPER = ROOT / "system_files/usr/bin/genesis-llama-server"

pytestmark = pytest.mark.skipif(sys.platform != "linux" and not os.environ.get("GENESIS_WRAPPER_TESTS"), reason="the wrapper is bash for the image; run where its tools exist")


def fake_server(tmp_path, body):
    d = tmp_path / "bin"; d.mkdir(exist_ok=True)
    f = d / "llama-server"; f.write_text("#!/bin/bash\n" + body); f.chmod(f.stat().st_mode | stat.S_IEXEC)
    return f


def run(tmp_path, args, timeout=20):
    env = dict(os.environ, GENESIS_LLAMA_BIN=str(tmp_path / "bin/llama-server"))
    return subprocess.run(["bash", str(WRAPPER)] + args, capture_output=True, text=True, timeout=timeout, env=env)


def test_the_childs_last_words_reach_stderr(tmp_path):
    fake_server(tmp_path, 'echo "model loaded" >&2; echo "terminate called after throwing an instance of std::runtime_error" >&2; exit 134\n')
    r = run(tmp_path, ["--port", "1", "-m", "x.gguf"])
    assert "terminate called" in r.stderr, "the line a crashing server dies with must survive the wrapper's exit"


def test_a_clean_exit_code_comes_back(tmp_path):
    fake_server(tmp_path, 'echo hello >&2; exit 0\n')
    assert run(tmp_path, ["--port", "1"]).returncode == 0


def test_a_signal_reaches_the_child(tmp_path):
    fake_server(tmp_path, 'trap "echo child got TERM >&2; exit 0" TERM; echo listening >&2; while :; do sleep 0.2; done\n')
    env = dict(os.environ, GENESIS_LLAMA_BIN=str(tmp_path / "bin/llama-server"))
    p = subprocess.Popen(["bash", str(WRAPPER), "--port", "1"], stderr=subprocess.PIPE, text=True, env=env)
    time.sleep(0.8)
    p.send_signal(signal.SIGTERM)
    try:
        _, err = p.communicate(timeout=10)
    except subprocess.TimeoutExpired:
        p.kill(); pytest.fail("the wrapper did not stop when told to")
    assert "child got TERM" in err, err
