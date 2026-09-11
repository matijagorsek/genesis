#!/usr/bin/env bash
# Run the agent daemon locally against the router on 127.0.0.1:8080 with a scratch audit log and tx store.
set -euo pipefail
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:$PATH"
root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$root/prototype/run/agentd"
export GENESIS_AUDIT="$root/prototype/run/agentd/audit.jsonl"
export GENESIS_TX_STORE="$root/prototype/run/agentd/tx"
cd "$root/src" && cargo build -q -p genesis-agentd
if curl -fsS -m 2 http://127.0.0.1:11520/api/health >/dev/null 2>&1; then echo "genesis-agentd already running: open http://127.0.0.1:11520"; exit 0; fi
pkill -f "genesis-agentd serve" 2>/dev/null || true
exec ./target/debug/genesis-agentd serve --listen 127.0.0.1:11520
