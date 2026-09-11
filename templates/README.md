# Project templates

Shipped at `/usr/share/genesis/templates`. The agent's `scaffold` tool copies one into `~/Projects/<name>`,
replacing `{{name}}` in file contents and `__name__` in file names. `genesis.json` tells Genesis how to run
a preview (`dev`) and how to run the finished thing (`run`). Templates are dependency-light on purpose:
Python 3 is on the image; Node/Rust toolchains come via distrobox when a task needs them.
