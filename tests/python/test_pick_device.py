"""genesis-pick-device: the arrangement it keeps, given what each one measured.

The point of the tool is that no table of device names decides this, so the tests are about the decision:
a crash is an answer, a device that barely wins is not worth its memory, and the CPU is the fallback that
always exists.
"""
import importlib.util, json, pathlib, subprocess, sys, types

SRC = pathlib.Path(__file__).resolve().parents[2] / "system_files/usr/bin/genesis-pick-device"


def load(monkeypatch=None):
    spec = importlib.util.spec_from_loader("pick", loader=None)
    m = types.ModuleType("pick")
    m.__dict__["__file__"] = str(SRC)
    exec(compile(SRC.read_text(), str(SRC), "exec"), m.__dict__)
    return m


def run(monkeypatch, measured, devices=("Vulkan0",)):
    """Run main() with the measurements faked, and return the parsed JSON it printed."""
    pick = load()
    monkeypatch.setattr(pick, "smallest_model", lambda: "/models/fast/x.gguf")
    monkeypatch.setattr(pick, "devices", lambda: list(devices))
    monkeypatch.setattr(pick, "measure", lambda model, dev, threads, seconds: measured[dev])
    out = []
    monkeypatch.setattr("builtins.print", lambda *a, **k: out.append(" ".join(str(x) for x in a)) if k.get("file") is None else None)
    pick.main(["--json"])
    return json.loads("\n".join(out))


def test_a_device_that_crashes_is_not_chosen(monkeypatch):
    # the first real laptop: the Intel chip ran out of memory, so the CPU is what is left
    r = run(monkeypatch, {"none": (7.55, ""), "Vulkan0": (None, "ran out of memory on the device")})
    assert r["device"] == "none"
    assert r["flags"] == "-ngl 0 -dev none"
    assert [t["why_not"] for t in r["tried"] if t["device"] == "Vulkan0"] == ["ran out of memory on the device"]


def test_a_real_card_wins_by_a_mile(monkeypatch):
    r = run(monkeypatch, {"none": (7.5, ""), "Vulkan0": (95.0, "")})
    assert r["device"] == "Vulkan0"
    assert r["flags"] == "-ngl 99 -dev Vulkan0"
    assert r["ngl"] == 99


def test_a_device_that_barely_wins_is_not_worth_its_memory(monkeypatch):
    # 8% faster costs a few gigabytes and adds a way to crash: not worth it
    r = run(monkeypatch, {"none": (7.0, ""), "Vulkan0": (7.6, "")})
    assert r["device"] == "none"


def test_the_fastest_of_several_devices_is_chosen(monkeypatch):
    r = run(monkeypatch, {"none": (7.0, ""), "Vulkan0": (12.0, ""), "Vulkan1": (40.0, "")},
            devices=("Vulkan0", "Vulkan1"))
    assert r["device"] == "Vulkan1"


def test_everything_failing_still_produces_a_usable_answer(monkeypatch):
    # a machine where even the CPU run timed out must still boot with something
    r = run(monkeypatch, {"none": (None, "took longer than the time allowed"), "Vulkan0": (None, "exited 1")})
    assert r["device"] == "none"
    assert r["flags"] == "-ngl 0 -dev none"


def test_it_parses_what_llama_bench_actually_prints(monkeypatch):
    """The device list and the JSON come from llama.cpp, so parse the real shapes, not invented ones."""
    pick = load()
    listing = (
        "load_backend: loaded Vulkan backend from /usr/lib/genesis/llama.cpp/libggml-vulkan.so\n"
        "Available devices:\n"
        "  Vulkan0: Intel(R) HD Graphics 530 (SKL GT2) (11900 MiB, 9749 MiB free)\n"
        "  Vulkan1: NVIDIA GeForce GTX 960M (NVK GM107) (4352 MiB, 3916 MiB free)\n"
    )
    monkeypatch.setattr(pick.subprocess, "run", lambda *a, **k: subprocess.CompletedProcess(a, 0, listing, ""))
    assert pick.devices() == ["Vulkan0", "Vulkan1"]

    rows = json.dumps([
        {"n_prompt": 32, "n_gen": 0, "avg_ts": 16.98},
        {"n_prompt": 0, "n_gen": 16, "avg_ts": 7.55},
    ])
    monkeypatch.setattr(pick.subprocess, "run", lambda *a, **k: subprocess.CompletedProcess(a, 0, rows, ""))
    tps, why = pick.measure("/m.gguf", "none", 6, 60)
    assert (round(tps, 2), why) == (7.55, "")

    # the crash the laptop actually produced
    monkeypatch.setattr(pick.subprocess, "run", lambda *a, **k: subprocess.CompletedProcess(a, -6, "", "terminate called after throwing an instance of 'vk::OutOfDeviceMemoryError'"))
    tps, why = pick.measure("/m.gguf", "Vulkan0", 6, 60)
    assert tps is None and "out of memory" in why


def test_it_measures_with_a_model_that_can_generate(tmp_path, monkeypatch):
    """An embedding model produces no tokens, so measuring generation with it measures nothing.

    On the first real laptop the embedder was the smallest file on disk and was picked, and every
    arrangement came back with no measurement at all.
    """
    pick = load()
    for role, name, size in [("embed", "e.gguf", 10), ("rerank", "r.gguf", 20),
                             ("fim", "f.gguf", 300), ("code", "c.gguf", 900)]:
        d = tmp_path / role
        d.mkdir()
        (d / name).write_bytes(b"x" * size)
    monkeypatch.setattr(pick, "MODELS", str(tmp_path))
    chosen = pick.smallest_model()
    assert chosen.endswith("f.gguf"), f"the smallest model that can generate, not the smallest file: {chosen}"


def test_a_measurement_that_measured_nothing_says_so(monkeypatch):
    """Everything failing is not a decision to use the CPU, and must not look like one.

    The arm64 image shipped llama-bench without its libraries, so every run exited 127 and the file it
    wrote was indistinguishable from a real answer that had chosen the CPU. The CPU is still what to run
    with; what changes is that nobody can mistake it for something that was measured.
    """
    r = run(monkeypatch, {"none": (None, "exited 127"), "Vulkan0": (None, "exited 127")})
    assert r["device"] == "none", "the CPU is still the safe thing to run with"
    assert r["measured"] is False
    assert "127" in r["why_not"]


def test_a_real_measurement_is_marked_as_one(monkeypatch):
    r = run(monkeypatch, {"none": (7.5, ""), "Vulkan0": (95.0, "")})
    assert r["measured"] is True
    assert r["device"] == "Vulkan0"
