#!/usr/bin/env python3
"""Boot a Genesis qcow2 headless in QEMU and take screenshots on a schedule.
usage: vm-shots.py DISK OUTDIR [seconds...]   (default schedule: 20 60 120 240 360 480 600 720 840 960)
Logs in as genesis/genesis on the graphical login after 300 s (typed via the QEMU monitor)."""
import socket, subprocess, sys, time, os, shutil
disk, outdir = sys.argv[1], sys.argv[2]
plan = [int(x) for x in sys.argv[3:]] or [20, 60, 120, 240, 360, 480, 600, 720, 840, 960]
os.makedirs(outdir, exist_ok=True)
work = os.path.join(outdir, "work.qcow2"); shutil.copyfile(disk, work)
sock = os.path.join(outdir, "mon.sock")
fw = "/opt/homebrew/share/qemu/edk2-x86_64-code.fd"
def mon(cmd):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.settimeout(5); s.connect(sock); time.sleep(0.3)
    try: s.recv(4096)
    except Exception: pass
    s.sendall((cmd + "\n").encode()); time.sleep(0.5)
    try: s.recv(4096)
    except Exception: pass
    s.close()
qemu = subprocess.Popen(["qemu-system-x86_64", "-machine", "q35", "-cpu", "max", "-smp", "4", "-m", "4096",
    "-drive", f"if=pflash,format=raw,readonly=on,file={fw}", "-drive", f"file={work},if=virtio,format=qcow2",
    "-device", "virtio-vga", "-device", "qemu-xhci", "-device", "usb-tablet", "-display", "none", "-vga", "none", "-monitor", f"unix:{sock},server,nowait",
    "-serial", f"file:{outdir}/serial.log", "-netdev", "user,id=n0,hostfwd=tcp::11512-:11510,hostfwd=tcp::11522-:11520", "-device", "virtio-net-pci,netdev=n0"])
time.sleep(8); t0 = time.time(); done = set(); logged = False; login_at = 0
while time.time() - t0 < max(plan) + 5:
    el = int(time.time() - t0)
    for p in plan:
        if el >= p and p not in done:
            try: mon(f"screendump {outdir}/t{p:04d}.ppm")
            except Exception as e: print("mon err", e, flush=True)
            done.add(p); print("shot", p, flush=True)
    if not logged and el > int(os.environ.get("GENESIS_SHOT_LOGIN_AT", "300")):
        try:
            for ch in "genesis": mon(f"sendkey {ch}"); time.sleep(0.15)
            mon("sendkey ret"); logged = True; login_at = el; print("typed password", flush=True)
        except Exception as e: print("key err", e, flush=True)
    # after login: skip the wizard (GENESIS_SHOT_SKIP=1 clicks "Set up models later", bottom-left of the kiosk window)
    if logged and os.environ.get("GENESIS_SHOT_SKIP") and "skip" not in done and el > login_at + 200:
        try:
            x, y = int(90 * 32767 / 1280), int(770 * 32767 / 800)
            mon(f"mouse_move {x} {y}"); time.sleep(0.5); mon("mouse_button 1"); time.sleep(0.2); mon("mouse_button 0")
            time.sleep(1.5); mon("sendkey ctrl-shift-s")   # the wizard's keyboard skip (no confirmation)
            print("clicked skip + ctrl-shift-s", flush=True)
        except Exception as e: print("skip err", e, flush=True)
        done.add("skip")
    # after login: open the application menu for one frame, then close it (GENESIS_SHOT_MENU=1)
    if logged and os.environ.get("GENESIS_SHOT_MENU") and "menu" not in done and el > login_at + 240:
        try:
            mon("sendkey meta_l"); time.sleep(6); mon(f"screendump {outdir}/menu.ppm"); time.sleep(1); mon("sendkey esc")
            print("shot menu", flush=True)
        except Exception as e: print("menu err", e, flush=True)
        done.add("menu")
    time.sleep(3)
try: mon("quit")
except Exception: pass
qemu.wait(timeout=20)
for f in sorted(os.listdir(outdir)):
    if f.endswith(".ppm"):
        subprocess.run(["sips", "-s", "format", "png", os.path.join(outdir, f), "--out", os.path.join(outdir, f[:-4] + ".png")], capture_output=True)
os.remove(work); print("done")
