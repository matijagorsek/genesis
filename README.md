# Genesis

**A complete Linux desktop that can build things for you, with AI that runs on your machine.**

Genesis is its own distribution, derived from Fedora bootc (Universal Blue Aurora, KDE Plasma 6).
It ships a full desktop: browser, files, terminal, documents, photos, music, video, calculator,
app store, office, mail. On top of that it has a **maker**: press `Meta+Space`, type or say what you
want made, and it builds it, shows it live, and installs it to your app menu. It can drive its own
private browser, listen to your voice and talk back. Nothing leaves the computer. Every change is
permission-gated and undoable.

| | |
|---|---|
| ![Desktop](docs/screens/real/desktop-v0.1.47.png) | ![Login](docs/screens/real/login-v0.1.47.png) |
| The desktop, release v0.1.47 booted in QEMU | The login screen, same boot |
| ![Maker](docs/screens/real/maker-v0.1.47.png) | ![App menu](docs/screens/real/menu-v0.1.34.png) |
| The maker, opened after first run | The application menu (v0.1.34 session) |

Preview renderings of the rest (desktop, launcher, maker): [docs/screens/preview](docs/screens/preview).
Design brief: [docs/genesis-brief.html](docs/genesis-brief.html) · Design plan: [docs/genesis-design-plan.html](docs/genesis-design-plan.html)
· Decision log: [docs/decisions.md](docs/decisions.md) · Walkthrough: [docs/genesis-walkthrough.html](docs/genesis-walkthrough.html).

## What works today (v0.1.34 and the next release)

- **Bootable image and installer ISO**, built and boot-tested in CI on every push; releases carry a
  qcow2 disk and the ISO ([Releases](../../releases)).
- **Complete desktop**: KDE Plasma 6 with Firefox, Dolphin, Konsole, Kate, Okular, Gwenview, Haruna,
  Elisa, KCalc, Discover (Flatpak), Ark, Spectacle, System Monitor as RPMs; LibreOffice, Thunderbird,
  VLC, GIMP, Krita preinstalled as Flatpaks once online. Genesis identity: Genesis Dark colour scheme,
  "First Light" wallpaper, Inter and IBM Plex Mono, Papirus icons, a centered floating dock, splash and
  boot watermark, login screen.
- **Local models as an OS service**: llama.cpp (Vulkan) behind llama-swap on `127.0.0.1:8080`,
  one OpenAI-compatible endpoint; model packs per hardware tier (`packs/`), chosen at first run.
- **First-run wizard** (`genesis-firstrun`): detects GPU, RAM and disk (`genesis-probe`), shows every
  pack with "fits your machine" bars for memory and disk, proposes one, downloads it, or lets you set
  models up later.
- **Maker** (`genesis-agentd` + `genesis-window`): "what do you want to make?" → template → files →
  live preview → steer → install as app. Starter prompts, plain-language permission prompts, a
  Keep / Undo toast, and a "Made here" gallery with shareable recipes and one-click export bundles. Templates: static web app,
  Python CLI, Python script.
- **Permission broker** (`genesis-permd`): every tool call classified into tiers (read, write in
  project, write elsewhere, system, secrets, never) and answered per session mode
  (assist / auto-edit / autonomous). Hash-chained audit log. D-Bus `org.genesis.Permission1`.
- **Undo** (`genesis-txd`): snapshots (btrfs or file pre-images) before risky changes;
  `genesis-txd undo`.
- **One door**: `Meta+Space` opens the palette, a small centered window that takes a typed request,
  a spoken one (hold the mic), or the current selection (`Meta+Shift+Space`), and hands it to the maker.
  `ask …` also works in KRunner.
- **Genesis Settings**: a Settings menu entry showing the machine, models on disk, the default autonomy
  mode (Ask / Trusted / Hands-off), voice readiness, the measured tokens per second, the undo history,
  and a Day / Night switch for the whole desktop (also `Meta+Shift+T`).
- **Status widget** in the dock: model loaded, whether any job used the network, sandbox on, one click
  to the maker.
- **Browser control**: the agent drives a headless Chromium with a throw-away profile (open, read,
  click, type, screenshot). Page content is treated as untrusted and taints the session.
- **Terminal**: `ask list the ten biggest files here` prints the command, says what it would touch
  (reads only, changes files here, changes the system), and runs it when you confirm. In bash, type a
  sentence and press `Ctrl+G` to replace it with the command.
- **Voice**: hold-to-talk (whisper.cpp) with a chime and glow the instant you press, and spoken replies
  (Piper), all local.

## Try it

Download the latest `genesis-<version>.qcow2.zst.*.part` files from a release, join and boot:

```bash
cat genesis-*.qcow2.zst.*.part | zstd -d -o disk.qcow2
just boot iso/output/disk.qcow2        # QEMU, EFI; test user genesis / genesis
```

Or install from the ISO in a VM (UTM, virt-manager, VirtualBox) or on a spare machine.

## Develop

Fast loop, no image build:

```bash
just vm-dev            # boots the last downloaded disk with a window and SSH (port 2222); the VM has
                       # internet, so the wizard can download the tiny pack and the maker and `ask` work
just vm-push           # cross-compiles the daemons in Docker and copies them into the running VM
prototype/serial-cmd.py iso/output/dev/serial.sock 'cmd'   # run commands over the serial console
just firstrun          # the wizard, natively on this Mac, at http://127.0.0.1:11510
just workspace         # the maker, natively, at http://127.0.0.1:11520 (needs `just up`)
cd src && cargo test   # Rust workspace: permd, probe, firstrun, agentd, txd, krunner
```

Full image, locally (emulated on Apple Silicon, slow) or in CI (every push to `main`):

```bash
just image             # podman/docker build of the bootc image
just iso               # installer ISO via bootc-image-builder
```

CI (`.github/workflows/build-image.yml`): Rust tests and policy diff → image build (strict
`bootc container lint`, ~45 image self-checks) → push to GHCR → qcow2 → KVM boot test → release.

## Layout

```
Containerfile        the OS image: Rust daemons (cross-compiled), Qt window, whisper.cpp, Piper,
                     llama.cpp, llama-swap, desktop apps, Genesis system files
system_files/        what ships in the image: systemd units, policy, look-and-feel, wallpaper,
                     login defaults, Flatpak preinstall list, image self-check
src/                 Rust workspace: genesis-permd, -probe, -firstrun, -agentd, -txd, -krunner,
                     and genesis-window (C++/Qt WebEngine)
packs/               model packs per hardware tier (JSON, schema.json)
templates/           maker templates (genesis.json: dev/run commands)
iso/                 bootc-image-builder config and distro definition
prototype/           macOS dev scripts, router config, VM boot/screenshot/dev-loop scripts
docs/                brief, decisions, status board, previews, screenshots
examples/            recorded real runs (Phase 0 tasks, gated install, pomodoro app)
```

## Principles

1. A complete desktop first; AI is one integrated capability, not the point of the desktop.
2. Local by default. No account, no cloud, no telemetry.
3. Ask before touching the system; record everything; make it undoable.
4. Web content is data, never instructions.
5. The base is a swappable `FROM`; Genesis is the layer on top.

Genesis is a Fedora Remix and is not endorsed by the Fedora Project.
