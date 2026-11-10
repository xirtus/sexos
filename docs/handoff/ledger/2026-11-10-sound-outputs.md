## The person picks where sound plays (2026-11-10)
- Bell finds the analog outputs of every HD Audio codec (speaker, headphones, line
  out; HDMI stays with the monitor), up to 8, and plays to the ones chosen
  (`hda::Hda::select`): a chosen jack is powered and its converter joins the stream,
  the others are switched off. The default is unchanged: the first codec's best output
  and every jack sharing its converter.
- Mixer actions `OUTPUTS` (one word: count, kinds, which play) and `SET_OUTPUTS` (a
  mask). Spindle: `volume outputs`, `volume output N`, `volume output all`. A virtio
  card has one output.
- Gate: `scripts/sound_output_gate.py`, two codecs (QEMU hda-output and hda-micro) each
  recorded to its own WAV: every note is heard on the chosen output only.
- Seen while building it: a mixer request Spindle sends from inside its handling of
  Bell's previous reply never got its answer, so the outputs come in one word.
  `volume list` asks row by row the same way and needs checking.
- Next: follow the headphone jack (speaker off while headphones are in) by pin sense
  and unsolicited responses; HDMI and DisplayPort audio.
=======
