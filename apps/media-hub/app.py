#!/usr/bin/python3
"""Media: one window for the services you pay for, and a player for your own IPTV subscription.

A small HTTP server on the loopback address with a page in front of it. It launches things and asks your
IPTV provider what your subscription carries; it does not stream, store or decode anything itself.

  app.py [--port 5600]

Every streaming service is opened into ONE Firefox profile, so a login stays a login and all of them live
in the same place. Firefox is what handles DRM, and it fetches Widevine itself if you allow it — nothing
proprietary ships here. IPTV streams are handed to Haruna, the player already on the machine.
"""
import argparse, json, os, re, shutil, subprocess, sys, urllib.parse, urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
STATIC = os.path.join(HERE, "static")
CONFIG = os.path.expanduser("~/.config/genesis-media")
IPTV_FILE = os.path.join(CONFIG, "iptv.json")
PROFILE = os.path.expanduser("~/.local/share/genesis-media/firefox")

# The services people actually subscribe to. Opening a website is not a partnership with anyone: these are
# the addresses you would type yourself, and each one asks you to log in exactly as it would in a browser.
SERVICES = [
    {"id": "netflix", "name": "Netflix", "url": "https://www.netflix.com", "colour": "#e50914", "drm": True},
    {"id": "disney", "name": "Disney+", "url": "https://www.disneyplus.com", "colour": "#0063e5", "drm": True},
    {"id": "hbo", "name": "HBO Max", "url": "https://play.max.com", "colour": "#8a2be2", "drm": True},
    {"id": "prime", "name": "Prime Video", "url": "https://www.primevideo.com", "colour": "#00a8e1", "drm": True},
    {"id": "youtube", "name": "YouTube", "url": "https://www.youtube.com", "colour": "#ff0000", "drm": False},
    {"id": "spotify", "name": "Spotify", "url": "https://open.spotify.com", "colour": "#1db954", "drm": True},
    {"id": "skyshowtime", "name": "SkyShowtime", "url": "https://www.skyshowtime.com", "colour": "#7b61ff", "drm": True},
    {"id": "twitch", "name": "Twitch", "url": "https://www.twitch.tv", "colour": "#9146ff", "drm": False},
]


