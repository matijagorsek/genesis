#!/bin/bash
# Genesis arm64 VM sized for a 4K monitor (T27p-30 or similar): 90% of 3840x2160, 8 GB, 6 cores.
# Plasma inside runs at 2x, so text is the same size as the Mac's own at "looks like 1920x1080".
cd "$(dirname "$0")/.."
export GENESIS_VM_XRES=3456 GENESIS_VM_YRES=1944 GENESIS_VM_MEM="${GENESIS_VM_MEM:-8192}" GENESIS_VM_CPUS="${GENESIS_VM_CPUS:-6}"
exec prototype/vm-dev-arm.sh "$@"
