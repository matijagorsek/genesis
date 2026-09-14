# Security

Genesis runs language models and an agent on your machine. The parts that matter most for security:

- The permission broker (`genesis-permd`) classifies every action the agent wants to take and asks before anything
  outside the project, with a hash-chained audit log. The policy is in `system_files/usr/share/genesis/policy.d/`.
- Commands run in a bubblewrap sandbox. Web pages the agent reads taint the session so system-level actions ask again.
- Images are signed (sigstore keyless and a release key); installed systems refuse unsigned updates.
- Nothing leaves the machine by default. Claude Code is optional, separate, and signs in with your own account.

To report a vulnerability, open a private security advisory on GitHub (Security tab → Report a vulnerability)
rather than a public issue. Please include the Genesis version (`cat /usr/lib/os-release`) and steps to reproduce.