def read_iptv():
    try:
        with open(IPTV_FILE) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def write_iptv(d):
    os.makedirs(CONFIG, mode=0o700, exist_ok=True)
    # the file carries a password: nobody else on this machine needs to read it
    fd = os.open(IPTV_FILE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(d, f, indent=1)


def ensure_profile():
    """One Firefox profile for every service, with DRM allowed so the services that need it can ask."""
    if not os.path.isdir(PROFILE):
        os.makedirs(PROFILE, exist_ok=True)
    user_js = os.path.join(PROFILE, "user.js")
    if not os.path.exists(user_js):
        with open(user_js, "w") as f:
            # Firefox asks before it downloads Widevine; this only says the option exists
            f.write('user_pref("media.eme.enabled", true);\n')
            f.write('user_pref("media.gmp-widevinecdm.enabled", true);\n')
            f.write('user_pref("browser.shell.checkDefaultBrowser", false);\n')
            f.write('user_pref("datareporting.healthreport.uploadEnabled", false);\n')
            # A fresh profile opens Firefox's welcome tour and puts the address that was asked for behind
            # it. Someone who pressed Netflix wants Netflix, not an onboarding tour in a browser they did
            # not choose to set up — this profile exists to hold logins, not to be configured.
            f.write('user_pref("browser.aboutwelcome.enabled", false);\n')
            f.write('user_pref("browser.startup.homepage_override.mstone", "ignore");\n')
            f.write('user_pref("startup.homepage_welcome_url", "");\n')
            f.write('user_pref("startup.homepage_welcome_url.additional", "");\n')
            f.write('user_pref("browser.messaging-system.whatsNewPanel.enabled", false);\n')
            f.write('user_pref("trailhead.firstrun.didSeeAboutWelcome", true);\n')
    return PROFILE


def no_desktop():
    """Started without a desktop to open windows on — over SSH, or from a service with no session.

    Firefox's own words for this are "Error: no DISPLAY environment variable specified", which tells
    somebody who just pressed Netflix nothing at all.
    """
    if os.environ.get("WAYLAND_DISPLAY") or os.environ.get("DISPLAY"):
        return None
    return ("Media is not attached to a desktop, so it cannot open a window. Start it from the "
            "application menu, or from a terminal on this machine rather than over SSH.")


def open_service(url):
    blocked = no_desktop()
    if blocked:
        return False, blocked
    firefox = shutil.which("firefox")
    if not firefox:
        return False, "Firefox is not installed, and it is what handles the DRM these services need."
    p = ensure_profile()
    # No --no-remote. This profile is almost always already open — it is where this page is being read —
    # and --no-remote refuses to speak to a running instance, so a second one dies at once and pressing a
    # service did nothing at all. Naming the profile is enough: a window for it opens in the instance that
    # already has it, which is the point of keeping every login in one place.
    cmd = [firefox, "--profile", p, "--new-window", url]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
    # Firefox that cannot start says so within a moment and exits; one that worked is still running. Either
    # way the person hears back, instead of pressing a tile and watching nothing happen.
    try:
        _, err = proc.communicate(timeout=2.5)
        if proc.returncode != 0:
            msg = (err or b"").decode("utf-8", "ignore").strip().splitlines()
            return False, "Firefox could not open it: " + (msg[-1] if msg else f"it stopped with code {proc.returncode}")
    except subprocess.TimeoutExpired:
        pass  # still running, which is what a window being open looks like
    return True, ""


def play(url):
    blocked = no_desktop()
    if blocked:
        return False, blocked
    for player in ("haruna", "mpv", "vlc"):
        exe = shutil.which(player)
        if exe:
            subprocess.Popen([exe, url], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            return True, player
    return False, "no player found (looked for haruna, mpv and vlc)"


def xtream(base, user, password, action=None, **params):
    """Ask an Xtream Codes provider what this subscription carries. Their address, nobody else's."""
    q = {"username": user, "password": password}
    if action:
        q["action"] = action
    q.update({k: v for k, v in params.items() if v})
    url = f"{base.rstrip('/')}/player_api.php?" + urllib.parse.urlencode(q)
    with urllib.request.urlopen(url, timeout=30) as r:
        return json.load(r)


def parse_m3u(text):
    """A plain playlist: #EXTINF lines with a name, then the address on the next line."""
    out, name, logo, group = [], None, "", ""
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("#EXTINF"):
            logo = (re.search(r'tvg-logo="([^"]*)"', line) or [None, ""])[1] if 'tvg-logo="' in line else ""
            group = (re.search(r'group-title="([^"]*)"', line) or [None, ""])[1] if 'group-title="' in line else ""
            name = line.split(",", 1)[1].strip() if "," in line else "channel"
        elif line and not line.startswith("#"):
            out.append({"name": name or line, "url": line, "logo": logo, "group": group})
            name, logo, group = None, "", ""
    return out


class H(BaseHTTPRequestHandler):
    server_version = "genesis-media/1.0"
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def _local(self):
        """This answers the page it serves and nothing else: not another site, not another machine."""
        host = (self.headers.get("Host") or "").split(":")[0]
        if host not in ("127.0.0.1", "localhost", "[::1]", "::1", ""):
            self._json({"error": "forbidden"}, 403)
            return False
        origin = self.headers.get("Origin")
        if origin and not origin.startswith(("http://127.0.0.1", "http://localhost")):
            self._json({"error": "forbidden: a web page cannot drive this"}, 403)
            return False
        return True

    def _json(self, obj, code=200):
        b = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def _file(self, name):
        path = os.path.join(STATIC, name)
        if not os.path.isfile(path) or not os.path.abspath(path).startswith(STATIC):
            self._json({"error": "not found"}, 404)
            return
        kind = {"html": "text/html", "js": "text/javascript", "css": "text/css"}.get(name.rsplit(".", 1)[-1], "text/plain")
        with open(path, "rb") as f:
            b = f.read()
        self.send_response(200)
        self.send_header("Content-Type", kind + "; charset=utf-8")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def _body(self):
        n = int(self.headers.get("Content-Length", 0) or 0)
        try:
            return json.loads(self.rfile.read(n) or b"{}")
        except ValueError:
            return {}

    def do_GET(self):
        if not self._local():
            return
        path = urllib.parse.urlparse(self.path).path
        if path == "/":
            self._file("index.html")
        elif path in ("/app.js", "/style.css"):
            self._file(path.lstrip("/"))
        elif path == "/api/services":
            self._json({"services": SERVICES, "firefox": bool(shutil.which("firefox")),
                        "player": next((p for p in ("haruna", "mpv", "vlc") if shutil.which(p)), None)})
        elif path == "/api/iptv":
            d = read_iptv()
            # the page never needs the password back
            self._json({"configured": bool(d.get("host")), "host": d.get("host", ""), "username": d.get("username", ""), "kind": d.get("kind", "xtream")})
        elif path == "/api/iptv/channels":
            self._channels()
        else:
            self._json({"error": "not found"}, 404)

    def do_POST(self):
        if not self._local():
            return
        path = urllib.parse.urlparse(self.path).path
        body = self._body()
        if path == "/api/open":
            url = next((s["url"] for s in SERVICES if s["id"] == body.get("id")), None)
            if not url:
                return self._json({"error": "no such service"}, 404)
            ok, why = open_service(url)
            self._json({"ok": ok, "error": why} if not ok else {"ok": True})
        elif path == "/api/iptv":
            kind = body.get("kind", "xtream")
            d = {"kind": kind, "host": (body.get("host") or "").strip(), "username": (body.get("username") or "").strip(),
                 "password": body.get("password") or "", "m3u": (body.get("m3u") or "").strip()}
            if kind == "xtream" and not (d["host"] and d["username"]):
                return self._json({"error": "an address and a username are needed"}, 400)
            if kind == "m3u" and not d["m3u"]:
                return self._json({"error": "a playlist address is needed"}, 400)
            write_iptv(d)
            self._json({"ok": True})
        elif path == "/api/iptv/forget":
            try:
                os.remove(IPTV_FILE)
            except OSError:
                pass
            self._json({"ok": True})
        elif path == "/api/play":
            url = body.get("url") or ""
            if not url.startswith(("http://", "https://", "rtsp://", "rtmp://")):
                return self._json({"error": "that is not a stream address"}, 400)
            ok, which = play(url)
            self._json({"ok": ok, "player": which} if ok else {"error": which}, 200 if ok else 500)
        else:
            self._json({"error": "not found"}, 404)

    def _channels(self):
        d = read_iptv()
        if not d:
            return self._json({"error": "no IPTV subscription is set up yet"}, 404)
        try:
            if d.get("kind") == "m3u":
                with urllib.request.urlopen(d["m3u"], timeout=60) as r:
                    items = parse_m3u(r.read().decode("utf-8", "ignore"))
                return self._json({"channels": items[:4000], "total": len(items)})
            live = xtream(d["host"], d["username"], d["password"], "get_live_streams")
            base = d["host"].rstrip("/")
            items = [{
                "name": c.get("name", "channel"),
                "logo": c.get("stream_icon", ""),
                "group": str(c.get("category_id", "")),
                "url": f"{base}/live/{urllib.parse.quote(d['username'])}/{urllib.parse.quote(d['password'])}/{c.get('stream_id')}.m3u8",
            } for c in (live if isinstance(live, list) else [])]
            self._json({"channels": items[:4000], "total": len(items)})
        except urllib.error.HTTPError as e:
            self._json({"error": f"the provider answered {e.code}: check the address, the username and the password"}, 502)
        except Exception as e:
            self._json({"error": f"could not reach the provider: {str(e)[:160]}"}, 502)


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=5600)
    a = ap.parse_args(argv)
    srv = ThreadingHTTPServer(("127.0.0.1", a.port), H)
    srv.daemon_threads = True
    print(f"Media is at http://127.0.0.1:{a.port}/", flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
