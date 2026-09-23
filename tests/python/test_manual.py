"""The manual: every picture it shows exists, every link inside it lands, and the image ships the same
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
    assert 'href="manual.html"' in (ROOT / "docs/index.html").read_text()


def test_it_says_the_hard_parts_plainly():
    text = re.sub(r"<[^>]+>", " ", MANUAL.read_text())
    for phrase in ("10 to 30 minutes", "It isn't", "Nothing restarts on its own", "Nothing, by default", "It is working, not stuck", "rollback"):
        assert phrase in text, phrase


def test_the_front_page_shows_only_pictures_that_exist_and_keeps_its_downloads():
    html = (ROOT / "docs/index.html").read_text()
    for src in re.findall(r'<img src="([^"]+)"', html) + re.findall(r'<source src="([^"]+)"', html) + re.findall(r'poster="([^"]+)"', html):
        assert (ROOT / "docs" / src).is_file(), f"the front page shows {src} and it is not in docs/"
    ids = set(re.findall(r'id="([^"]+)"', html))
    for anchor in re.findall(r'href="#([^"]+)"', html):
        assert anchor in ids, f"#{anchor} goes nowhere"
    # the download cards are built from the releases API; the pieces that logic needs must still be there
    for needle in ('id="dl"', 'id="rel"', "api.github.com/repos/matijagorsek/genesis/releases", "SHA256SUMS.", 'href="manual.html"'):
        assert needle in html, needle
    text = re.sub(r"<[^>]+>", " ", html)
    assert "10 to 30 minutes" in text and "Nothing, by default" in text and "one laptop" in text.lower() or "one Intel laptop" in text
