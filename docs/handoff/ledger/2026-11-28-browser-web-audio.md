# 2026-11-28: Web Audio for web pages, stage 2 of StreamYard

Goal (Andreas): Kaleidoscope works with StreamYard; this is stage C2 (Web Audio, the
mic level meter). Plan: docs/handoff/BROWSER_CAPTURE_WEBRTC_PLAN_2026_11_28.md.
Stacked on C1 (getUserMedia, PR #863).

## Graph (crates/sex-webaudio, no_std, new)
- Runs in the browser, one 128 frame quantum at a time. Samples Q15 in i32, params Q16
  per sample, biquad coefficients Q26 with i64 sums; floats only once per quantum
  (automation, coefficients) or at setup (FFT tables, buffers): cheap on soft-float.
- Nodes: destination, Gain, Oscillator (sine/square/sawtooth/triangle/custom
  PeriodicWave), AudioBufferSource (loop, playbackRate, detune), ConstantSource,
  BiquadFilter (all 8 types, getFrequencyResponse), Delay (cycles allowed through it),
  StereoPanner, ChannelSplitter/Merger, DynamicsCompressor (reduction), Analyser
  (fftSize 32..32768, smoothing, byte/float frequency and time domain), stream source,
  stream destination, element source (silent for now).
- AudioParam: setValueAtTime, linear/exponential ramps, setTargetAtTime,
  setValueCurveAtTime, cancel(AndHold)ScheduledValues, audio-rate param inputs.
- Channel up/down-mix (speakers and discrete), topological order per change.
- Taps: tap 0 is the microphone (apps/web/src/capture.rs feeds it); every
  MediaStreamAudioDestinationNode gets a tap of its own. Recording and WebRTC read them
  with `Hub::read_tap(id, &mut cursor, out)` (48 kHz stereo i16, one second kept);
  in JS the track carries the id as `track._tap`.
- OfflineAudioContext renders to the end in one call (ten minutes at most).
- Host tests: crates/sex-webaudio/tests/graph.rs (mixing, 440 Hz analyser bin, ramps,
  filters, splitter/merger/compressor, mic tap, decode resampling). CI Host step.

## Pages (sex-js)
- `AudioContext` (+ `webkitAudioContext`), `OfflineAudioContext`, `BaseAudioContext`,
  state/resume/suspend/close/currentTime/baseLatency, `createX` and constructors for
  each node, `AudioBuffer`, `decodeAudioData` (callbacks and promise),
  `OfflineAudioCompletionEvent`, `AudioListener` (params only).
- New ops: text `AUDIO = 41` (`Hub::call` verbs new, node, connect, connectparam,
  disconnect, param, get, set, start, stop, state, time, reduction) and bytes
  `AUDIO_DATA = 77` (`Hub::call_bytes`: analyser data, buffers, periodic waves,
  responses, offline renders, decode). Dispatch `audio\tended\tCTX\tNODE`.
- Contexts live as long as the page (`Document::audio_alive`), close on navigation.
- Host test: crates/sex-js/tests/webaudio.rs, including the gate's own page.

## Browser (apps/web/src/webaudio.rs)
- Renders due quanta each tick; output through the program's one sound ring. A playing
  `<video>`/`<audio>` keeps the ring (Web Audio's clock runs on ticks meanwhile); when
  none plays, Web Audio takes it when it has sound and gives it back after two quiet
  seconds or when an element starts (`hush_media` calls `release`).
- `decodeAudioData` through sex-media's decoders (48 kHz stereo).
- Serial: `[web.audio.running]`, `[web.audio.ring] held=`, `[web.audio.out] frames=
  heard= peak= ring=`, `[web.audio.decode]`.
- QEMU gate: scripts/program_web_audio_gate.py (--fake-media: mic into an analyser
  finds bin 19 = 440 Hz; an oscillator plays through the ring), in CI after the
  capture gate. Page: scripts/web_site/webaudio.html.

## Left
- MediaElementAudioSourceNode is silent (element sound still plays on its own).
- AudioWorklet, PannerNode (3D), ConvolverNode, WaveShaper, IIRFilter, ScriptProcessor.
- Web Audio and an element do not mix into one ring yet: one wins (element first).
