#!/usr/bin/env bash
# Download every model in a pack JSON into $GENESIS_MODELS/<role>/.
# Pack format: see packs/schema.json. Uses `hf download` with include patterns, resumable.
set -euo pipefail

pack=${1:?usage: models.sh packs/<pack>.json}
dest=${GENESIS_MODELS:-$HOME/.local/share/genesis/models}
mkdir -p "$dest"

python3 - "$pack" <<'PY' | while IFS=$'\t' read -r role repo include; do
import json, sys
p = json.load(open(sys.argv[1]))
for m in p["models"]:
    print("\t".join([m["role"], m["repo"], " ".join(m["include"])]))
PY
  echo "== $role  ($repo)"
  mkdir -p "$dest/$role"
  # shellcheck disable=SC2086
  hf download "$repo" --local-dir "$dest/$role" $(for i in $include; do printf -- "--include %s " "$i"; done)
done

echo
echo "== on disk"
du -sh "$dest"/* 2>/dev/null
echo
echo "next: just up && just smoke"
