#!/usr/bin/env bash
# Dev bridge: let the Genesis VM use the Mac's own Metal-accelerated model service (llama-swap on
# 127.0.0.1:8080, `just up`) instead of CPU inference inside the VM. QEMU user networking reaches the
# host at 10.0.2.2. Dev only: the installed OS never uses another machine.
#   prototype/vm-bridge.sh [ssh-port]   default 2223 (the arm64 VM)
set -euo pipefail
port="${1:-2223}"
command -v socat >/dev/null || { echo "brew install socat"; exit 1; }
curl -s -m 3 http://127.0.0.1:8080/v1/models >/dev/null || { echo "start the Mac model service first: just up"; exit 1; }
pkill -f "socat TCP-LISTEN:18080" 2>/dev/null || true
nohup socat TCP-LISTEN:18080,bind=0.0.0.0,fork,reuseaddr TCP:127.0.0.1:8080 >/dev/null 2>&1 &
ssh_opts=(-p "$port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR)
run() { if command -v sshpass >/dev/null; then sshpass -p genesis ssh "${ssh_opts[@]}" genesis@127.0.0.1 "$@"; else ssh "${ssh_opts[@]}" genesis@127.0.0.1 "$@"; fi; }
run 'mkdir -p ~/.config/systemd/user/genesis-agentd.service.d; printf "[Service]\nEnvironment=GENESIS_ENDPOINT=http://10.0.2.2:18080/v1\nEnvironment=GENESIS_MODEL=code\n" > ~/.config/systemd/user/genesis-agentd.service.d/bridge.conf; printf "export GENESIS_ENDPOINT=http://10.0.2.2:18080/v1\n" > ~/.config/genesis-bridge.sh; grep -q genesis-bridge ~/.bashrc 2>/dev/null || echo ". ~/.config/genesis-bridge.sh" >> ~/.bashrc; export XDG_RUNTIME_DIR=/run/user/1000; systemctl --user daemon-reload; systemctl --user restart genesis-agentd.service; sleep 2; curl -s http://127.0.0.1:11520/api/health'
echo
echo "bridge on: the VM's maker and 'ask' now use the Mac's models (remove ~/.config/systemd/user/genesis-agentd.service.d/bridge.conf in the VM to undo)"
