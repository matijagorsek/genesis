#!/usr/bin/env bash
# Fast loop: cross-compile the daemons (Docker, cached, ~1-3 min incremental), copy them into the running
# dev VM over SSH, and restart the services. No image build, no release, no reboot.
#
#   prototype/vm-push.sh                 push all daemons
#   prototype/vm-push.sh genesis-agentd  push one
#
# Inside the VM /usr is read-only (bootc); `bootc usr-overlay` adds a transient writable layer that
# vanishes on reboot, which is exactly what a dev loop wants.
set -euo pipefail
cd "$(dirname "$0")/.."
bins=("$@"); [ ${#bins[@]} -gt 0 ] || bins=(genesis-permd genesis-probe genesis-firstrun genesis-agentd genesis-txd genesis-krunner)
ssh_opts=(-p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR)
command -v sshpass >/dev/null || echo "tip: brew install sshpass for password-less pushes"
# disks built before v0.1.34 have sshd off: once, in the VM window, run: sudo systemctl enable --now sshd
run_ssh() { if command -v sshpass >/dev/null; then sshpass -p genesis ssh "${ssh_opts[@]}" genesis@127.0.0.1 "$@"; else ssh "${ssh_opts[@]}" genesis@127.0.0.1 "$@"; fi; }
run_scp() { if command -v sshpass >/dev/null; then sshpass -p genesis scp "${ssh_opts[@]}" "$@"; else scp "${ssh_opts[@]}" "$@"; fi; }

echo "== cross-compiling (docker, cached)"
docker build -q -f Containerfile --target daemons -t genesis-daemons . >/dev/null
cid=$(docker create genesis-daemons)
tmp=$(mktemp -d)
for b in "${bins[@]}"; do docker cp -q "$cid:/out/usr/bin/$b" "$tmp/$b"; done
docker rm "$cid" >/dev/null
echo "== copying ${bins[*]} into the VM"
run_scp "${tmp[@]}"/* genesis@127.0.0.1:/tmp/
run_ssh "echo genesis | sudo -S true 2>/dev/null; sudo bootc usr-overlay >/dev/null 2>&1 || true; for b in ${bins[*]}; do sudo install -m 0755 /tmp/\$b /usr/bin/\$b; done; sudo systemctl daemon-reload; sudo systemctl try-restart genesis-probe.service genesis-firstrun.service 2>/dev/null; systemctl --user try-restart genesis-permd.service genesis-agentd.service 2>/dev/null; echo pushed: ${bins[*]}; genesis-agentd --version 2>/dev/null"
rm -rf "$tmp"
