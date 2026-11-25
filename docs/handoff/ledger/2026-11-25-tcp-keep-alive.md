# 2026-11-25 sex-net: keep-alive on open-internet flows

Third x17 `tether sweep` (553e5dbc, CCM accepted on MIC OK): 110/128 trials pass,
2 MiB up at ~350 KB/s, 512 KiB down in 1.5-3 s; nothing left undecrypted at any
TID. Every `idle45` trial failed: smoltcp's 30 s timeout counts silence from the
peer, so an idle flow (a tether waiting for a command) was closed after 30 s.
Open flows now send keep-alives every 10 s (`set_keep_alive`), which draw an
ACK; only a peer that is really gone times out.
