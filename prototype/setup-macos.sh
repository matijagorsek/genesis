#!/usr/bin/env bash
# Phase 0 dependencies on macOS (Apple Silicon). Idempotent.
set -euo pipefail

need() { command -v "$1" >/dev/null 2>&1; }

echo "== Homebrew packages"
brew list --versions llama.cpp >/dev/null 2>&1 || brew install llama.cpp        # llama-server with Metal
brew list --versions block-goose-cli >/dev/null 2>&1 || brew install block-goose-cli
brew list --versions just >/dev/null 2>&1 || brew install just

echo "== huggingface cli (hf)"
if ! need hf; then
  if need uv; then uv tool install -q "huggingface_hub[cli]"; else pip3 install -q "huggingface_hub[cli]"; fi
fi

echo "== llama-swap (GitHub release binary)"
if ! need llama-swap; then
  tag=$(curl -fsSL https://api.github.com/repos/mostlygeek/llama-swap/releases/latest | python3 -c 'import sys,json;print(json.load(sys.stdin)["tag_name"])')
  ver=${tag#v}
  tmp=$(mktemp -d)
  curl -fsSL -o "$tmp/ls.tgz" "https://github.com/mostlygeek/llama-swap/releases/download/${tag}/llama-swap_${ver}_darwin_arm64.tar.gz"
  tar -xzf "$tmp/ls.tgz" -C "$tmp"
  install -m 0755 "$tmp/llama-swap" /opt/homebrew/bin/llama-swap
  rm -rf "$tmp"
fi

echo "== versions"
llama-server --version 2>&1 | head -1 || true
llama-swap --version 2>&1 | head -1 || true
goose --version || true
hf version 2>/dev/null || hf --version || true

echo
echo "next: just models   (downloads the pack)"
