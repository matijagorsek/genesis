#!/usr/bin/env bash
# Double-click (or `open`) on macOS: starts the Genesis dev VM in a window with full speed.
cd "$(dirname "$0")/.." || exit 1
exec prototype/vm-dev.sh "$@"
