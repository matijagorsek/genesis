# Examples: real runs

Everything here happened on a Genesis machine, recorded as it ran. Each is a recipe you can paste into
"Make this from a recipe" on the maker's start page.

## A checklist app (the first-run starter)

Made with Genesis (node-web)
1. a checklist app that saves to a file

The wizard's starter on a fresh disk: a Node web app with a JSON file behind it, built by the 2B model
on a CPU-only VM. Capture: docs/screens/fresh-run/02-first-make-from-the-wizard.png.

## A pomodoro timer

Made with Genesis (python-script)
1. a pomodoro timer with a bell
2. add a seconds option to pomodoro.py

Step 1 from the launcher (about four minutes on CPU), step 2 from Babel's sidebar. Captures:
docs/screens/fresh-run/03-pomodoro-made-by-the-2B-model.png, docs/screens/babel/01-maker-in-the-sidebar.png.

## A word-count tool, asked for from a phone

Made with Genesis (python-script)
1. a word-count tool for text files

Asked from the Genesis app on a Samsung phone paired over the local network; the job ran on the
computer and the phone showed each decision. Capture: docs/screens/phone-app/02-job-from-the-phone.png.

## The checklist CLI with tests (an older run, 27B model)

Made with Genesis (python-cli)
1. a checklist app that saves to a file

Seventeen turns, bugs found and fixed by the model along the way, tests green:
examples/checklist-in-vm/.
