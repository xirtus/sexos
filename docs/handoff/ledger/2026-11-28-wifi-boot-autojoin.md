# 2026-11-28: Wi-Fi boot auto-join keeps trying

Symptom: with Midnight1 saved in /data/wifi.conf, the x17 still came up off Wi-Fi after
restarts, so the tether never reconnected until Andreas joined by hand.

Cause (apps/spindle/src/wifi_command.rs boot reconnect): the first state poll ended the
whole attempt when Wi-Fi said NONE (the chip not started yet, early in boot) or FAILED
(one refused scan or join), and the budget was 20 polls of 1.5 s; each join also reset
the budget.

Change: NONE/FAILED now count as "not yet": a fresh scan is asked for and polling goes on;
a failed join is retried from that scan. One budget of 80 polls (about two minutes) covers
every attempt. `[spindle.wifi.auto] phase= polls= joined= retry=1` and `... gave_up` say
what happened, readable over the tether after the fact (`hw spindle.wifi.auto`).
