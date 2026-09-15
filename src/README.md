# src: the Genesis daemons and surfaces (Rust, C++/QML)

Cargo workspace; `cargo test` runs every test that needs no desktop.

| crate            | what it is |
|------------------|------------|
| genesis-permd    | permission broker: tiers, modes, taint, keyed audit chain, policy files, the adversarial battery |
| genesis-agentd   | the maker: sessions, tools, sandbox (bubblewrap), previews, gallery, notices, voice, vision, phone and index endpoints; serves the workspace, palette and Settings pages from `ui/` |
| genesis-firstrun | first-run wizard: hardware profile, packs, resumable verified downloads, router config, phone step |
| genesis-probe    | hardware probe: GPUs, memory, disk, which packs fit |
| genesis-txd      | transactions and undo: pre-images, rollback, history |
| genesis-ask      | `ask` in the terminal and Ctrl+G |
| genesis-krunner  | KRunner plugin: "Ask Genesis" from the launcher |
| genesis-window   | Qt WebEngine window for the pages (C++) |
| genesis-kcm      | the Genesis module in System Settings (QML) |

The permission policy that ships is generated from `genesis-permd` (`policy dump`) into
`system_files/usr/share/genesis/policy.d/00-default.toml`; CI fails when the two drift.
