# Ten minutes with a real phone

What the emulator could not settle: whether phone notifications reach the desktop and whether replies
go back. Everything else on the phone side (pairing, commands, "ask" text, voice notes) is verified.

You need: Genesis running (the VM on the Mac is fine), your phone on the same Wi-Fi as the Mac, and
the KDE Connect app (Play Store, F-Droid, or the App Store on iOS; the iOS app has no notification sync).

1. **Pair.** In Genesis: Settings > Phone (or the "Pair your phone" step of first run). On the phone:
   open KDE Connect, look for "genesis". If it does not show up, the phone's Wi-Fi and the VM are on
   different networks: type the Mac's address into "Phone not showing up?" in Genesis, and on the phone
   under "Add devices by IP" enter the Mac's address too. Accept on the phone; the eight-character code
   must match on both screens.
2. **Notification sync.** On the phone: the "genesis" device page > Plugin settings > Notification sync
   must be on, and Android will ask to grant notification access. In the plugin's own settings, make
   sure "Send notifications only if the screen is off" is off for this test.
3. **See them.** Send yourself a message in any messaging app. Within a few seconds the Genesis palette
   (Meta+Space) shows the notification under the starter ideas.
4. **Reply.** For a notification whose app allows inline replies (most messengers), a "Reply from here"
   field appears in the palette. Type, Enter. The reply goes through the phone, as if typed there.
5. **Report.** Open an issue with the "Hardware report" template, section "Phone", noting the phone
   model, Android version, KDE Connect version, and which of steps 3 and 4 worked.

If step 3 shows nothing: on a terminal in Genesis, `genesis-phone notifications` prints what KDE
Connect exposes; empty means the phone is not sending (plugin, permission or filter), a list means
the palette is at fault.
