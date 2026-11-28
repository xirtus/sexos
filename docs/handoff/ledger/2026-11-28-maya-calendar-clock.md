# Maya calendar in the clock, calendar and bar (2026-11-28)

Andreas asked for SexTime's Maya calendar everywhere the date shows, and the
Dreamspell wave spell and oracle cross in the calendar drop-down.

- `sex-time`: Tzolk'in, Haab' and `MayaDate` (Long Count + Calendar Round,
  GMT 584283), and `dreamspell` (kin, tones, seals, wave spell, oracle;
  29 February keeps 28 February's kin; 2012-12-21 = Kin 207, 2026-11-28 = Kin 94).
- SilkBar clock: `22:40 CST | 28 NOV 2026 | 13.0.14.2.10`, always 24-hour.
  The clock box is 280 logical px when the bar is at least 700 px wide.
- Clock menu: a Maya row (Long Count and Calendar Round) and the real zone name.
- Calendar drop-down: an oracle band above the month (guide, antipode, destiny,
  analog, occult as seal tiles with Maya-numeral tones; kin, tone, seal, wave
  spell and its day) and a Maya footer under the weeks. Local day, not UTC.
- Seal glyphs: 20 new `line seal-*` icons in `assets/icons/icons.txt` (our own
  drawings), SVGs in `assets/icons/svg/line/`.
- Desktop clock card: a third line with the full Maya date.
- Bar time fix: SilkBar published the raw CMOS reading, ignoring the network
  time step sexnet sets (`civil_time_ms`). The x17's CMOS runs 6 h off, so the
  bar said 10:40 UTC at 04:40 UTC. It now publishes the kernel's civil time;
  the rollback guard still watches the CMOS reading itself.

Gates updated: display clock, civil clock, slim bar layout, silkbar clock gate
and its Python tests, tray, surface groups. Ships to the x17 in v7 with fresh
Shell, Display and SilkBar (Shell's calendar/menu hit areas share the new layout).
