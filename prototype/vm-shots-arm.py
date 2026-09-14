#!/usr/bin/env python3
"""Screenshots from the arm64 flavour, natively on Apple Silicon (HVF): boot, log in, skip the wizard,
open the menu, capture. Usage: vm-shots-arm.py disk.qcow2 outdir [t1 t2 …]  (seconds after boot)
Writes PNGs into outdir. Login at GENESIS_SHOT_LOGIN_AT (default 70 s), skip 60 s later, menu 40 s after."""
import os, socket, subprocess, sys, time
disk, outdir = sys.argv[1], sys.argv[2]
times = [int(t) for t in sys.argv[3:]] or [20, 60, 120, 150, 180, 220, 260]
os.makedirs(outdir, exist_ok=True)
work = os.path.join(outdir, "work.qcow2"); sock = os.path.join(outdir, "mon.sock")
subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", "-F", "qcow2", "-b", os.path.abspath(disk), work], check=True)
vars_fd = os.path.join(outdir, "vars.fd"); open(vars_fd, "wb").write(b"\0" * (64 << 20))
qemu = subprocess.Popen(["qemu-system-aarch64", "-M", "virt,highmem=on", "-accel", "hvf", "-cpu", "host", "-smp", "4", "-m", "4096",
    "-drive", "if=pflash,format=raw,readonly=on,file=/opt/homebrew/share/qemu/edk2-aarch64-code.fd", "-drive", f"if=pflash,format=raw,file={vars_fd}",
    "-drive", f"file={work},format=qcow2,if=virtio", "-device", "virtio-gpu-pci", "-display", "none", "-device", "qemu-xhci", "-device", "usb-kbd", "-device", "usb-tablet",
    "-monitor", f"unix:{sock},server,nowait", "-serial", f"file:{outdir}/serial.log", "-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0"])
def mon(cmd):
    s = socket.socket(socket.AF_UNIX); s.settimeout(3); s.connect(sock)
    try: s.recv(4096)
    except Exception: pass
    s.sendall((cmd + "\n").encode()); time.sleep(0.3)
    try: s.recv(4096)
    except Exception: pass
    s.close()
time.sleep(8); t0 = time.time(); done = set(); logged = False; login_at = int(os.environ.get("GENESIS_SHOT_LOGIN_AT", "70")); logged_t = 0
while True:
    el = int(time.time() - t0)
    if not logged and el > login_at:
        for ch in "genesis": mon(f"sendkey {ch}"); time.sleep(0.15)
        mon("sendkey ret"); logged = True; logged_t = el; print("typed password", flush=True)
    if logged and "skip" not in done and el > logged_t + 60:
        mon("sendkey ctrl-shift-s"); done.add("skip"); print("skip sent", flush=True)
    if logged and "menu" not in done and el > logged_t + 100:
        mon("sendkey meta_l"); time.sleep(5); mon(f"screendump {outdir}/menu.ppm"); time.sleep(1); mon("sendkey esc"); done.add("menu"); print("shot menu", flush=True)
    for t in times:
        if t not in done and el >= t:
            mon(f"screendump {outdir}/t{t:04d}.ppm"); done.add(t); print(f"shot {t}", flush=True)
    if all(t in done for t in times) and "menu" in done: break
    if el > max(times) + 200: break
    time.sleep(2)
mon("quit"); time.sleep(2); qemu.kill()
for f in sorted(os.listdir(outdir)):
    if f.endswith(".ppm"):
        subprocess.run(["sips", "-s", "format", "png", os.path.join(outdir, f), "--out", os.path.join(outdir, f[:-4] + ".png")], capture_output=True)
        os.remove(os.path.join(outdir, f))
print("done", flush=True)
