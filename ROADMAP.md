# Roadmap

What Genesis is: a complete desktop that can build things for you, with the AI on your own machine.
What it is not: a chat app with a window manager. The order below is the order we believe in; dates
are not promises. The decision log (`docs/decisions.md`) records what actually happened and why.

## Now (0.2, shipping on the stable channel)

- Real hardware. Every claim so far is verified in virtual machines and on one phone. The first
  installs on real laptops decide what comes next. File a "Hardware report" issue, good or bad.
- Babel, the IDE, in daily use: what is missing when you write real code with the maker beside you.
- The phone app on a store, so nobody needs a cable.
- The weekly canary is now a gate: the stable tags only move when the last canary was green. Keeping
  it green for a few weeks in a row is the test of that.

## Next

- JavaScript/TypeScript language tooling in the box (Python, Rust, Go and C/C++ are there).
- A better small coder model for CPU-only machines, whenever one appears; the packs are data, not code.
  The weekly ten-make evaluation is the yardstick.
- The maker handing hard steps to Claude Code with the user's own account, when the user chose that.
- German and Slovenian beyond the maker, palette and settings: the first-run wizard and Babel's welcome.

## Later

- More phones: iOS companion, if Apple's limits leave enough to be useful.
- Voice replies in more languages.
- A "recipes" exchange: what people made, replayable on your machine, with the same permission cards.

## Not planned

- Any cloud component on the path between you and your machine. No relay, no account, no telemetry.
- Our own editor core, browser engine or model runtime. Genesis stands on Fedora, KDE, Code-OSS,
  llama.cpp, whisper.cpp and KDE Connect, and says so.

## How to help

Good first issues are labelled `good first issue`. Templates, dictionaries, packs for hardware we
do not have, and hardware reports are where a first contribution lands well. See CONTRIBUTING.md.
