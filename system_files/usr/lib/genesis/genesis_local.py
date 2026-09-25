"""How a local Genesis tool talks to the maker daemon.

The Unix socket in the runtime directory comes first: it is mode 0600, and a sandboxed command cannot
reach it, because the sandbox puts a tmpfs over /run. The loopback port is the fallback, for anything
started before the daemon wrote its socket.
"""
import http.client, json, os, socket


def token():
    try:
        return open(os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid()), "genesis", "agentd.token")).read().strip()
    except OSError:
        return ""


def port():
    """This user's port: every account has its own daemon, and it writes where it listens."""
    try:
        return int(open(os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid()), "genesis", "agentd.port")).read().strip())
    except (OSError, ValueError):
        return 11520


def socket_path():
    return os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid()), "genesis", "agentd.sock")


class _UnixConnection(http.client.HTTPConnection):
    def __init__(self, path, timeout=30):
        super().__init__("localhost", timeout=timeout)
        self._path = path

    def connect(self):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(self.timeout)
        s.connect(self._path)
        self.sock = s


def call(method, path, body=None, timeout=60):
    """One request to the maker daemon; returns the parsed JSON."""
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Content-Type": "application/json", "X-Genesis-Token": token()}
    sock = socket_path()
    conn = _UnixConnection(sock, timeout) if os.path.exists(sock) else http.client.HTTPConnection("127.0.0.1", port(), timeout=timeout)
    try:
        conn.request(method, path, body=data, headers=headers)
        raw = conn.getresponse().read()
        return json.loads(raw or b"{}")
    finally:
        conn.close()
