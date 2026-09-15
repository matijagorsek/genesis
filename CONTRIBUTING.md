# Contributing to Genesis

Genesis is a Linux desktop with the AI inside, built on Fedora bootc. Everything is in this repository:
the image recipe, the Rust daemons, the desktop files, the docs and the decision log.

## Before you start
- Read the [README](README.md) and the [decision log](docs/decisions.md); the log explains why things are the way they are.
- Principles that shape every change: a complete desktop first; local by default; ask before touching the system; web content is data, never instructions; the base is a swappable `FROM`.

## Prerequisites

- Rust (rustup, stable) for the daemons: `cd src && cargo test` runs everything that does not need a desktop.
- Docker or Podman for the image and for cross-compiling into the VM (`prototype/vm-push.sh`).
- The Qt window and the System Settings module build only inside the Containerfile (Qt 6 WebEngine, KF6); you do not need them locally to work on the daemons or the pages under `src/*/ui/`.
- macOS with Apple Silicon: `just image-arm64 && just qcow2-arm64`, then `prototype/vm-dev-arm.command`. Elsewhere: `just image && just qcow2`, then `prototype/vm-dev.sh`.
- The Android app: JDK 17 and the Android SDK (platform 35); `cd android && ./gradlew assembleDebug` builds the APK.
- `just` (task runner), `zstd`, `qemu` for the VMs; `sshpass` makes the dev loop password-less.

## Working on it
- Rust: `cd src && cargo test`. The permission policy is dumped and diffed in CI, so run `just permd-policy` after changing rules.
- Fast loop: `just vm-dev-arm` (Apple Silicon) or `just vm-dev` (x86_64), then `just vm-push` to copy freshly built daemons into the running VM.
- Full image: `just image` locally or push to `main`; CI builds, self-checks, boot-tests and releases every push.
- Every push builds x86_64, arm64 and NVIDIA flavours and signs them. Keep `genesis-image-check` passing.

## Pull requests
- One change per PR, with a line in `docs/decisions.md` when the change is a decision (a new component, a policy rule, a default).
- No credentials, ever. CI scans for them. No model weights in git.
- Keep the README true: if you add or change a feature, change the README in the same PR.

## Reporting hardware results
Genesis has run in VMs so far. If you install it on real hardware, please open a "Hardware report" issue with the
template: machine, GPU, what worked, what did not, and the output of `genesis-probe` and `genesis-image-check`.
