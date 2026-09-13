#!/usr/bin/env bash
# Native-speed Genesis VM on Apple Silicon: the arm64 flavour under Apple's hypervisor (HVF).
#   prototype/vm-dev-arm.sh [base-arm64.qcow2]   default: newest iso/output/v*-arm64/disk.qcow2 or iso/output/arm64/disk.qcow2
set -euo pipefail
cd "$(dirname "$0")/.."
base="${1:-$(ls -d iso/output/*arm64*/disk.qcow2 2>/dev/null | sort | tail -1)}"
[ -f "$base" ] || { echo "no arm64 disk image (build one with: just image-arm64 && just qcow2-arm64)"; exit 1; }
ovl=iso/output/dev-arm64-overlay.qcow2
if [ ! -f "$ovl" ] || [ "$(cat iso/output/dev-arm64-overlay.base 2>/dev/null)" != "$base" ]; then
  qemu-img create -q -f qcow2 -F qcow2 -b "$(cd "$(dirname "$base")" && pwd)/$(basename "$base")" "$ovl"
  echo "$base" > iso/output/dev-arm64-overlay.base
  echo "fresh overlay on $base"
fi
mkdir -p iso/output/dev
# UEFI firmware for the virt machine; the vars file is per-VM (keeps the boot entry)
fw=/opt/homebrew/share/qemu/edk2-aarch64-code.fd
vars=iso/output/dev/edk2-arm-vars.fd
[ -f "$vars" ] || dd if=/dev/zero of="$vars" bs=1m count=64 2>/dev/null
echo "arm64 dev VM (HVF): ssh -p 2223 genesis@127.0.0.1 (password genesis); wizard http://127.0.0.1:11513 ; maker http://127.0.0.1:11523"
exec qemu-system-aarch64 -M virt,highmem=on -accel hvf -cpu host -smp 6 -m 8192 \
  -drive if=pflash,format=raw,readonly=on,file="$fw" -drive if=pflash,format=raw,file="$vars" \
  -drive file="$ovl",format=qcow2,if=virtio \
  -device virtio-gpu-pci -display cocoa -device qemu-xhci -device usb-kbd -device usb-tablet \
  -device virtio-rng-pci \
  -monitor unix:iso/output/dev/mon-arm.sock,server,nowait -serial file:iso/output/dev/serial-arm.log \
  -netdev user,id=n0,hostfwd=tcp::2223-:22,hostfwd=tcp::11513-:11510,hostfwd=tcp::11523-:11520 -device virtio-net-pci,netdev=n0
