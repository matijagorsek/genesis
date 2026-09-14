#!/usr/bin/env python3
"""CI screenshots: boot the x86_64 disk under KVM headless, log in, skip the wizard, open the menu, capture.
Usage: ci-shots.py disk.qcow2 firmware.fd outdir. Writes PNGs (QEMU's own png screendump)."""
import os, socket, subprocess, sys, time
disk, fw, outdir = sys.argv[1], sys.argv[2], sys.argv[3]
os.makedirs(outdir, exist_ok=True)
work = os.path.join(outdir, "work.qcow2"); sock = os.path.join(outdir, "mon.sock")
subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", "-F", "qcow2", "-b", os.path.abspath(disk), work], check=True)
accel = ["-enable-kvm", "-cpu", "host"] if os.access("/dev/kvm", os.W_OK) else ["-cpu", "max"]
q = subprocess.Popen(["qemu-system-x86_64", "-machine", "q35", *accel, "-smp", "4", "-m", "4096",
    "-drive", f"if=pflash,format=raw,readonly=on,file={fw}", "-drive", f"file={work},format=qcow2,if=virtio",
    "-device", "virtio-vga", "-display", "none", "-vga", "none", "-device", "qemu-xhci", "-device", "usb-tablet",
    "-monitor", f"unix:{sock},server,nowait", "-serial", f"file:{outdir}/serial.log", "-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0"])
def mon(cmd):
    s = socket.socket(socket.AF_UNIX); s.settimeout(3); s.connect(sock)
    try: s.recv(4096)
    except Exception: pass
    s.sendall((cmd + "\n").encode()); time.sleep(0.3)
    try: s.recv(4096)
    except Exception: pass
    s.close()
def shot(name):
    mon(f"screendump {outdir}/{name}.png -f png")
time.sleep(5); t0 = time.time(); login_at = int(os.environ.get("LOGIN_AT", "150")); done = set()
while time.time() - t0 < login_at + 260:
    el = int(time.time() - t0)
    if "boot" not in done and el > 60: shot("01-boot"); done.add("boot")
    if "login" not in done and el > login_at - 10: shot("02-login"); done.add("login")
    if "typed" not in done and el > login_at:
        for ch in "genesis": mon(f"sendkey {ch}"); time.sleep(0.15)
        mon("sendkey ret"); done.add("typed"); print("typed password", flush=True)
    if "wizard" not in done and el > login_at + 90: shot("03-first-run"); done.add("wizard")
    if "skip" not in done and el > login_at + 100: mon("sendkey ctrl-shift-s"); done.add("skip")
    if "desktop" not in done and el > login_at + 170: mon("sendkey meta_l-d"); time.sleep(3); shot("04-desktop"); done.add("desktop")
    if "menu" not in done and el > login_at + 185: mon("sendkey meta_l"); time.sleep(5); shot("05-app-menu"); mon("sendkey esc"); done.add("menu")
    if "maker" not in done and el > login_at + 200: mon("sendkey meta_l-spc"); time.sleep(25); shot("06-palette"); done.add("maker")
    if "maker2" not in done and el > login_at + 260: shot("06-palette"); mon("sendkey esc"); done.add("maker2"); break
    time.sleep(2)
mon("quit"); time.sleep(2); q.kill()
print("captured:", sorted(f for f in os.listdir(outdir) if f.endswith(".png")), flush=True)
