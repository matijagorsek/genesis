# How to use Genesis

The user guide. For installing, see the [download page](https://matijagorsek.github.io/genesis/); for the project itself, the [README](README.md).

<p align="center"><img src="docs/screens/latest/04-desktop.png" width="88%" alt="The Genesis desktop"></p>

Genesis is a normal desktop first. Everything below is optional and lives behind one key, Meta+Space
(the Windows or Command key, plus Space). Nothing here needs an account, and nothing leaves the machine
unless you choose it.

## First run

<p align="center"><img src="docs/screens/latest/03-first-run.png" width="88%" alt="First run: what this machine can run, and the recommended pack"></p>

The first time you log in, a window asks three things:

1. **Which models.** Genesis measures the machine and recommends the biggest model pack that fits. Take
   the recommended one; it downloads in the background (about 2 GB for the smallest, more for GPUs).
   You can keep using the desktop meanwhile.
2. **Privacy.** Local only is the default. Leave it.
3. **Your phone.** Optional. Install KDE Connect on the phone, same Wi-Fi, accept the code on both
   screens. You can do this later from Settings.


<p align="center"><img src="docs/screens/phone/01-first-run-pairing.png" width="88%" alt="First run: pair your phone, the same code on both screens"></p>

At the end, pick something to make, or don't. If you skipped models, "Set up models" in the app menu
brings the window back.

## Make something

<p align="center"><img src="docs/screens/fresh-run/03-pomodoro-made-by-the-2B-model.png" width="88%" alt="The maker after building a pomodoro timer: steps on the left, preview on the right"></p>

Press Meta+Space, type what you want in plain words, Enter.

- "a checklist app that saves to a file"
- "a pomodoro timer with a bell"
- "rename the photos in this folder by the date they were taken"

The maker window shows every step as it happens. A preview appears when the thing runs. When it is
done: **Install as app** puts it in the app menu; **Open in browser** for web things; the folder is
under your home, named after the project.

**Steer it:** type in the box at the bottom ("make the button bigger", "add a dark mode") and Enter.

**Speed:** on a machine without a GPU, a step can take minutes. The line under the steps tells you
how long the current one has run. It is working, not stuck.

## When it asks

Before anything that changes your machine outside the project, a card appears with a plain verb:
"Genesis wants to install a package", "wants to write to your Documents". Choose:

- **Allow once**
- **Allow for this project**
- **Not now**

Some things it never does, whatever you answer: read your keys and passwords, wipe disks, change the
base system. Those are refused by the rules, not by the model.

**How much on its own:** Settings > "How much Genesis may do on its own". *Assist* asks before every
change. *Auto edit* (default) edits the project freely and asks for the rest. *Hands-off* also handles
installs and user settings on its own; things that touch the system still ask.

## Undo

Every job that changed something outside its project ends with **Keep / Undo**. Later, Settings >
Undo history lists those jobs with an Undo button. In a terminal, `genesis-txd list` and
`genesis-txd undo <id>` do the same.

## Babel, the IDE

<p align="center"><img src="docs/screens/babel/02-babel.png" width="88%" alt="Babel, the Genesis IDE"></p>
<p align="center"><img src="docs/screens/babel/01-maker-in-the-sidebar.png" width="88%" alt="Babel with the Genesis maker in the sidebar after changing pomodoro.py"></p>

Babel is in the dock and the app menu: a full code editor (Code-OSS, so every language, debugger and
extension VS Code has), with Genesis built in. Python, JavaScript and TypeScript, Rust, Go and C/C++
understand your code out of the box: completion, errors, go to definition, and debugging (Python,
Rust, Go, C/C++); the language servers and debuggers are part of the image, nothing is downloaded on
first use.

- **Genesis in the sidebar** (the ring icon): type what should change in the open folder, watch the
  steps, answer the cards, steer, undo. It is the same maker, working on the folder you have open.
- **Right-click in the editor**: "Ask about the selection" explains the selected code in the Genesis
  output panel; "Change this file…" asks for a change to just that file.
- **Ctrl+Alt+Space** anywhere in Babel: make something in this workspace.
- **Made here** in the sidebar lists everything Genesis built; one click opens it in Babel. The maker's
  gallery has "Open in Babel" too.

### Suggestions while you type

Babel suggests the next lines as you write, in every language, from the small model on this machine. The
suggestion appears in grey; `Tab` takes it, keep typing to ignore it. The status bar says which model is
answering ("fast completion") and one click turns it off. Packs with a dedicated completion model use
that; the others use the always-loaded small model, so the first suggestion after a pause takes a few
seconds on CPU-only machines and is quick once the file is warm. Nothing leaves the computer.

## Chat

Not everything is a thing to build. **Chat with Genesis** in the app menu (or the Chat button in the
maker) is an ordinary conversation that stays on this machine: ask what you wrote about a trip, what a
term means, whether an update is waiting, which apps are installed. Every chat is kept and the list on
the left searches all of them. Attach a PDF, a document or a photo by its path and ask about it: text
is read from the file, a photo goes to the local vision model. When a question needs one of your tools
or a file outside your opted-in folders, the same permission card appears as in the maker.

## Ask, anywhere

<p align="center"><img src="docs/screens/more/02-screen-region-asked.png" width="88%" alt="A screen region handed to "What is on my screen"; the local model described it in seven seconds"></p>

- **Meta+Space**, type a question instead of a request; the answer appears in the same window.
- **Select text** in any app, press Meta+Shift+Space: ask about the selection.
- **Meta+Shift+A**: select a region of the screen and ask what it is; the answer arrives as a
  notification and is copied to the clipboard.
- **Terminal:** `ask how do I find large files` answers in the shell; Ctrl+G turns the line you typed
  into a command.
- **Your files:** Settings > "Files Genesis may search", add a folder (Documents, Notes). Then "what did I
  write about the trip?" finds it, even when the note says "our week in Lisbon" and never "trip": the
  index searches by words and by meaning, with the small embedding model every pack ships. Only the
  folders you add are read, and nothing leaves the machine.

## Voice

Hold the microphone button in the maker or the palette, speak, release. The transcript appears before
anything runs. Replies can be spoken back (toggle in the window). Speech recognition understands many
languages; spoken replies are English for now.

## Your phone

<p align="center"><img src="docs/screens/phone/05-phone-replies.png" width="88%" alt="Answers from Genesis arriving as phone notifications"></p>

After pairing with KDE Connect:

- share any text to Genesis starting with **ask** and the answer comes back as a notification;
- send a voice memo to Genesis and it is transcribed and answered the same way;
- in KDE Connect, open the "genesis" device and use **Run Command** for day / night, lock the screen, or
  "Ask Genesis what I just shared";
- on Android, the palette shows your phone's notifications and can reply to those that allow it.

If the phone does not find the computer, type the phone's address into Settings > Phone; the computer
starts the connection instead.

<p align="center"><img src="docs/screens/phone-app/01-pair.png" width="30%" alt="The Genesis app: pairing"> <img src="docs/screens/phone-app/02-job-from-the-phone.png" width="30%" alt="The Genesis app: a job started from the phone"> <img src="docs/screens/phone-app/03-permission-notification.png" width="30%" alt="A permission card as a phone notification"></p>

**The Genesis app (Android).** Settings > Phone app shows a code; scan it with the app (or copy and
paste it). From then on the phone shows your jobs, lets you answer permission cards, start a make, see what was
made and fetch one screenshot of the computer. When a job waits for your answer, the phone gets a
notification with **Allow once** and **Not now** right on it; a quiet "Connected to genesis"
notification stays while the app watches. Same network, encrypted to this machine only; "Regenerate the
code" unpairs every phone.

## Your own tools (MCP)

The maker's abilities are not fixed. Any program that speaks MCP, the open Model Context Protocol, can
give it new tools, and Genesis ships one: the **system** server, which lets the maker read the
machine's status, network and updates, search Flathub and install or remove apps, list printers, and
switch day and night. Ask "install VLC" or "is an update waiting?" and the maker uses them, with the
same permission cards as everything else: installing is a system change and asks first, searching the
catalogue is network use, reading status is free.

Settings, Tools shows every server, whether it runs, and each tool's tier. To add your own, put it in
`~/.config/genesis/mcp.json` and press Reload:

```json
{"servers": {"notes": {"command": "/home/you/bin/notes-mcp", "tier": "read",
                       "tiers": {"add_note": "write"}, "description": "my notes"}}}
```

The tier is what the permission rules apply to every tool of that server: `read`, `write`, `network`,
`system` or `never`, per tool if you list them. The server runs as you, like any program you start.
The shipped one, `/usr/bin/genesis-mcp-system`, is 120 lines of Python and a fine template.

## Backup, and moving to a new machine

Settings > Backup and moving > **Export to Downloads** writes one file with everything Genesis made
and knows about you: the projects with their recipes, your Genesis settings, the folders you opted in
for search, the phone pairings and the undo history. No models (they download again), no keys, no
passwords. On a new Genesis, put the file anywhere in your home folder and use **Restore**; existing
projects are kept, restored ones get a "-restored" suffix if a name is taken. Pair the phone again
afterwards. In a terminal: `genesis-backup export`, `genesis-backup restore <file>`.

## Recipes: what to make, step by step

The maker's start page has a **Recipes** row: a few that ship with Genesis (a checklist app, a
pomodoro timer, a website for a club, a CLI grown in three steps…) and your own. One click replays the
steps, with the same permission cards. In the gallery, **Save recipe** keeps a made project's steps
under Recipes; **Copy recipe** puts them on the clipboard to send to someone, who pastes them into
"Make this from a recipe". Right-click one of your recipes to delete it. **Export** bundles the files
with the recipe, as one archive.

