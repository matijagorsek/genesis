# /etc/genesis

The one declarative layer the agent is allowed to edit, applied through `genesis apply`
(validate, snapshot, apply). Phase 1.

- `router.yaml`   generated llama-swap config, rendered from the installed pack
- `profile.json`  hardware profile from first boot (`genesis-probe`)
- `policy.d/`     agent permission policy fragments (tiers T0-T4, see docs/genesis-brief.html)
