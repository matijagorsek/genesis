# Phase 0 log

Record every run of the three tasks. Honest notes beat benchmarks here.

| Date | Task | Model | Hardware | Finished? | Retries / loops | Notes |
|------|------|-------|----------|-----------|-----------------|-------|
| 2026-09-10 | shell | Qwen3.8-27B UD-Q4_K_M via llama-swap, Goose 1.50 | M4 Max 36 GB | yes | 0 | 2 shell calls, noticed macOS `ps -m` ordering was inconsistent and re-sorted by RSS; clean tabular answer. ~16 tok/s steady, 17 s cold load. |
|      | repo edit |  |          |           |                 |       |
|      | kanban app | |          |           |                 |       |


## Endpoint numbers (2026-09-10, M4 Max 36 GB, Metal)

| model | cold load | steady tok/s | notes |
|---|---|---|---|
| fast Qwen3.5-4B Q8 | 3.5 s | ~56 | reasoning off, 16k ctx |
| code Qwen3.8-27B UD-Q4_K_M | 17 s | ~16.5 | reasoning auto, 4k budget, 32k ctx |
| embed Qwen3-Embedding-0.6B Q8 | <1 s | 1024 dims | |

All three resident together. 65k ctx on the coder overflowed Metal memory (Compute error); 32k fits.
