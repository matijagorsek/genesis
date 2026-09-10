# Phase 0 log

Record every run of the three tasks. Honest notes beat benchmarks here.

**Phase 0 verdict (2026-09-10): all three tasks pass on the first run with a local 27B at Q4.** The core loop is good enough to build on. Main finding: the agent takes system-scope actions (package installs with policy overrides) without asking, so the permission broker is the first Phase 1 component, not the router.

| Date | Task | Model | Hardware | Finished? | Retries / loops | Notes |
|------|------|-------|----------|-----------|-----------------|-------|
| 2026-09-10 | shell | Qwen3.8-27B UD-Q4_K_M via llama-swap, Goose 1.50 | M4 Max 36 GB | yes | 0 | 2 shell calls, noticed macOS `ps -m` ordering was inconsistent and re-sorted by RSS; clean tabular answer. ~16 tok/s steady, 17 s cold load. |
| 2026-09-10 | repo edit | same | same | yes | 0 | Fixture: 3-file Python CLI. Added `--json`, README section, new test; ran pytest, 2 passed. Verified both modes manually. **Escalated on its own:** pytest missing, hit PEP 668, then used `pip3 install --user --break-system-packages` without asking. Evidence for gating S1 actions in permd. |
| 2026-09-10 | kanban app | same | same | yes | 0 loops, 1 self-fixed tsc error | 24 tool calls. Vite vanilla-ts scaffold, dev-server middleware for GET/POST /api/todo, drag and drop with reorder, debounced write-back, light/dark CSS. Ran tsc and `npm run build`, tested middleware end to end with a mock harness and restored todo.md. Verified after: dev server up, API returns 4 tasks with correct statuses. |


## Endpoint numbers (2026-09-10, M4 Max 36 GB, Metal)

| model | cold load | steady tok/s | notes |
|---|---|---|---|
| fast Qwen3.5-4B Q8 | 3.5 s | ~56 | reasoning off, 16k ctx |
| code Qwen3.8-27B UD-Q4_K_M | 17 s | ~16.5 | reasoning auto, 4k budget, 32k ctx |
| embed Qwen3-Embedding-0.6B Q8 | <1 s | 1024 dims | |

All three resident together. 65k ctx on the coder overflowed Metal memory (Compute error); 32k fits.
