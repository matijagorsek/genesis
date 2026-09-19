# Media

One window for the streaming services you already pay for, and a player for your own IPTV subscription.

## What it does

- **Services** — Netflix, Disney+, HBO Max, Prime Video, YouTube, Spotify and the rest open as their own
  windows, all sharing one Firefox profile, so you log in once per service and stay logged in. There is no
  single sign-on between these companies and there cannot be: each one authenticates separately.
- **IPTV** — your own provider's credentials (Xtream Codes, or a plain M3U playlist URL). It lists what your
  subscription carries, searches it, and hands the stream to Haruna.

## What it deliberately does not do

No DRM is bundled. Netflix, Disney+, HBO and Prime need Widevine; Firefox downloads that itself, with your
consent, under Mozilla's arrangement with Google. Genesis ships none of it, which is also why these services
top out at 720p on Linux — there is no hardware DRM path on the Linux desktop, for anyone.

Nothing is downloaded, recorded, re-streamed or scraped, and no ads are blocked. A player is neutral: what
your IPTV provider is allowed to carry is between you and them.

## Your credentials

IPTV details live in `~/.config/genesis-media/iptv.json`, readable only by you (0600). They are sent to the
address you entered and nowhere else. Streaming services keep their own cookies in the Firefox profile at
`~/.local/share/genesis-media/firefox`; delete that folder to sign out of everything at once.
