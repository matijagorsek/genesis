# Security

Genesis runs language models and an agent on your machine. The parts that matter most for security:

- The permission broker (`genesis-permd`) classifies every action the agent wants to take and asks before anything
  outside the project, with a hash-chained audit log. The policy is in `system_files/usr/share/genesis/policy.d/`.
- Commands run in a bubblewrap sandbox. Web pages the agent reads taint the session so system-level actions ask again.
- Images are signed (sigstore keyless and a release key); installed systems refuse unsigned updates.
- Nothing leaves the machine by default. Claude Code is optional, separate, and signs in with your own account.

To report a vulnerability, open a private security advisory on GitHub (Security tab → Report a vulnerability)
rather than a public issue. Please include the Genesis version (`cat /usr/lib/os-release`) and steps to reproduce.

## What has been reviewed

An independent adversarial review of the daemons, units, scripts and supply chain was done on
2026-09-15 (decisions 70 and 72 in `docs/decisions.md`). The local HTTP daemons answer only their own
pages and local helpers (same-origin checks plus a per-boot token), request bodies are capped, previews
are classified by the command they really run, paths are matched on their canonical form, the audit
chain is keyed, base images are pinned by digest, upstream binaries are verified by checksum, and CI
runs a RustSec audit. The shell classifier remains heuristic: the sandbox is the boundary, the policy is
the second line. Reports of anything that gets past either are welcome, privately, as described above.

## MCP tool servers

A tool server declared in `/usr/share/genesis/mcp` or `~/.config/genesis/mcp.json` runs as the user,
without the shell sandbox: it is the user's own choice of program, like anything they start from a
terminal. What stays in place is the permission broker: every call is classified by the tier the
server's configuration declares (`read`, `write`, `network`, `system`, `never`), per tool when listed,
and answered per session mode, so a `system`-tier tool asks before it runs unless the user chose
Hands-off. Tool results are model input, never instructions to Genesis. A server that needs the
network should say so with the `network` tier; the network badge then shows the job used it.

