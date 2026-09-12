#!/usr/bin/env bash
# Development VM: boots a Genesis disk with a visible window, SSH on 2222, wizard on 11511, maker on 11521.
# The disk is used through a copy-on-write overlay, so the downloaded release stays clean; delete
# iso/output/dev-overlay.qcow2 to start fresh.
#
#   prototype/vm-dev.sh [base.qcow2]        default: newest iso/output/v*/disk.qcow2
set -euo pipefail
cd "$(dirname "$0")/.."
base="${1:-$(ls -d iso/output/v0.1.*/disk.qcow2 2>/dev/null | sort -t. -k3 -n | tail -1)}"
[ -f "$base" ] || { echo "no disk image; run: just fetch-images"; exit 1; }
ovl=iso/output/dev-overlay.qcow2
if [ ! -f "$ovl" ] || [ "$(cat iso/output/dev-overlay.base 2>/dev/null)" != "$base" ]; then
  qemu-img create -q -f qcow2 -F qcow2 -b "$(cd "$(dirname "$base")" && pwd)/$(basename "$base")" "$ovl"
  echo "$base" > iso/output/dev-overlay.base
  echo "fresh overlay on $base"
fi
cp -n /opt/homebrew/share/qemu/edk2-x86_64-code.fd iso/output/ovmf-code.fd 2>/dev/null || true
echo "dev VM: window opens; ssh -p 2222 genesis@127.0.0.1 (password genesis); wizard http://127.0.0.1:11511 ; maker http://127.0.0.1:11521"
exec qemu-system-x86_64 -machine q35 -cpu max -smp 4 -m 6144 \
  -drive if=pflash,format=raw,readonly=on,file=iso/output/ovmf-code.fd \
  -drive file="$ovl",format=qcow2,if=virtio \
  -device virtio-vga -display cocoa -device usb-tablet \
  -netdev user,id=n0,hostfwd=tcp::2222-:22,hostfwd=tcp::11511-:11510,hostfwd=tcp::11521-:11520 -device virtio-net-pci,netdev=n0
