#!/usr/bin/env bash
# Run the first-run wizard locally against this machine's real hardware profile, with all writes in a scratch dir.
set -euo pipefail
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:$PATH"
root=$(cd "$(dirname "$0")/.." && pwd)
scratch=${GENESIS_FIRSTRUN_SCRATCH:-$root/prototype/run/firstrun}
mkdir -p "$scratch/models" "$scratch/etc"
cd "$root/src"
cargo build -q -p genesis-probe -p genesis-firstrun
./target/debug/genesis-probe --packs "$root/packs" --write --out "$scratch/etc/profile.json" --models-dir "$scratch/models" >/dev/null
rm -f "$scratch/first-run-done"
if curl -fsS -m 2 http://127.0.0.1:11510/api/state >/dev/null 2>&1; then
  echo "genesis-firstrun is already running: open http://127.0.0.1:11510"
  echo "(to replace it: pkill -f genesis-firstrun, then run again)"
  exit 0
fi
pkill -f "genesis-firstrun --listen 127.0.0.1:11510" 2>/dev/null || true
exec ./target/debug/genesis-firstrun --listen 127.0.0.1:11510 \
  --profile "$scratch/etc/profile.json" --packs "$root/packs" --models-dir "$scratch/models" \
  --router-out "$scratch/etc/router.yaml" --settings-out "$scratch/etc/settings.json" --done-marker "$scratch/first-run-done"
