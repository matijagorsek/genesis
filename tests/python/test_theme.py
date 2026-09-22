"""genesis-theme make: what is done to a palette before it reaches a screen, and what gets written.

The first two palettes a 4B model ever produced for this are kept verbatim. One of them called itself dark
with a background of #e6e6e6. That is the case the checks exist for.
"""
import configparser, importlib.util, json, pathlib, sys, types

SRC = pathlib.Path(__file__).resolve().parents[2] / "system_files/usr/bin/genesis-theme"

# verbatim from the first real laptop, 22 Sep: "foggy morning by the sea" and "warm autumn library"
SEAMIST = {"name": "SeaMist", "dark": True, "background": "#e6e6e6", "foreground": "#3a3f44", "accent": "#8c9fa5",
           "terminal": ["#e6e6e6", "#a8a8b0", "#5a6570", "#9c8c70", "#6a7c8c", "#8c9fa5", "#708c9c", "#4a5a6a"]}
SAGELEAF = {"name": "Sageleaf", "dark": True, "background": "#2e332d", "foreground": "#d8c6a6", "accent": "#c79d61",
            "terminal": ["#2e332d", "#a37e54", "#8c9e8b", "#c4a574", "#6a7c74", "#9e7a8b", "#8ba89e", "#d8c6a6"]}


