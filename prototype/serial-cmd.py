#!/usr/bin/env python3
"""Run commands in a VM over its QEMU serial console (unix socket). Usage: serial-cmd.py SOCK 'cmd' ['cmd'...]
Logs in as genesis/genesis if a login prompt appears. Prints command output."""
import socket, sys, time, re
sock, cmds = sys.argv[1], sys.argv[2:]
s = socket.socket(socket.AF_UNIX); s.settimeout(1.0)
for _ in range(60):
    try: s.connect(sock); break
    except OSError: time.sleep(1)
buf = b""
def read_for(sec):
    global buf
    end = time.time() + sec
    while time.time() < end:
        try: buf += s.recv(4096)
        except socket.timeout: pass
    return buf
def wait(pattern, timeout):
    global buf
    end = time.time() + timeout
    while time.time() < end:
        read_for(0.5)
        if re.search(pattern, buf.decode("utf-8", "replace")): return True
    return False
s.sendall(b"\n")
if not wait(r"login: $|\$ $", 8): s.sendall(b"\n")
for _ in range(90):
    txt = buf.decode("utf-8", "replace")
    if re.search(r"\$ *$", txt): break
    if re.search(r"login: *$", txt):
        s.sendall(b"genesis\n"); wait(r"assword: *$", 15); s.sendall(b"genesis\n"); wait(r"\$ *$", 30); break
    s.sendall(b"\n"); read_for(3)
for c in cmds:
    buf = b""
    s.sendall((c + " ; echo __END__\n").encode()); wait(r"__END__\s*\n.*\$ *$", 120)
    out = buf.decode("utf-8", "replace")
    out = out.split("__END__\n")[1] if "__END__\n" in out else out
    print(f"$ {c}\n{out.rsplit('__END__',1)[0].strip()}")
