#!/usr/bin/env bash
# Phase 0 dependencies on Fedora 43/44 (also works inside a distrobox on Bluefin/Aurora).
# GPU: uses the Vulkan build of llama.cpp from Fedora's repo. CUDA/ROCm come via containers in Phase 1.
set -euo pipefail

need() { command -v "$1" >/dev/null 2>&1; }

echo "== dnf packages"
sudo dnf install -y llama-cpp just python3-pip curl tar bubblewrap vulkan-tools 2>/dev/null || \
  echo "llama-cpp not in this repo; install from https://github.com/ggml-org/llama.cpp/releases (vulkan build) into ~/.local/bin"

echo "== huggingface cli"
need hf || pip3 install --user -q "huggingface_hub[cli]"

echo "== llama-swap"
if ! need llama-swap; then
  tag=$(curl -fsSL https://api.github.com/repos/mostlygeek/llama-swap/releases/latest | python3 -c 'import sys,json;print(json.load(sys.stdin)["tag_name"])')
  ver=${tag#v}
  arch=$(uname -m); case "$arch" in x86_64) a=amd64;; aarch64) a=arm64;; esac
  tmp=$(mktemp -d)
  curl -fsSL -o "$tmp/ls.tgz" "https://github.com/mostlygeek/llama-swap/releases/download/${tag}/llama-swap_${ver}_linux_${a}.tar.gz"
  tar -xzf "$tmp/ls.tgz" -C "$tmp"
  mkdir -p ~/.local/bin && install -m 0755 "$tmp/llama-swap" ~/.local/bin/llama-swap
  rm -rf "$tmp"
fi

echo "== goose"
if ! need goose; then
  curl -fsSL https://github.com/block/goose/releases/download/stable/download_cli.sh | CONFIGURE=false bash
fi

echo "== gpu"
vulkaninfo --summary 2>/dev/null | grep -E "deviceName|deviceType" | head -4 || echo "no vulkan device visible"

echo
echo "next: GENESIS_PACK=gpu-24 just models"
