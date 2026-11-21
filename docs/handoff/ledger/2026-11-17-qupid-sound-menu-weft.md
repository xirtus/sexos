# Qupid: sound menu (boost, EQ, dialogue lift) and Weft-styled controls

`Player::set_sound_effects` runs `sex_media::audio_dsp` (from #375) over every decoded
sound frame. Qupid's E opens a sound menu card: Boost 50-300%, equalizer presets
(Flat, Projector, Dialogue, Cinema, Night) and dialogue lift; B toggles 200%. Settings
carry from item to item. The controls band and cards now use Weft's dark theme
(surface #26232B, track #45404C, accent #DB4A7E, 6 px radius): a translucent band, an
accent progress track with a knob, rounded cards with an accent bar.
`qupid_chapters_gate.py` checks B is heard twice as loud and E changes the preset.
