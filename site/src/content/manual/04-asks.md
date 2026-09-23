---
title: "When it asks, and undo"
number: 4
slug: "asks"
---
<p>Genesis works in one of three modes, and <kbd>Meta</kbd>+<kbd>Shift</kbd>+<kbd>M</kbd> cycles them from anywhere:</p>
<ul>
<li><b>Ask</b> — asks before every change.</li>
<li><b>Trusted</b> (the default) — edits the project on its own, asks for everything outside it.</li>
<li><b>Hands-off</b> — also handles installs and your own settings on its own; things that touch the system still ask.</li>
</ul>
<p>Before anything outside the project, a card appears with a plain verb: "Genesis wants to install a package", "wants to write to your Documents". <b>Allow once</b> or <b>Not now</b>. If you always (or never) want something, Settings › Always and never keeps the rule.</p>
<figure><img src="screens/more/02-screen-region-asked.png" alt="A permission card with a plain verb"><figcaption>A card before it happens, in words, with two buttons. Rules, not the model, decide what can be asked at all.</figcaption></figure>
<p>Some things it never does, whatever you answer: read your keys and passwords, wipe disks, change the base system. Those are refused by the rules, not by the model's judgement.</p>
<p><b>Undo.</b> Every job that changed something outside its project was snapshotted first. The toast after a job offers Undo; Settings › Undo history lists every job, shows what it changed file by file, and can restore one file without undoing the rest. <b>Stop</b> ends a running job within a second and keeps what was made so far.</p>
