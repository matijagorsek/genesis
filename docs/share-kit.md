# Share kit

Texts for the first outside eyes. Post them yourself, where you want, when you want; nothing here is
sent by the repository or by Genesis. Each is written to bring back hardware reports, not stars.

## One line

Genesis: a Linux desktop that builds things for you with local models. Fedora bootc, KDE Plasma, a
maker behind a permission broker with undo, phone pairing, everything on your own machine. Looking
for people to try the ISO on real hardware.

## Short post (forums, Mastodon, a Discord)

I have been building Genesis, a desktop OS with local language models as one of its capabilities, not
as the product. Press Meta+Space, say "a pomodoro timer with a bell", and it makes one on your
machine, every step that changes something waiting for your yes, with Keep / Undo at the end. First
run measures your hardware and installs the biggest model pack that fits, from a 3.7 GB pack for 8 GB
laptops upwards. Pair your phone once and you can ask it questions from there; an Android companion app shows your jobs and their permission cards on the phone.

It is Fedora bootc under KDE Plasma, so updates are signed images with rollback. Three flavours:
x86_64, x86_64 with NVIDIA, arm64. Apache-2.0, everything in the repo, including the decision log and
an adversarial security review.

So far it is verified in VMs. I am looking for people to try the ISO on real laptops and file a
hardware report, good or bad.

Download and screenshots: https://matijagorsek.github.io/genesis/
Repo: https://github.com/matijagorsek/genesis

## What to ask for

- Which laptop, which GPU, did the installer finish, did Wi-Fi and suspend work.
- Which pack first run recommended, how many tokens per second the maker got.
- One thing you asked it to make, and whether it did.
- Anything that felt wrong or unsafe. The permission cards are the product; if one was missing, say so.

## Where it tends to land well

- r/linux, r/Fedora, r/LocalLLaMA (hardware-minded, will run it)
- Fediverse with #KDE #Fedora #bootc #LocalLLM
- The KDE and Universal Blue community channels, with credit to what Genesis stands on
- Hacker News, only once the first hardware reports exist

## What not to claim

Not "verified on real hardware" until it is. Not "private" in the absolute; say what leaves the
machine (nothing unless you choose Claude Code with your account) and what the phone bridge does
(same network, no relay).
