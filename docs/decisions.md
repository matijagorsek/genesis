# Decision log

Short form. Evidence and alternatives are in [genesis-brief.html](genesis-brief.html).

| # | Date | Decision | Status |
|---|------|----------|--------|
| 1 | 2026-09-10 | Base: Fedora bootc, forked from Universal Blue Aurora (KDE). Re-examined against from-scratch and independent routes on 2026-09-11, see [base-decision.md](base-decision.md). Ubuntu 26.04 LTS is the runner-up. | reconfirmed |
| 2 | 2026-09-10 | Inference: llama.cpp `llama-server` behind llama-swap, one OpenAI-compatible endpoint on localhost. Ollama only as a compatibility shim. | accepted |
| 3 | 2026-09-10 | GPU: Vulkan on the host for all vendors; `genesis-nvidia` image flavour with open kernel modules; CUDA/ROCm/SYCL via ramalama containers. | accepted |
| 4 | 2026-09-10 | Agent core: embed Goose (Rust, Apache 2.0, MCP-native). OpenHands SDK optional in a builder container. | accepted |
| 5 | 2026-09-10 | Default models: Qwen3.5 / 3.6 / 3.8 family (Apache 2.0). Packs per hardware tier, downloaded at first boot as OCI artifacts. | accepted |
| 6 | 2026-09-10 | Licensing: three tiers (main / restricted / excluded). Default packs only from main. | accepted |
| 7 | 2026-09-10 | Sandbox tiers T0-T4: bubblewrap → containers → confirmation + snapshot → image rebuild → never. Agent runs as its own user. | accepted |
| 8 | 2026-09-10 | Stack: Rust for daemons and UI; Python only in containers; TypeScript only for the browser extension. | accepted |
| 9 | 2026-09-10 | v1 surfaces: launcher, agent workspace, model manager, terminal, select-to-act. Browser sidebar and voice in v2. No always-on screen memory. | accepted |
| 10 | 2026-09-10 | Phase 0 runs natively on the dev Mac first (Metal), Fedora VM second. Linux-only parts deferred to Phase 1. | accepted |
| 11 | 2026-09-11 | llama.cpp ships as the upstream Vulkan release build under `/usr/lib/genesis/llama.cpp`, not Fedora's `llama-cpp` package (months behind, no Vulkan backend, drags the full ROCm stack in, +2.5 GB). CUDA/ROCm builds via ramalama containers. | accepted |
| 12 | 2026-09-11 | First Genesis image built: `genesis:0.1` from `ghcr.io/ublue-os/aurora:stable`, 3.80 GB (base 3.70). `bootc container lint` is strict in CI, skipped in emulated local builds. | done |
| 13 | 2026-09-11 | First boot. Release v0.1.9: CI boot test green under KVM; the same image boots on the dev Mac under QEMU emulation in 90 s with `genesis-router.socket` listening. Pipeline: strict lint, self-check, push, qcow2, boot test, split release; ISO in its own job. | done |
| 14 | 2026-09-11 | Release v0.1.10: first fully green pipeline, both jobs. Installer ISO (5.2 GB, split) and qcow2 attached. Hostname still `aurora` (open). | done |
