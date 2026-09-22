"""genesis-apps: the catalogue is well formed, names resolve, and install does the right kind of thing.

The catalogue is data a person reads, so the tests are about its shape; whether every Flatpak id exists on
Flathub is checked against Flathub itself, from a real machine, not here.
"""
import importlib.util, json, pathlib, types

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "system_files/usr/bin/genesis-apps"
CAT = ROOT / "system_files/usr/share/genesis/apps.json"


def load(monkeypatch):
    monkeypatch.setenv("GENESIS_APPS", str(CAT))
    spec = importlib.util.spec_from_loader("genesis_apps", loader=None, origin=str(SRC))
    mod = importlib.util.module_from_spec(spec)
    exec(compile(SRC.read_text(), str(SRC), "exec"), mod.__dict__)
    mod.CATALOGUE = str(CAT)
    return mod


def test_the_catalogue_is_well_formed():
    cat = json.loads(CAT.read_text())
    seen_ids, seen_refs = set(), set()
    n = 0
    for c in cat["categories"]:
        assert c["name"] and c["apps"]
        for a in c["apps"]:
            n += 1
            for k in ("id", "title", "why", "kind", "ref"):
                assert a.get(k), f"{a} lacks {k}"
            assert a["kind"] in ("here", "flatpak", "toolbox"), a
            assert a["id"] not in seen_ids, f"duplicate id {a['id']}"
            assert a["ref"] not in seen_refs, f"duplicate ref {a['ref']}"
            seen_ids.add(a["id"]); seen_refs.add(a["ref"])
            assert a["id"] == a["id"].lower() and " " not in a["id"], "short names are one lowercase word"
            assert a["why"].endswith("."), f"a reason is a sentence: {a['why']!r}"
            assert len(a["why"]) <= 130, f"one line: {a['why']!r}"
            if a["kind"] == "flatpak":
                assert a["ref"].count(".") >= 2, f"a Flatpak id has a reverse domain: {a['ref']}"
    assert 40 <= n <= 70, f"{n} entries: a catalogue, not a store"


def test_names_resolve_by_short_name_title_or_id(monkeypatch):
    m = load(monkeypatch)
    assert m.find("obsidian")["ref"] == "md.obsidian.Obsidian"
    assert m.find("Obsidian")["id"] == "obsidian"
    assert m.find("md.obsidian.Obsidian")["id"] == "obsidian"
    assert m.find("nothing-like-this") is None


def test_install_does_what_each_kind_needs(monkeypatch):
    m = load(monkeypatch)
    ran = []
    monkeypatch.setattr(m, "run", lambda cmd, timeout=0: (ran.append(cmd), (0, ""))[1])
    code, msg = m.install("obsidian")
    assert code == 0 and ran[-1][:3] == ["flatpak", "install", "--user"] and ran[-1][-1] == "md.obsidian.Obsidian"
    # a per-user install cannot see the system's Flathub: the user remote is added first, once
    assert ran[-2][:3] == ["flatpak", "remote-add", "--user"], "the first real install failed for want of this"
    code, msg = m.install("rust")
    assert code == 0 and ran[-1][:2] == ["genesis-toolbox", "rust"] and "genesis-toolbox rust" in msg
    n = len(ran)
    code, msg = m.install("kate")
    assert code == 0 and len(ran) == n and "already part of Genesis" in msg
    # a raw Flatpak id still works; a word that is neither is refused with a pointer, not sent to flatpak
    code, msg = m.install("org.videolan.VLC")
    assert code == 0 and ran[-1][-1] == "org.videolan.VLC"
    n = len(ran)
    code, msg = m.install("photoshop")
    assert code == 2 and len(ran) == n and "search" in msg
    # an option is never an id: "--all" must not reach flatpak
    code, msg = m.install("--all")
    assert code == 2 and len(ran) == n


def test_remove_never_touches_the_image(monkeypatch):
    m = load(monkeypatch)
    ran = []
    monkeypatch.setattr(m, "run", lambda cmd, timeout=0: (ran.append(cmd), (0, ""))[1])
    code, msg = m.remove("firefox")
    assert code == 2 and not ran and "image" in msg
    code, msg = m.remove("signal")
    assert code == 0 and ran[-1][:2] == ["flatpak", "uninstall"] and ran[-1][-1] == "org.signal.Signal"


def test_the_list_marks_what_is_here(monkeypatch):
    m = load(monkeypatch)
    monkeypatch.setattr(m, "flatpaks_installed", lambda: {"org.signal.Signal"})
    monkeypatch.setattr(m, "toolboxes_present", lambda: {"rust"})
    monkeypatch.setattr(m.shutil, "which", lambda x: "/usr/bin/x" if x in ("firefox", "kate") else None)
    monkeypatch.setattr(m.os.path, "exists", lambda p: False)
    by_id = {a["id"]: a["state"] for c in m.catalogue_with_state() for a in c["apps"]}
    assert by_id["signal"] == "installed" and by_id["telegram"] == "available"
    assert by_id["rust"] == "installed" and by_id["node"] == "available"
    assert by_id["firefox"] == "installed" and by_id["haruna"] == "missing", "a 'here' app that is not here is a fault worth showing"
    text = m.show()
    assert "* signal" in text and "  telegram" in text and "! haruna" in text


def test_the_user_flathub_remote_is_added_once(monkeypatch):
    m = load(monkeypatch)
    ran = []
    def run(cmd, timeout=0):
        ran.append(cmd)
        if cmd[:2] == ["flatpak", "remotes"]: return (0, "flathub\n" if any(c[1] == "remote-add" for c in ran) else "")
        return (0, "")
    monkeypatch.setattr(m, "run", run)
    assert m.install("signal")[0] == 0 and m.install("telegram")[0] == 0
    assert sum(1 for c in ran if c[1] == "remote-add") == 1
