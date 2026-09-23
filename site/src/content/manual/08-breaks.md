---
title: "When something breaks"
number: 8
slug: "breaks"
---
<p><b>A program crashes.</b> A notification says so, once, and offers <b>Ask Genesis</b>. Pressing it hands the crash — the program, the signal, the stack — to the assistant here, which reads the journal around it and says in plain words what went wrong and whether it matters. A crash dump carries the arguments a program was given and the paths it had open, which is exactly why this stays on the machine.</p>
<p><b>A service fails.</b> Same offer. "backup.service failed" becomes "your backup disk was not plugged in". Ten times more common than a crash and just as opaque without this.</p>
<p><b>The assistant gives a bad answer.</b> It's a small model on your hardware; it will sometimes be wrong. The rules above mean a wrong answer can't do harm without asking first, and Undo takes back what it did.</p>
<p><b>An update broke something.</b> Restart, and pick the previous entry in the boot menu. The image you were on is kept as the rollback, every time. Nothing to reinstall.</p>
<p><b>Something else.</b> <code>genesis-report</code> prints what this machine is and how it was set up — hardware, models, what was measured — with nothing personal in it. That's the file to attach when you ask for help.</p>
