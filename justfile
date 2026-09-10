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
