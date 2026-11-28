# Maya calendar in the clock, calendar and bar (2026-11-28)

Andreas asked for SexTime's Maya calendar everywhere the date shows, and the
Dreamspell wave spell and oracle cross in the calendar drop-down.

- `sex-time`: Tzolk'in, Haab' and `MayaDate` (Long Count + Calendar Round,
  GMT 584283), and `dreamspell` (kin, tones, seals, wave spell, oracle;
  29 February keeps 28 February's kin; 2012-12-21 = Kin 207, 2026-11-28 = Kin 94).
- SilkBar clock: `22:40 CST | 28 NOV 2026 | 13.0.14.2.10`, always 24-hour.
  The clock box is 280 logical px when the bar is at least 700 px wide.
- Clock menu: a Maya row (Long Count and Calendar Round) and the real zone name.
- Calendar drop-down: an oracle band above the month laid out like the 13 Moon
  day charts (starroot daycalc): the oracle cross (guide above, antipode left,
  destiny framed green, analog right, occult below, each with its tone in Maya
  dots and bars) and the 13-day wave spell as a U on the right (1-5 down, 6-9
  along the bottom, 10-12 up, 13 beside 12; gates 1/5/9/13 marked, today
  framed), "Wave spell N" and "Kin N / Day N of 13" inside the U, and under
  both the day's name ("White Electric Wizard"), its wave spell and its castle.
  Layout lives in `sex_pdx::tray::Oracle` (tests/tray.rs checks no overlap).
  A Maya footer sits under the weeks. Local day, not UTC.
- Seal glyphs: the real Maya day signs, traced from CJLL Wright's Commons
  drawings (CC BY 2.5, `assets/maya/ATTRIBUTION.md`) by
  `scripts/brand/maya_glyphs.py` into `assets/maya/day-signs.txt` (silhouette +
  ink, nonzero fill) and `assets/maya/svg/`. `sex_icons::maya` draws them with
  a winding-keeping fill (the icon set's fills normalise winding, which would
  close the holes); each cartouche is filled with its seal colour.
- Desktop clock card: a third line with the full Maya date.
- Bar time fix: SilkBar published the raw CMOS reading, ignoring the network
  time step sexnet sets (`civil_time_ms`). The x17's CMOS runs 6 h off, so the
  bar said 10:40 UTC at 04:40 UTC. It now publishes the kernel's civil time;
  the rollback guard still watches the CMOS reading itself.

Gates updated: display clock, civil clock, slim bar layout, silkbar clock gate
and its Python tests, tray, surface groups. Ships to the x17 in release 20 with fresh
Shell, Display and SilkBar (Shell's calendar/menu hit areas share the new layout).
