# The first install on a real machine

Everything in Genesis so far ran in CPU virtual machines and on one phone. This is the checklist for the
first install on real hardware with a GPU: what to do, what to look at, and what to write down. Twenty
minutes of notes here answer more open questions than a week of CI. Paste the results into issue #3.

## Before

- Download the ISO from the [download page](https://matijagorsek.github.io/genesis/) and write it to a
  USB stick (Fedora Media Writer, or `dd`). The ISO is about 6.3 GB, so the stick must be **8 GB or
  larger**; 4 GB is not enough. Everything the install needs is on the ISO, so no network is needed
  until the first boot. There is one ISO. With an NVIDIA card you install it, and then switch to the
  NVIDIA flavour (last section) — but only if the card is Turing or newer (GTX 16xx, RTX 20xx and later).
- The target disk: the system is about 13 GB installed, and bootc keeps the previous version alongside it
  so an upgrade can be rolled back. On top of that comes the model pack, which the wizard sizes to the
  machine: 3.8 GB for the tiny pack, 12.5 GB for CPU-only, 20–50 GB for an 8–24 GB GPU, 130 GB for a
  48 GB card (the `disk_gb` field in `packs/*.json`). 64 GB is a sensible floor; a 32 GB disk only works
  with the tiny or CPU pack.
- Note the machine: model, CPU, RAM, GPU and its memory, Wi-Fi chip if you know it.
- Secure Boot: note whether it is on. The kernel is signed with the Universal Blue key, which a fresh
  machine does not know. If the first boot stops with a signature error, turn Secure Boot off in the
  firmware for now and note it; enrolling the key (`ujust enroll-secure-boot-key`, password
  `universalblue`, confirm at the next boot) is the proper way and needed for the NVIDIA driver.
- The installer asks you to create your own account. There is no default account on an installed machine.

## Install

| Step | Look at | Write down |
|---|---|---|
| Boot the stick | Does the boot menu appear, does the installer start | Any black screen, and at which point |
| Installer | Disk selection, language, keyboard | Minutes from start to reboot |
| First boot | Plymouth, the login screen, the desktop | Seconds from power button to login |

## First run

| Step | Look at | Write down |
|---|---|---|
| The wizard's hardware page | Does it name the GPU and its memory correctly | What it showed |
| The pack it recommends | Is it the one you expected for this GPU | Pack name |
| Download | The small model first, then the rest | Seconds until "the maker is ready"; total minutes |
| The first make (pick one of the three offers) | Steps, preview | Did it finish, how long |

## One command, for the report

When the desktop is up and the first make has run:

    sudo genesis-report > report.md

That collects the image version, the hardware as the wizard saw it, which pack it chose and why the
others did not fit, whether the models are really on the GPU, how many tokens a second each model
answers at, and the self-check. It reads nothing in your home folder and sends nothing anywhere: it
prints, you read it, and you decide what to do with it. The end of the file has the desktop checklist
to fill in by hand. Attach it to issue #3.

## The numbers that matter

Open **Settings, On this machine** after the first reply: it shows tokens per second and the model.

| Measure | Where | Value |
|---|---|---|
| Tokens per second, small model | Settings, On this machine | |
| Tokens per second, coder model (GPU packs) | same, after a make | |
| Is the GPU used | `journalctl -u genesis-router \| grep -iE "vulkan|offloaded"` shows the device and "offloaded N/N layers"; `sudo genesis-image-check` says whether the model service can open the GPU | |
| Self-check | `sudo genesis-image-check` in Konsole | ok count, every FAIL line |

## The desktop basics

Tick or note: Wi-Fi connects · Bluetooth pairs · sound out and mic in · suspend and resume · brightness
and volume keys · external monitor · the touchpad gestures · day/night with `Meta+Shift+T`.

## The Genesis things

- `Meta+Space`, ask something; `Meta+Shift+V`, say something.
- "Show me Genesis" from the app menu: does the minute run through.
- `Meta+Shift+A`, drag over a window, ask what it shows (the vision model).
- Chat: attach a PDF by path and ask about it.
- Babel: type in a Python file and wait for a suggestion; `Ctrl+I` on a selection.
- Pair the phone (Settings, Phone app) if it is at hand.

## No network at first boot

The wizard is a maximized window; the panel with the network icon stays visible at the bottom. Connect
to Wi-Fi there first: the wizard says so when the machine is offline.

## NVIDIA

**Only for Turing cards and newer** (GTX 16xx, RTX 20xx, and later). The flavour carries NVIDIA's open
kernel modules, which do not support Maxwell or Pascal — a GTX 9xx or 10xx will not get a working driver
from it, so leave those machines on the standard image and let the models run on the CPU.

After the install, in Konsole:

    sudo bootc switch ghcr.io/matijagorsek/genesis:stable-nvidia
    systemctl reboot

Then enrol the Secure Boot key if Secure Boot is on (see above), and open "Set up models" again so the
pack is chosen with the driver loaded. This switch has never been done on real hardware: note every step.

## If something breaks

- A black screen at boot: add `nomodeset` at the boot menu (press `e`), note that it was needed.
- The wizard recommends the wrong pack: note the GPU lines from `/etc/genesis/profile.json`.
- The model service does not start: `systemctl status genesis-router` and the last lines of
  `journalctl -u genesis-router`.
- Anything else: `sudo genesis-image-check > check.txt` and attach it.
