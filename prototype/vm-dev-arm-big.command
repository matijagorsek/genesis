#!/usr/bin/env bash
# A bigger native VM (16 GB, 8 cores): enough for the "cpu" pack with its 9B code model.
cd "$(dirname "$0")/.." || exit 1
export GENESIS_VM_MEM=16384 GENESIS_VM_CPUS=8
exec prototype/vm-dev-arm.sh "$@"
