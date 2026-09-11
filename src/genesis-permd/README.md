# genesis-permd

The permission broker. Every tool call the agent wants to make is described as an *intent*
(tool, command, paths read and written, network, privilege) and sent here first. The broker
classifies it into a tier, applies the session mode, applies taint, writes an audit entry, and
answers **allow**, **allow with snapshot**, **prompt** or **deny**.

Why it exists: in Phase 0 the agent hit a Python policy error and worked around it with
`pip --break-system-packages` without asking. That call is `S1` here and prompts.

| tier | meaning | assist | auto_edit | autonomous |
|---|---|---|---|---|
| R0 | read inside project, list windows, query index | allow | allow | allow |
| R1 | fetch a URL, web search | prompt | allow | allow |
| W1 | edit files in the project, run tests, commit | prompt | allow | allow |
| W2 | write elsewhere in $HOME, change settings, drive the desktop | prompt | prompt | allow + snapshot |
| S1 | user-scope installs, policy overrides, `systemd --user` | prompt | prompt | allow + snapshot |
| S2 | bootc, dnf, flatpak system, NetworkManager, sudo | prompt | prompt | prompt |
| X  | secrets | deny | deny | deny |
| N  | wipe home, disk ops, disable Genesis services, send messages | deny | deny | deny |

Once a session has read untrusted content (web page, download), anything at W2 or above prompts
regardless of mode. Denials never relax.

Policy is TOML: defaults ship in `/usr/share/genesis/policy.d/00-default.toml`; overrides in
`/etc/genesis/policy.d/*.toml` and `~/.config/genesis/policy.toml`, applied in order.

    genesis-permd check --cmd "pip3 install --break-system-packages pytest"
    genesis-permd check --mode autonomous --tainted --cmd "flatpak install --user org.x.App"
    genesis-permd check --intent intent.json --json
    genesis-permd policy table
    genesis-permd audit verify
    genesis-permd serve            # D-Bus org.genesis.Permission1 on the session bus (Linux)

Audit log: `~/.local/state/genesis/audit.jsonl`, one JSON entry per line, each carrying the SHA-256
of the previous line.
