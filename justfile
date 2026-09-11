# Genesis Phase 0 — prototype tasks. Run `just` to list.

set shell := ["bash", "-euo", "pipefail", "-c"]

models_dir := env_var_or_default("GENESIS_MODELS", env_var("HOME") + "/.local/share/genesis/models")
run_dir    := justfile_directory() + "/prototype/run"
pack       := env_var_or_default("GENESIS_PACK", "mac-36")
endpoint   := "http://127.0.0.1:8080"

default:
    @just --list

# Install prototype dependencies for the current OS
setup:
    #!/usr/bin/env bash
    case "$(uname -s)" in
      Darwin) prototype/setup-macos.sh ;;
      Linux)  prototype/setup-fedora.sh ;;
      *) echo "unsupported OS"; exit 1 ;;
    esac

# Download the model pack (default: mac-36). Override with GENESIS_PACK=gpu-24
models:
    GENESIS_MODELS="{{models_dir}}" prototype/models.sh packs/{{pack}}.json

# Start llama-swap in the background on 127.0.0.1:8080
up:
    mkdir -p "{{run_dir}}"
    GENESIS_MODELS="{{models_dir}}" prototype/render-router.sh > "{{run_dir}}/router.yaml"
    @if pgrep -f "llama-swap.*router.yaml" >/dev/null; then echo "llama-swap already running"; exit 0; fi
    nohup llama-swap --config "{{run_dir}}/router.yaml" --listen 127.0.0.1:8080 > "{{run_dir}}/llama-swap.log" 2>&1 &
    @sleep 1; echo "llama-swap started; log: {{run_dir}}/llama-swap.log"; echo "UI: {{endpoint}}/ui"

# Stop llama-swap and any llama-server children
down:
    -pkill -f "llama-swap.*router.yaml"
    -pkill -f "llama-server"
    @echo stopped

# Tail the router log
logs:
    tail -f "{{run_dir}}/llama-swap.log"

# Which models are loaded right now
status:
    @curl -s {{endpoint}}/running | python3 -m json.tool || echo "router not running"

# Check the endpoint and send one request to each model
smoke:
    prototype/smoke.sh {{endpoint}}

# Open Goose against the local endpoint (model: code)
goose model="code":
    GOOSE_PROVIDER=openai OPENAI_HOST={{endpoint}} OPENAI_API_KEY=local GOOSE_MODEL={{model}} goose session

# First feel without downloads: Goose against the Ollama models already installed
quick model="qwen3-coder:30b":
    GOOSE_PROVIDER=ollama OLLAMA_HOST=http://127.0.0.1:11434 GOOSE_MODEL={{model}} goose session

# Free disk: list the pack on disk with sizes
du:
    du -sh "{{models_dir}}"/* 2>/dev/null || echo "no models yet"

# Headless agent run against the local endpoint, e.g. just run "list the 5 largest files here"
run text model="code":
    GOOSE_PROVIDER=openai OPENAI_HOST={{endpoint}} OPENAI_API_KEY=local GOOSE_MODEL={{model}} GOOSE_MODE=auto goose run --no-session --text "{{text}}"

# Run a Goose recipe from prototype/goose/, e.g. just recipe kanban
recipe name:
    GOOSE_PROVIDER=openai OPENAI_HOST={{endpoint}} OPENAI_API_KEY=local GOOSE_MODEL=code GOOSE_MODE=auto goose run --no-session --recipe prototype/goose/{{name}}.yaml

# Build the Genesis OCI image (x86_64) with Docker
image tag="genesis:0.1":
    docker buildx build -f Containerfile --platform linux/amd64 --build-arg BOOTC_LINT=skip --load -t {{tag}} .
    docker run --rm --platform linux/amd64 {{tag}} genesis-image-check

# Build an installer ISO from the image (needs a Linux/Docker host with loop devices)
iso tag="genesis:0.1":
    mkdir -p iso/output
    docker run --rm --privileged --platform linux/amd64 -v "$PWD/iso/output:/output" -v "$PWD/iso/config.toml:/config.toml:ro" -v "$PWD/iso/defs/genesis-44.yaml:/usr/share/bootc-image-builder/defs/genesis-44.yaml:ro" -v /var/lib/containers/storage:/var/lib/containers/storage quay.io/centos-bootc/bootc-image-builder:latest --type anaconda-iso --rootfs btrfs --config /config.toml --local {{tag}}

# Build a qcow2 disk image for QEMU boot tests
qcow2 tag="genesis:0.1":
    mkdir -p iso/output
    docker run --rm --privileged --platform linux/amd64 -v "$PWD/iso/output:/output" -v "$PWD/iso/config.toml:/config.toml:ro" -v "$PWD/iso/defs/genesis-44.yaml:/usr/share/bootc-image-builder/defs/genesis-44.yaml:ro" -v /var/lib/containers/storage:/var/lib/containers/storage quay.io/centos-bootc/bootc-image-builder:latest --type qcow2 --rootfs btrfs --config /config.toml --local {{tag}}

# Download the latest CI-built qcow2 and ISO into iso/output/
fetch-images:
    mkdir -p iso/output
    gh release download --repo matijagorsek/genesis --pattern "disk.qcow2" --pattern "*.iso" --dir iso/output --clobber
    ls -la iso/output

# Boot the qcow2 in QEMU (x86_64 emulated on Apple Silicon: slow, but works). Login genesis/genesis. Ctrl-a x to quit.
boot disk="iso/output/disk.qcow2":
    cp -n /opt/homebrew/share/qemu/edk2-x86_64-code.fd iso/output/ovmf-code.fd 2>/dev/null || true
    qemu-system-x86_64 -machine q35 -cpu max -smp 4 -m 6144 \
      -drive if=pflash,format=raw,readonly=on,file=iso/output/ovmf-code.fd \
      -drive file={{disk}},if=virtio,format=qcow2 \
      -device virtio-vga -display default,show-cursor=on \
      -netdev user,id=n0,hostfwd=tcp::2222-:22 -device virtio-net-pci,netdev=n0 \
      -serial mon:stdio

# Boot the qcow2 headless on the serial console only
boot-serial disk="iso/output/disk.qcow2":
    cp -n /opt/homebrew/share/qemu/edk2-x86_64-code.fd iso/output/ovmf-code.fd 2>/dev/null || true
    qemu-system-x86_64 -machine q35 -cpu max -smp 4 -m 4096 -nographic \
      -drive if=pflash,format=raw,readonly=on,file=iso/output/ovmf-code.fd \
      -drive file={{disk}},if=virtio,format=qcow2
