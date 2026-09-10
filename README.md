# Genesis

A Linux distribution where several local LLMs are a first-class OS service and an
agent can build apps, browse, install software and reconfigure the machine. Everything
local by default.

Design brief with all decisions and evidence: [docs/genesis-brief.html](docs/genesis-brief.html).
Short decision log: [docs/decisions.md](docs/decisions.md).

## Status: Phase 0 prototype

Goal of Phase 0 is one question: does the core loop feel good enough on real hardware?
The core loop is a small always-hot model plus a 27B coder behind one local endpoint,
driven by an agent that can run commands and edit files.

The prototype runs natively on macOS (Apple Silicon, Metal) and on Fedora. The Linux-only
parts (systemd units, bubblewrap sandbox, bootc image) live under `system_files/` and are
exercised in Phase 1.

## Quick start (macOS)

```bash
just setup      # brew: llama.cpp, llama-swap, goose, just, hf
just models     # downloads the "mac-36" pack (~22 GB) into ~/.local/share/genesis/models
just up         # starts llama-swap on http://127.0.0.1:8080
just smoke      # checks the endpoint and routes one request to each model
just goose      # opens Goose pointed at the local endpoint
```

`just quick` skips the downloads and points Goose at the Ollama models already on this
machine (qwen3-coder:30b), useful for a first feel before the pack finishes downloading.

## Layout

```
prototype/        Phase 0: llama-swap config, setup scripts, smoke test, Goose config
packs/            model pack definitions per hardware tier (JSON)
system_files/     files that ship in the OS image: systemd units, udev, sysusers, /etc/genesis
src/              Rust daemons (gateway, agentd, permd, txd, indexd) — Phase 1
docs/             brief and decision log
Containerfile     bootc image recipe — Phase 1 skeleton, not yet built
```

## Phase 0 exit criteria

- Ask anything, run a command, and edit a repo from one entry point, fully local.
- The 4B model answers in under a second; the 27B loads on demand and unloads after idle.
- Three real tasks succeed with Goose: a shell command, a repo edit, a small web app.