## Updates

<p align="center"><img src="docs/screens/more/03-settings.png" width="88%" alt="Genesis Settings: updates, this machine, models, and the rest"></p>

Genesis fetches updates in the background and stages them; nothing restarts on its own. The dock's
status widget and Settings > Updates say when one is ready; **Apply at next restart** is a click, and
every update can be rolled back from the boot menu.

## Day and night

Settings > Day / night, or say "dark mode" to the microphone. The desktop, the terminal and the
maker switch together.

## Your language, and screen readers

The maker, the palette and Genesis Settings follow the language of your desktop: German and Slovenian
are complete, English is the source. Anything not yet translated stays in English rather than
disappearing. To help with another language, see CONTRIBUTING.md.

A screen reader works with all of Genesis: turn it on in System Settings, Accessibility, Screen Reader
(Orca), and every button in the maker, the palette and Settings reads its purpose, including which
project a "Continue" or "Open in Babel" button belongs to. The dictation field says what it wants,
and the mic button says "Hold to talk".

## Claude Code, with your own account

If you have a Claude subscription: Settings > "Claude, with your account" opens Claude Code in a
terminal, signed in with your login, working in the same project folders. Nothing of it is used
unless you open it.

## When something is wrong

- **"No models yet" in the maker:** the download did not finish or the model service is starting.
  App menu > Set up models.
- **The maker says the model is unreachable:** wait a minute after login, then try again; Settings >
  "On this machine" shows whether the model service runs.
- **A step runs for very long:** on CPU-only machines the bigger packs are slow. The tiny pack answers
  in seconds; choose it under Set up models.
- **Something changed that you did not want:** Settings > Undo history.
- **Report:** GitHub issues, "Hardware report" or "Bug" template. `genesis-image-check` in a terminal
  prints a self-check you can paste.
