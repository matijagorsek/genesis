"""Every account has its own maker daemon on its own port, and every client finds it the same way: the
port the daemon wrote to the runtime directory. A client that went to 11520 regardless talked to whoever
logged in first.
"""
import importlib.util, pathlib, re

ROOT = pathlib.Path(__file__).resolve().parents[2]


def load_local():
    spec = importlib.util.spec_from_file_location("genesis_local", ROOT / "system_files/usr/lib/genesis/genesis_local.py")
    m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
    return m


def test_the_helper_reads_the_port_the_daemon_wrote(tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_RUNTIME_DIR", str(tmp_path))
    local = load_local()
    assert local.port() == 11520, "no file yet: the first user's port"
    (tmp_path / "genesis").mkdir()
    (tmp_path / "genesis/agentd.port").write_text("13001\n")
    assert local.port() == 13001


def test_no_client_calls_a_fixed_port():
    # a genesis-window URL may say 11520 (the window maps it to this user's port); a direct call may not
    direct = re.compile(r"(curl|urlopen|HTTPConnection|XMLHttpRequest|x\.open|http\.request)[^\n]*11520")
    for f in list((ROOT / "system_files").rglob("*")) + list((ROOT / "src").rglob("*.qml")):
        if f.is_file() and f.suffix not in (".png", ".svg", ".gguf", ".webm", ".avif"):
            try:
                text = f.read_text()
            except UnicodeDecodeError:
                continue
            for line in text.splitlines():
                if direct.search(line) and "agentd.port" not in line and "genesisPort" not in line:
                    raise AssertionError(f"{f.relative_to(ROOT)} calls the maker at a fixed port: {line.strip()[:160]}")
