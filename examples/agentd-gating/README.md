# genesis-agentd: two real runs on the local 27B (2026-09-11, M4 Max, Qwen3.8-27B via llama-swap)

Unedited event logs from `genesis-agentd run --mode auto_edit`.

- `run1-project-task.events`: count lines/words, write a summary file, verify. Six turns; every tool
  call gated by the permission broker (R0 reads, W1 write) and allowed.
- `run2-gated-install.events`: the Phase 0 escalation, on purpose. The model runs
  `pip3 install --break-system-packages pytest`; the broker classifies it S1 and blocks it pending
  permission; the headless run denies it; the model does not retry and proposes a virtualenv instead.

Compare with `examples/kanban-phase0/agent-transcript.txt`, where the same action ran unasked.
