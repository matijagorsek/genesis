#!/usr/bin/env bash
# Render prototype/router.yaml with the models directory and the actual GGUF filenames found on disk.
# Each role directory may hold one weights file (and optionally one mmproj-* file for vision).
set -euo pipefail

dest=${GENESIS_MODELS:-$HOME/.local/share/genesis/models}
here=$(cd "$(dirname "$0")" && pwd)

first() { ls "$dest/$1"/*.gguf 2>/dev/null | grep -v mmproj | head -1 || true; }
mmproj() { ls "$dest/$1"/mmproj-F16.gguf "$dest/$1"/mmproj*.gguf 2>/dev/null | head -1 || true; }

FAST=$(first fast)   FAST_MM=$(mmproj fast)
CODE=$(first code)   CODE_MM=$(mmproj code)
EMBED=$(first embed)

for v in FAST CODE EMBED; do
  [ -n "${!v}" ] || echo "warning: no gguf for role $(echo $v | tr A-Z a-z) under $dest" >&2
done

sed -e "s|@FAST@|$FAST|" -e "s|@FAST_MM@|${FAST_MM:+--mmproj $FAST_MM}|" \
    -e "s|@CODE@|$CODE|" -e "s|@CODE_MM@|${CODE_MM:+--mmproj $CODE_MM}|" \
    -e "s|@EMBED@|$EMBED|" \
    "$here/router.yaml"
