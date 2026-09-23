"""genesis-backup: export from one home, restore into another, and what must never be in the file.

A home is made in a temporary directory with two made projects, Genesis settings, a phone pairing, a
Babel setting and a transaction history -- and things that must not travel: models, keys, node_modules.
Export, list, restore into a fresh home, restore again on top of an existing project. The archive is
also opened by hand, because "restore worked" does not prove "nothing private went in".
"""
import importlib.util, json, os, pathlib, tarfile

SRC = pathlib.Path(__file__).resolve().parents[2] / "system_files/usr/bin/genesis-backup"


def load(home, monkeypatch):
    for k in ("XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME"):
        monkeypatch.delenv(k, raising=False)
    monkeypatch.setenv("HOME", str(home))
    spec = importlib.util.spec_from_loader("genesis_backup", loader=None, origin=str(SRC))
    mod = importlib.util.module_from_spec(spec)
    exec(compile(SRC.read_text(), str(SRC), "exec"), mod.__dict__)
    return mod


def a_home(root):
    home = root / "home"; home.mkdir()
    def w(rel, text="x"):
        p = home / rel; p.parent.mkdir(parents=True, exist_ok=True); p.write_text(text); return p
    w("checklist/.genesis-recipe.json", '{"name": "checklist"}'); w("checklist/app.py", "print(1)")
    w("checklist/node_modules/left-pad/index.js", "junk")
    w("Projects/timer/.genesis-recipe.json", '{"name": "timer"}'); w("Projects/timer/index.html", "<b>")
    w("Projects/timer/.venv/lib/x.py", "junk")
    w("not-made/notes.txt", "a folder with no recipe is not a project")
    w(".config/genesis/settings.json", '{"mode": "trusted"}'); w(".config/genesis/mcp.json", "{}")
    w(".config/kdeconnect/certificate.pem", "CERT"); w(".config/Babel/User/settings.json", '{"theme": "dark"}')
    w(".local/state/genesis/tx/0001.json", '{"job": 1}'); w(".local/share/applications/genesis-checklist.desktop", "[Desktop Entry]")
    # what must never travel
    w(".local/share/genesis/models/fast/model.gguf", "GGUF-bytes"); w(".ssh/id_ed25519", "PRIVATE KEY"); w(".config/genesis/api-keys.json", "should-not-exist-but-if-it-did")
    return home


def test_export_takes_what_genesis_made_and_nothing_private(tmp_path, monkeypatch):
    home = a_home(tmp_path); b = load(home, monkeypatch)
    r = b.export(str(tmp_path / "out"))
    assert r["projects"] == 2 and r["bytes"] > 0
    with tarfile.open(r["file"]) as tar:
        names = tar.getnames()
    assert "projects/checklist/app.py" in names and "projects/Projects/timer/index.html" in names
    assert "config/genesis/settings.json" in names and "config/kdeconnect/certificate.pem" in names
    assert "state/genesis/tx/0001.json" in names and "data/applications/genesis-checklist.desktop" in names
    assert not any("node_modules" in n or ".venv" in n for n in names), "dependency folders re-install; they do not travel"
    assert not any("not-made" in n for n in names), "a folder without a recipe is not a made project"
    assert not any("models" in n or ".gguf" in n or "id_ed25519" in n or ".ssh" in n for n in names), "models re-download and keys never leave"
    man = b.listing(r["file"])["manifest"]
    assert sorted(man["projects"]) == ["Projects/timer", "checklist"] and man["version"] == 1


def test_restore_into_a_fresh_home_puts_it_all_back(tmp_path, monkeypatch):
    home = a_home(tmp_path); b = load(home, monkeypatch)
    f = b.export(str(tmp_path / "out"))["file"]
    new = tmp_path / "new-home"; new.mkdir(); b2 = load(new, monkeypatch)
    r = b2.restore(f)
    assert sorted(r["projects"]) == ["Projects", "checklist"] and r["skipped"] == []
    assert (new / "checklist/app.py").read_text() == "print(1)"
    assert (new / "Projects/timer/index.html").read_text() == "<b>"
    assert (new / ".config/genesis/settings.json").read_text() == '{"mode": "trusted"}'
    assert (new / ".config/kdeconnect/certificate.pem").exists() and (new / ".local/state/genesis/tx/0001.json").exists()
    assert not (new / "checklist/.genesis-restored").exists(), "the marker used during restore is cleaned up"


def test_restore_over_an_existing_project_keeps_both(tmp_path, monkeypatch):
    home = a_home(tmp_path); b = load(home, monkeypatch)
    f = b.export(str(tmp_path / "out"))["file"]
    (home / "checklist/app.py").write_text("print(2)  # edited since the backup")
    r = b.restore(f)
    assert (home / "checklist/app.py").read_text().startswith("print(2)"), "what is there is kept"
    assert (home / "checklist-restored/app.py").read_text() == "print(1)", "the restored one sits beside it"


def test_a_hostile_archive_cannot_write_outside_home(tmp_path, monkeypatch):
    home = tmp_path / "victim"; home.mkdir(); b = load(home, monkeypatch)
    evil = tmp_path / "evil.tar.gz"
    with tarfile.open(evil, "w:gz") as tar:
        p = tmp_path / "payload"; p.write_text("owned")
        tar.add(p, arcname="projects/../../outside.txt")
        tar.add(p, arcname="config/../../../outside2.txt")
    r = b.restore(str(evil))
    assert not (tmp_path / "outside.txt").exists() and not (tmp_path / "outside2.txt").exists()
    assert len(r["skipped"]) == 2, r
