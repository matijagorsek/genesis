"""Media: the parts that can be wrong without anyone noticing until it matters.

The playlist parser, the stream address built from a subscription, where credentials land on disk and
with what permissions, and that the local server refuses a web page trying to drive it.
"""
import importlib.util, json, os, pathlib, stat, sys, types

SRC = pathlib.Path(__file__).resolve().parents[2] / "apps/media-hub/app.py"


def load(tmp_path=None):
    m = types.ModuleType("mh")
    m.__dict__["__file__"] = str(SRC)
    exec(compile(SRC.read_text(), str(SRC), "exec"), m.__dict__)
    if tmp_path:
        m.CONFIG = str(tmp_path / "cfg")
        m.IPTV_FILE = str(tmp_path / "cfg" / "iptv.json")
        m.PROFILE = str(tmp_path / "ff")
    return m


def test_m3u_with_the_shapes_providers_actually_send():
    mh = load()
    text = (
        "#EXTM3U\n"
        '#EXTINF:-1 tvg-id="rtv1" tvg-logo="http://x/l.png" group-title="Slovenia",RTV SLO 1\n'
        "http://provider:8080/live/u/p/101.m3u8\n"
        "#EXTINF:-1,Channel With, Comma\n"
        "http://provider:8080/live/u/p/102.ts\n"
        "# a stray comment that is not a channel\n"
    )
    ch = mh.parse_m3u(text)
    assert len(ch) == 2
    assert ch[0]["name"] == "RTV SLO 1"
    assert ch[0]["logo"] == "http://x/l.png"
    assert ch[0]["group"] == "Slovenia"
    # a comma inside the name must not lose the rest of it
    assert ch[1]["name"] == "Channel With, Comma"
    assert ch[1]["url"].endswith("102.ts")


def test_credentials_are_written_where_only_you_can_read_them(tmp_path):
    mh = load(tmp_path)
    mh.write_iptv({"kind": "xtream", "host": "http://p:8080", "username": "u", "password": "secret"})
    mode = stat.S_IMODE(os.stat(mh.IPTV_FILE).st_mode)
    assert mode == 0o600, f"the file carries a password: {oct(mode)}"
    assert stat.S_IMODE(os.stat(mh.CONFIG).st_mode) == 0o700
    assert mh.read_iptv()["password"] == "secret"


def test_a_missing_or_broken_file_is_not_a_crash(tmp_path):
    mh = load(tmp_path)
    assert mh.read_iptv() == {}
    os.makedirs(mh.CONFIG, exist_ok=True)
    open(mh.IPTV_FILE, "w").write("{not json")
    assert mh.read_iptv() == {}


def test_the_firefox_profile_allows_drm_and_asks_for_nothing_else(tmp_path):
    mh = load(tmp_path)
    p = mh.ensure_profile()
    prefs = open(os.path.join(p, "user.js")).read()
    assert 'media.eme.enabled", true' in prefs, "the services that need Widevine must be able to ask"
    # someone who pressed Netflix wants Netflix, not Firefox's welcome tour in front of it
    assert 'aboutwelcome.enabled", false' in prefs
    assert 'homepage_override.mstone", "ignore"' in prefs
    assert "healthreport.uploadEnabled\", false" in prefs
    # running it twice must not throw away a profile that already holds logins
    open(os.path.join(p, "cookies.sqlite"), "w").write("x")
    mh.ensure_profile()
    assert os.path.exists(os.path.join(p, "cookies.sqlite")), "a second run must not wipe the logins"


def test_every_service_is_a_real_https_address():
    mh = load()
    ids = [s["id"] for s in mh.SERVICES]
    assert len(ids) == len(set(ids)), "duplicate service ids"
    for s in mh.SERVICES:
        assert s["url"].startswith("https://"), s
        assert s["name"] and s["colour"].startswith("#")


def test_without_a_desktop_it_says_so_in_words_a_person_can_act_on(monkeypatch):
    """Firefox says "no DISPLAY environment variable specified", which helps nobody who pressed Netflix."""
    mh = load()
    monkeypatch.delenv("WAYLAND_DISPLAY", raising=False)
    monkeypatch.delenv("DISPLAY", raising=False)
    ok, why = mh.open_service("https://example.com")
    assert not ok
    assert "not attached to a desktop" in why and "application menu" in why
    ok, why = mh.play("http://example.com/s.m3u8")
    assert not ok and "not attached to a desktop" in why
    # and with a desktop it gets as far as looking for the browser
    monkeypatch.setenv("WAYLAND_DISPLAY", "wayland-0")
    assert mh.no_desktop() is None
