# ladspa-t5-plugins

This plugin collection is intended for use with [Pulseaudio Parametric
Equalizer](https://t-5.eu/hp/Software/Pulseaudio%20Parametric%20Equalizer/)
and will be extended to support building multi-way loudspeaker crossovers in
pulseaudio (look at [Pulseaudio Crossover
Rack](https://t-5.eu/hp/Software/Pulseaudio%20Crossover%20Rack/)).

Currently the following plugins are implemented:

* 3band_parameq_with_shelves (id 5541)
  Three-Band Parametric Equalizer (Frequency, Gain, Q)  
  with low- and high shelves (Frequency, Gain, Q)7
* inverter (id 5556)
  simply inverts the input signal
* lr4_lowpass (id 5542)
  4th-order Linkwitz-Riley low pass filter with 24dB/octave
  slope and -6dB attenuation at the cutoff frequency.
* lr4_highpass (id 5543)
  4th-order Linkwitz-Riley high pass filter with 24dB/octave
  slope and -6dB attenuation at the cutoff frequency.
* lr2_lowpass (id 5544)
  2nd-order Linkwitz-Riley low pass filter with 12dB/octave
  slope and -6dB attenuation at the cutoff frequency.
* lr2_highpass (id 5545)
  2nd-order Linkwitz-Riley high pass filter with 12dB/octave
  slope and -6dB attenuation at the cutoff frequency.
* fo_lowpass (id 5546)
  1st-order butterworth low pass filter with 6dB/octave slope
  and -3dB attenuation at the cutoff frequency.
* fo_highpass (id 5547)
  1st-order butterworth high pass filter with 6dB/octave slope
  and -3dB attenuation at the cutoff frequency.
* so_lowpass (id 5550)
  2nd-order butterworth low pass filter with 12dB/octave slope
  and -3dB attenuation at the cutoff frequency.
* so_highpass (id 5551)
  2nd-order butterworth high pass filter with 12dB/octave slope
  and -3dB attenuation at the cutoff frequency.
* so_lowpass_q (id 5554)
  2nd-order low pass filter with 12dB/octave slope, variable Q
  and -3dB attenuation at the cutoff frequency.
* so_highpass_q (id 5555)
  2nd-order high pass filter with 12dB/octave slope, variable Q
  and -3dB attenuation at the cutoff frequency.
* to_lowpass (id 5552)
  3rd-order butterworth low pass filter with 18dB/octave slope
  and -3dB attenuation at the cutoff frequency.
* to_highpass (id 5553)
  3rd-order butterworth high pass filter with 18dB/octave slope
  and -3dB attenuation at the cutoff frequency.
* sample_delay (id 5548)
  sample accurate delay, delay is given in milliseconds and
  is converted and rounded to the nearest whole sample count
* subsample_delay (id 5549)
  subsample accurate delay (fractional delay), delay is given
  in milliseconds. this is a linearly interpolating fractional
  delay filter with a sample accurate delay in frount of it.
  beware that the frequency response rolls off to max -3dB at 
  sample rate / 4 and so this should only be used with sample
  rates >= 96kHz for filtering tweeter outputs!
* linkwitz_transform (id 5557)

All plugins have a gain parameter.

## Notes

All plugins employ a shared memory parametrization interface exposed
via the /dev/shm filesystem. This is due to the fact that the two main
projects using these plugins insert them into the pulseaudio system
which exposes no way of changing filter parameters after insertion.
The details can be found in the code. The MMAP-Filename-Part parameter
is used to construct a filename in /dev/shm so the plugin's interfaces
can be identified correctly. It serves no other purpose.
