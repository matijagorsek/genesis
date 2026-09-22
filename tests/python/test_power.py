"""genesis-power: what it reads off a power supply, and what one transition does.

The sysfs layout is the part that varies between laptops (AC0, ACAD, ADP1; a battery slot with nothing
in it; a machine with no mains supply reported at all), so the reading is pinned against a fake tree.
The transition is pinned against a fake router so that "put the big models away, keep the small one"
is tested without a battery in the machine running the test.
"""
import importlib.util, json, os, pathlib, sys, types

SRC = pathlib.Path(__file__).resolve().parents[2] / "system_files/usr/bin/genesis-power"


def load(tmp_path, monkeypatch):
    monkeypatch.delenv("GENESIS_POWER", raising=False)
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "cfg"))
    spec = importlib.util.spec_from_loader("genesis_power", loader=None, origin=str(SRC))
    mod = importlib.util.module_from_spec(spec)
    exec(compile(SRC.read_text(), str(SRC), "exec"), mod.__dict__)
    return mod


def supply(tmp_path, **devices):
    root = tmp_path / "power_supply"
    for name, attrs in devices.items():
        d = root / name
        d.mkdir(parents=True)
        for k, v in attrs.items():
            (d / k).write_text(f"{v}\n")
    return str(root)


def test_reads_a_laptop_on_ac_and_on_battery(tmp_path, monkeypatch):
    m = load(tmp_path, monkeypatch)
    # exactly what the first real laptop shows
    assert m.source(supply(tmp_path / "a", AC0={"type": "Mains", "online": 1}, BAT0={"type": "Battery", "status": "Not charging", "present": 1})) == "ac"
    assert m.source(supply(tmp_path / "b", AC0={"type": "Mains", "online": 0}, BAT0={"type": "Battery", "status": "Discharging", "present": 1})) == "battery"


def test_a_desktop_is_left_alone(tmp_path, monkeypatch):
    m = load(tmp_path, monkeypatch)
    assert m.source(supply(tmp_path / "a", AC={"type": "Mains", "online": 1})) == "none"
    assert m.source(str(tmp_path / "missing")) == "none"
    # a battery slot that reports nothing present is not a battery
    assert m.source(supply(tmp_path / "b", AC={"type": "Mains", "online": 1}, BAT1={"type": "Battery", "present": 0})) == "none"


def test_other_supply_names_and_no_mains_at_all(tmp_path, monkeypatch):
    m = load(tmp_path, monkeypatch)
    assert m.source(supply(tmp_path / "a", ADP1={"type": "Mains", "online": 0}, BAT1={"type": "Battery", "status": "Discharging"})) == "battery"
    assert m.source(supply(tmp_path / "b", ucsi={"type": "USB", "online": 1}, BAT0={"type": "Battery", "status": "Charging"})) == "ac"
    # no mains supply reported: the battery's own word decides
    assert m.source(supply(tmp_path / "c", BAT0={"type": "Battery", "status": "Discharging"})) == "battery"
    assert m.source(supply(tmp_path / "d", BAT0={"type": "Battery", "status": "Full"})) == "ac"


def test_override_for_demos(tmp_path, monkeypatch):
    m = load(tmp_path, monkeypatch)
    monkeypatch.setenv("GENESIS_POWER", "battery")
    assert m.source(str(tmp_path / "missing")) == "battery"


class Resp:
    status = 200
    def __init__(self, body): self._b = body
    def read(self): return self._b
    def __enter__(self): return self
    def __exit__(self, *a): return None


class FakeRouter:
    """Answers /running and counts unloads, like llama-swap does."""
    def __init__(self, loaded):
        self.loaded = list(loaded)
        self.unloaded = []

    def urlopen(self, req, timeout=0):
        url = req if isinstance(req, str) else req.full_url
        method = "GET" if isinstance(req, str) else req.get_method()
        if url.endswith("/running"):
            return Resp(json.dumps({"running": [{"model": x, "state": "ready"} for x in self.loaded]}).encode())
        if "/api/models/unload/" in url and method == "POST":
            name = url.rsplit("/", 1)[1]
            self.unloaded.append(name)
            self.loaded.remove(name)
            return Resp(b"")
        raise AssertionError(f"unexpected request {method} {url}")


def test_going_on_battery_puts_the_big_models_away_and_keeps_the_small_one(tmp_path, monkeypatch):
    m = load(tmp_path, monkeypatch)
    router = FakeRouter(["fast", "code", "chat"])
    monkeypatch.setattr(m.urllib.request, "urlopen", router.urlopen)
    monkeypatch.setattr(m, "notify", lambda *a: None)
    out = m.on_change("ac", "battery")
    assert sorted(router.unloaded) == ["chat", "code"], out
    assert router.loaded == ["fast"], "the small model must stay loaded: the assistant never goes quiet"
    assert "put away: code, chat" in out


def test_plugging_back_in_unloads_nothing(tmp_path, monkeypatch):
    m = load(tmp_path, monkeypatch)
    router = FakeRouter(["fast"])
    monkeypatch.setattr(m.urllib.request, "urlopen", router.urlopen)
    monkeypatch.setattr(m, "notify", lambda *a: None)
    out = m.on_change("battery", "ac")
    assert router.unloaded == [] and "load the big models again" in out


def test_a_person_can_turn_it_off(tmp_path, monkeypatch):
    m = load(tmp_path, monkeypatch)
    router = FakeRouter(["fast", "code"])
    monkeypatch.setattr(m.urllib.request, "urlopen", router.urlopen)
    monkeypatch.setattr(m, "notify", lambda *a: None)
    m.set_saving(False)
    assert not m.saving_enabled()
    out = m.on_change("ac", "battery")
    assert router.unloaded == [] and "off" in out, "saver off means the coder stays on battery"
    m.set_saving(True)
    assert m.saving_enabled()
