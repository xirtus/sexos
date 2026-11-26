# 2026-11-25 sex-net: keep-alives only on Spindle's flows

Turning keep-alives off on closing flows (2026-11-25-tcp-keep-alive-closing) did
not bring back PR #526's Qupid Kodi gate: `run +net qupid` instances still never
exited. Keep-alives are now off by default again. `Engine::open_keep_alive` turns
them on per flow, `open_connect` clears them (flows are reused), and sexnet turns
them on only for flows a Spindle connects: the tether, which idles between commands.
