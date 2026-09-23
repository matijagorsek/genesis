"""The manual (built by the site into docs/manual.html): every picture it shows exists, every link inside it lands, and the image ships the same
pictures it shows. A manual with a broken picture on a machine with no network is worse than none.
"""
import pathlib, re

ROOT = pathlib.Path(__file__).resolve().parents[2]
MANUAL = ROOT / "docs/manual.html"


def test_every_screenshot_exists_and_is_shipped():
    html = MANUAL.read_text()
    srcs = re.findall(r'<img src="([^"]+)"', html)
    assert len(srcs) >= 5, "a manual with pictures"
    containerfile = (ROOT / "Containerfile").read_text()
    for src in srcs:
        assert (ROOT / "docs" / src).is_file(), f"the manual shows {src} and it is not in docs/"
        assert f"docs/{src}" in containerfile, f"the site shows {src} and the image does not carry it: Help would show a broken picture offline"


def test_every_link_in_the_page_lands():
    html = MANUAL.read_text()
    ids = set(re.findall(r'id="([^"]+)"', html))
    for anchor in re.findall(r'href="#([^"]+)"', html):
        assert anchor in ids, f"#{anchor} goes nowhere"
    for href in re.findall(r'href="(https?://[^"]+)"', html):
        assert "github.com/matijagorsek/genesis" in href, f"the manual links only to the project: {href}"


def test_the_manual_is_in_the_menu_and_the_self_check():
    entry = (ROOT / "system_files/usr/share/applications/org.genesis.manual.desktop").read_text()
    assert "Exec=xdg-open /usr/share/doc/genesis/manual.html" in entry
    assert "manual.html" in (ROOT / "system_files/usr/bin/genesis-image-check").read_text()
    assert (ROOT / "site/src/pages/manual.astro").exists(), "the manual is built by the site and copied back for the image"


def test_it_says_the_hard_parts_plainly():
    text = re.sub(r"<[^>]+>", " ", MANUAL.read_text())
    for phrase in ("10 to 30 minutes", "It isn't", "Nothing restarts on its own", "Nothing, by default", "It is working, not stuck", "rollback"):
        assert phrase in text, phrase
