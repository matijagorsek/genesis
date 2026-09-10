# Phase 0 prototype

One local OpenAI-compatible endpoint (`http://127.0.0.1:8080/v1`) fronting three models,
plus Goose as the agent. No Genesis code yet; this is glue around llama-swap, llama.cpp
and Goose to answer "does the core loop feel good?".

| Role  | Model (pack mac-36)              | Residency                 |
|-------|----------------------------------|---------------------------|
| fast  | Qwen3.5-4B Q8, with vision       | always loaded             |
| code  | Qwen3.8-27B Q4_K_M, with vision  | on demand, 10 min TTL     |
| embed | Qwen3-Embedding-0.6B Q8          | always loaded             |

Routing in Phase 0 is by the `model` field only (`fast`, `code`, `embed`, or an alias).
The intent-based router is Phase 1 work in `src/gateway`.

## Files

- `setup-macos.sh`, `setup-fedora.sh` — install llama.cpp, llama-swap, Goose, hf.
- `models.sh` — download a pack from `packs/*.json` with `hf download` (resumable).
- `router.yaml` — llama-swap template; `render-router.sh` fills in file paths.
- `smoke.sh` — one request per role with tok/s.
- `goose/` — recipes for the three Phase 0 tasks.

## Three tasks that decide Phase 0

Run each with `just goose` and note: did it finish, how many retries, did it loop.

1. **Shell**: "Find which process is using the most memory and show me its command line."
2. **Repo edit**: in a small existing repo, "Add a `--json` flag to the CLI and update the README."
3. **App**: "Make a small kanban board web app that reads tasks from ~/todo.md, with drag and drop."

Record results in `docs/phase0-log.md`.

## Known gaps

- Goose runs unsandboxed here. Bubblewrap sandboxing is Linux-only and lands with `permd` in Phase 1.
- Tool-call formatting on Q4 27B models can degrade; if task 2 or 3 loops, try `GENESIS_PACK=mac-36-q5`.
- llama-swap group semantics (`persistent`, `exclusive`) should be re-checked against the installed version's README.
