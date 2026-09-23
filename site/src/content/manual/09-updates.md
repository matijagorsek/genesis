---
title: "Updates"
number: 9
slug: "updates"
---
<p>Genesis fetches updates in the background and stages them. <b>Nothing restarts on its own.</b> When one is ready, a notification says what's in it — kernel, graphics, audio, what was added, which parts of Genesis changed — read off the two images on the disk, not off a website. "What is in it" hands the notes to the assistant for a plain-words version. The update applies the next time you restart.</p>
<figure><img src="screens/more/03-settings.png" alt="Genesis Settings: updates, this machine, models, and the rest"><figcaption>Settings: updates, this machine, models, tools, undo history.</figcaption></figure>
<p>Your machine follows the <code>stable</code> channel. Stable only moves after a nightly test has taken a machine installed a week ago, upgraded it to the new image, rebooted, and checked that it boots, sets itself up and answers a question. A build that fails that never reaches you. If you want every build as it lands: <code>sudo bootc switch ghcr.io/matijagorsek/genesis:testing</code>.</p>
