# The first install on a real machine

Everything in Genesis so far ran in CPU virtual machines and on one phone. This is the checklist for the
first install on real hardware with a GPU: what to do, what to look at, and what to write down. Twenty
minutes of notes here answer more open questions than a week of CI. Paste the results into issue #3.

## Before

- Download the ISO from the [download page](https://matijagorsek.github.io/genesis/) (NVIDIA card: the
  NVIDIA flavour) and write it to a USB stick (Fedora Media Writer, or `dd`).
- Note the machine: model, CPU, RAM, GPU and its memory, Wi-Fi chip if you know it.
- Secure Boot: leave it as it is and note whether it was on.

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

## The numbers that matter

Open **Settings, On this machine** after the first reply: it shows tokens per second and the model.

| Measure | Where | Value |
|---|---|---|
| Tokens per second, small model | Settings, On this machine | |
| Tokens per second, coder model (GPU packs) | same, after a make | |
| Is the GPU used | `genesis-image-check` prints the Vulkan device; `nvidia-smi` or `radeontop` during a make | |
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

## If something breaks

- A black screen at boot: add `nomodeset` at the boot menu (press `e`), note that it was needed.
- The wizard recommends the wrong pack: note the GPU line from `/run/genesis/profile.json`.
- The model service does not start: `systemctl status genesis-router` and the last lines of
  `journalctl -u genesis-router`.
- Anything else: `sudo genesis-image-check > check.txt` and attach it.
