# 2026-11-25 sex-net: no keep-alives on a closing flow

Keep-alives every 10 s (2026-11-25-tcp-keep-alive) also ran on flows we had shut
down. A peer that answers them but never sends its FIN held the flow in FinWait2
forever, because smoltcp's 30 s timeout only fires on silence. `open_shutdown` and
`open_close` now turn keep-alives off, so such a flow times out as before.
Seen as PR #526's Qupid Kodi gate: `run +net qupid` instances stayed alive until
the fifth was refused for memory.
