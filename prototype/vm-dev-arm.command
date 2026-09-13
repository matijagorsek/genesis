#!/usr/bin/env bash
cd "$(dirname "$0")/.." || exit 1
exec prototype/vm-dev-arm.sh "$@"
