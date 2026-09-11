# src — Genesis daemons (Phase 1, Rust)

Cargo workspace. `cd src && cargo test`. Phase 0 proved the loop with off-the-shelf parts; the daemons below add what the OS must own.

Planned crates, per the brief:

| crate           | role                                                                 |
|-----------------|----------------------------------------------------------------------|
| genesis-gateway | OpenAI-compatible endpoint on 127.0.0.1:11500; routing by surface and intent; VRAM plan; cloud opt-in |
| genesis-agentd  | embeds Goose; D-Bus `org.genesis.Agent1`; ACP over Unix socket for editors |
| genesis-permd   | permission broker: tiers, modes, taint, audit chain, D-Bus. **Built.** Keyring broker: later |
| genesis-txd     | transactions: btrfs snapshots, bootc/flatpak rollback, `genesis undo` |
| genesis-indexd  | file watcher + embeddings + sqlite-vec hybrid search                  |
| genesis-probe   | first-boot hardware detection → /etc/genesis/profile.json → pack      |
| genesis-cli     | `genesis ask | do | undo | models | packs | doctor | apply`            |