def load(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "share"))
    monkeypatch.setenv("XDG_STATE_HOME", str(tmp_path / "state"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    spec = importlib.util.spec_from_loader("genesis_theme", loader=None, origin=str(SRC))
    mod = importlib.util.module_from_spec(spec)
    exec(compile(SRC.read_text(), str(SRC), "exec"), mod.__dict__)
    return mod


def test_dark_is_read_off_the_background_not_off_the_claim(tmp_path, monkeypatch):
    t = load(tmp_path, monkeypatch)
    assert t.check(SEAMIST)["dark"] is False, "a #e6e6e6 background is light whatever the model says"
    assert t.check(SAGELEAF)["dark"] is True


def test_text_reads_on_the_background_whatever_was_proposed(tmp_path, monkeypatch):
    t = load(tmp_path, monkeypatch)
    grey_on_grey = dict(SAGELEAF, foreground="#3a3f3a", accent="#343833")
    c = t.check(grey_on_grey)
    assert t.contrast(c["fg"], c["bg"]) >= 4.5
    assert t.contrast(c["accent"], c["bg"]) >= 3.0
    for col in c["term"][1:7]:
        assert t.contrast(col, c["bg"]) >= 2.5
    # and a palette that already reads is left as it was
    good = t.check(SAGELEAF)
    assert t.rgb_to_hex(good["fg"]) == "#d8c6a6" and t.rgb_to_hex(good["accent"]) == "#c79d61"


def test_the_colour_scheme_is_the_shape_kde_reads(tmp_path, monkeypatch):
    t = load(tmp_path, monkeypatch)
    text = t.render_colors(t.check(SAGELEAF))
    cp = configparser.ConfigParser(strict=False)
    cp.read_string(text)
    for section in ("Colors:View", "Colors:Window", "Colors:Button", "Colors:Selection", "Colors:Header", "Colors:Tooltip", "Colors:Complementary", "General", "WM"):
        assert section in cp, section
    for section in cp.sections():
        if section.startswith("Colors:"):
            for key in ("BackgroundNormal", "ForegroundNormal", "DecorationFocus", "ForegroundNegative", "ForegroundPositive"):
                assert cp[section][key].count(",") == 2, f"{section}/{key} is not r,g,b"
    assert cp["General"]["ColorScheme"] == "GenesisMade" and cp["General"]["Name"] == "Sageleaf"
    assert cp["Colors:View"]["BackgroundNormal"] == "46,51,45", "the view is the background the model chose"
    # selection text reads on the accent
    sel_fg = tuple(int(v) for v in cp["Colors:Selection"]["ForegroundNormal"].split(","))
    sel_bg = tuple(int(v) for v in cp["Colors:Selection"]["BackgroundNormal"].split(","))
    assert t.contrast(sel_fg, sel_bg) >= 3.0


def test_the_konsole_scheme_has_all_sixteen(tmp_path, monkeypatch):
    t = load(tmp_path, monkeypatch)
    text = t.render_konsole(t.check(SAGELEAF))
    cp = configparser.ConfigParser(strict=False)
    cp.read_string(text)
    for i in range(8):
        assert f"Color{i}" in cp and f"Color{i}Intense" in cp
    assert "Background" in cp and "Foreground" in cp
    assert cp["General"]["Description"] == "Sageleaf"


def test_a_wallpaper_is_made_in_the_palette(tmp_path, monkeypatch):
    t = load(tmp_path, monkeypatch)
    try:
        from PIL import Image
    except ImportError:
        import pytest
        pytest.skip("no PIL here; the image ships it")
    out = tmp_path / "w.png"
    assert t.render_wallpaper(t.check(SAGELEAF), str(out), size=(256, 160))
    im = Image.open(out)
    assert im.size == (256, 160)
    corner_a, corner_b = im.getpixel((2, 2)), im.getpixel((253, 157))
    assert corner_a != corner_b, "it is a gradient, not a flat colour"


def test_make_writes_everything_and_remembers_what_it_replaced(tmp_path, monkeypatch):
    t = load(tmp_path, monkeypatch)
    monkeypatch.setattr(t, "ask_model", lambda mood, timeout=0: SAGELEAF)
    ran = []
    monkeypatch.setattr(t, "run", lambda cmd, **k: (ran.append(cmd), types.SimpleNamespace(returncode=0, stdout="GenesisDark\n", stderr=""))[1])
    monkeypatch.setattr(t, "notify", lambda *a: None)
    monkeypatch.setattr(t, "render_wallpaper", lambda c, p, size=None: pathlib.Path(p).write_bytes(b"png") or True)
    assert t.make("warm autumn library") == 0
    share = tmp_path / "share"
    assert (share / "color-schemes/GenesisMade.colors").exists()
    assert (share / "konsole/GenesisMade.colorscheme").exists()
    assert (share / "konsole/Genesis.profile").read_text().count("ColorScheme=GenesisMade") == 1
    assert (share / "wallpapers/GenesisMade.png").exists()
    undo = json.loads((tmp_path / "state/genesis/theme-undo.json").read_text())
    assert undo["scheme"] == "GenesisDark", "what was there before is what undo goes back to"
    applied = [c for c in ran if c[0] == "plasma-apply-colorscheme"]
    assert applied == [["plasma-apply-colorscheme", "GenesisMade"]]
    assert any(c[0] == "plasma-apply-wallpaperimage" for c in ran)


def test_undo_goes_back_and_toggle_still_works(tmp_path, monkeypatch):
    t = load(tmp_path, monkeypatch)
    ran = []
    monkeypatch.setattr(t, "run", lambda cmd, **k: (ran.append(cmd), types.SimpleNamespace(returncode=0, stdout="GenesisMade\n", stderr=""))[1])
    monkeypatch.setattr(t, "notify", lambda *a: None)
    assert t.undo() == 1, "nothing remembered yet"
    t.remember({"scheme": "GenesisLight", "wallpaper": "", "konsole": "BlackOnWhite"})
    assert t.undo() == 0
    assert ["plasma-apply-colorscheme", "GenesisLight"] in ran
    # the old interface: dark, light, toggle -- what the panel key and the phone page call
    ran.clear()
    assert t.shipped("dark") == 0
    assert ["plasma-apply-colorscheme", "GenesisDark"] in ran


def test_undo_to_a_time_with_no_konsole_profile_removes_the_made_one(tmp_path, monkeypatch):
    t = load(tmp_path, monkeypatch)
    ran = []
    monkeypatch.setattr(t, "run", lambda cmd, **k: (ran.append(cmd), types.SimpleNamespace(returncode=0, stdout="GenesisMade\n", stderr=""))[1])
    monkeypatch.setattr(t, "notify", lambda *a: None)
    t.apply_konsole("GenesisMade")
    prof = tmp_path / "share/konsole/Genesis.profile"
    assert prof.exists()
    # what the first real run remembered: no profile at all before the make
    t.remember({"scheme": "GenesisDark", "wallpaper": "/usr/share/wallpapers/Genesis/", "konsole": ""})
    assert t.undo() == 0
    assert not prof.exists(), "the made profile must not outlive the undo"
    assert any("--delete" in c for c in ran)
