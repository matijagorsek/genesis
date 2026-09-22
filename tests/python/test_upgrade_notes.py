"""genesis-upgrade-notes: the comparison of two images on disk, and what it says about it.

Two fake deployments, an rpm that answers from a file, and the shape of the first real comparison
(0.2.230 -> 0.2.237 on the first real laptop: kernel 7.1.8 -> 7.1.10, 125 changed, 5 added, two Genesis
tools changed) pinned so the words do not drift.
"""
import importlib.util, json, pathlib, types

SRC = pathlib.Path(__file__).resolve().parents[2] / "system_files/usr/bin/genesis-upgrade-notes"


def load(tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_STATE_HOME", str(tmp_path / "state"))
    monkeypatch.setenv("GENESIS_DEPLOY_DIR", str(tmp_path / "deploy"))
    monkeypatch.setenv("GENESIS_BOOTC_STATUS", str(tmp_path / "status.json"))
    spec = importlib.util.spec_from_loader("gun", loader=None, origin=str(SRC))
    mod = importlib.util.module_from_spec(spec)
    exec(compile(SRC.read_text(), str(SRC), "exec"), mod.__dict__)
    return mod


def deployment(root, version, pkgs, tools, changes="", llama="b10901"):
    root.mkdir(parents=True)
    (root / "usr/lib").mkdir(parents=True)
    (root / "usr/lib/os-release").write_text(f'ID=genesis\nIMAGE_VERSION={version}\n')
    (root / "usr/lib/sysimage/rpm").mkdir(parents=True)
    (root / "usr/lib/sysimage/rpm/fake.txt").write_text("".join(f"{n}\t{v}\n" for n, v in pkgs.items()))
    (root / "usr/bin").mkdir()
    for n, body in tools.items():
        (root / "usr/bin" / n).write_bytes(body)
    (root / "usr/share/genesis").mkdir(parents=True)
    (root / "usr/share/genesis/changes.txt").write_text(changes)
    (root / "usr/lib/genesis/llama.cpp").mkdir(parents=True)
    (root / "usr/lib/genesis/llama.cpp/BUILD").write_text(llama + "\n")
    return root


def fake_rpm(monkeypatch, m):
    def run(cmd, **k):
        assert cmd[0] == "rpm" and cmd[1] == "--dbpath"
        return types.SimpleNamespace(stdout=pathlib.Path(cmd[2], "fake.txt").read_text())
    monkeypatch.setattr(m.subprocess, "run", run)


OLD = {"kernel": "7.1.8-200.fc44", "mesa-dri-drivers": "1:26.2.2-1.fc44", "systemd": "259.8-1.fc44", "firefox": "156.0-1.fc44", "libfoo": "1-1", "old-only": "2-2"}
NEW = {"kernel": "7.1.10-200.fc44", "mesa-dri-drivers": "1:26.2.3-1.fc44", "systemd": "259.9-1.fc44", "firefox": "156.0-1.fc44", "libfoo": "1-2", "intel-npu-firmware": "1-1"}


def test_the_comparison_reads_both_images(tmp_path, monkeypatch):
    m = load(tmp_path, monkeypatch)
    fake_rpm(monkeypatch, m)
    b = deployment(tmp_path / "deploy/aaa.0", "0.2.230", OLD, {"genesis-report": b"#!/bin/sh\nv1", "genesis-agentd": b"\x7fELF\x80\x00binary", "genesis-gone": b"x"},
                   changes="1111111 first change\n")
    s = deployment(tmp_path / "deploy/bbb.0", "0.2.237", NEW, {"genesis-report": b"#!/bin/sh\nv2", "genesis-agentd": b"\x7fELF\x80\x00binary", "genesis-power": b"new"},
                   changes="3333333 the assistant follows the plug\n2222222 a service that fails gets an offer\n1111111 first change\n", llama="b10950")
    c = m.compare(str(b), str(s))
    assert (c["from"], c["to"]) == ("0.2.230", "0.2.237")
    assert c["notable"][0] == ("Kernel", "7.1.8-200.fc44", "7.1.10-200.fc44")
    assert ("Graphics (Mesa)", "26.2.2-1.fc44", "26.2.3-1.fc44") in c["notable"], "the epoch is not something a person needs to see"
    assert not any(l == "Firefox" for l, _, _ in c["notable"]), "an unchanged package is not news"
    assert c["changed_count"] == 4 and c["added"] == ["intel-npu-firmware"] and c["removed"] == ["old-only"]
    assert c["tools_changed"] == ["genesis-report"], "a compiled daemon that did not change is not listed, and does not crash the read"
    assert c["tools_new"] == ["genesis-power"] and c["tools_gone"] == ["genesis-gone"]
    assert c["commits"] == ["3333333 the assistant follows the plug", "2222222 a service that fails gets an offer"], "only the lines the running image does not already have"
    assert c["llama"] == ("b10901", "b10950")


def test_the_words(tmp_path, monkeypatch):
    m = load(tmp_path, monkeypatch)
    c = {"from": "0.2.230", "to": "0.2.237", "notable": [("Kernel", "7.1.8", "7.1.10"), ("Graphics (Mesa)", "26.2.2", "26.2.3"), ("systemd", "259.8", "259.9"), ("Audio (PipeWire)", "1.6.8", "1.6.9")],
         "added": ["intel-npu-firmware"], "removed": [], "changed_count": 125, "tools_new": ["genesis-power"], "tools_gone": [], "tools_changed": ["genesis-report"],
         "commits": ["3333333 the assistant follows the plug"], "llama": None}
    h = m.headline(c)
    assert h.startswith("Genesis 0.2.237: Kernel 7.1.10, Graphics (Mesa) 26.2.3, systemd 259.9") and "new: power" in h
    md = m.markdown(c)
    assert "# Update ready: Genesis 0.2.230 → 0.2.237" in md
    assert "- **Kernel** 7.1.8 → 7.1.10" in md
    assert "- the assistant follows the plug" in md, "the commit line, without its hash"
    assert "- New tools: genesis-power" in md
    assert "125 packages updated, 1 added" in md
    assert "rollback" in md
    # nothing notable and no Genesis change: still a sentence, not an empty section
    plain = dict(c, notable=[], tools_new=[], tools_changed=[], commits=[], added=[])
    assert m.headline(plain) == "Genesis 0.2.237: 125 packages updated"
    assert "## Genesis itself" not in m.markdown(plain)


def test_nothing_staged_and_one_notification_per_image(tmp_path, monkeypatch):
    m = load(tmp_path, monkeypatch)
    fake_rpm(monkeypatch, m)
    (tmp_path / "status.json").write_text(json.dumps({"status": {"booted": {"ostree": {"checksum": "aaa"}}, "staged": None}}))
    c, why = m.current()
    assert c is None and "nothing staged" in why
    deployment(tmp_path / "deploy/aaa.0", "0.2.230", OLD, {})
    deployment(tmp_path / "deploy/bbb.0", "0.2.237", NEW, {})
    (tmp_path / "status.json").write_text(json.dumps({"status": {"booted": {"ostree": {"checksum": "aaa"}}, "staged": {"ostree": {"checksum": "bbb"}, "image": {"imageDigest": "sha256:db82"}}}}))
    c, digest = m.current()
    assert c["to"] == "0.2.237" and digest == "sha256:db82"
    sent = []
    monkeypatch.setattr(m.subprocess, "run", lambda cmd, **k: (sent.append(cmd), types.SimpleNamespace(stdout=""))[1])
    assert m.notify_once(c, digest) == "said"
    assert m.notify_once(c, digest) == "already said", "the same staged image is announced once, however often the status file is rewritten"
    assert len(sent) == 1 and sent[0][0] == "notify-send" and "7.1.10" in " ".join(sent[0])
    assert m.notify_once(c, "sha256:other") == "said", "a newer staged image is news again"
