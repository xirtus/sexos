/*
  Header for lp4pole_filter struct, and functions to run instance.
  Copyright 2011 David Robillard
  Copyright 2003 Mike Rawes

  Originally originally appeared in CSound as Timo Tossavainen's (sp?)
  implementation from the Stilson/Smith CCRMA paper.

  See http://musicdsp.org/archive.php?classid=3#26

  Originally appeared in the arts softsynth by Stefan Westerfeld:
  http://www.arts-project.org/

  First ported to LADSPA by Reiner Klenk (pdq808[at]t-online.de)

  Tuning and stability issues (output NaN) and additional audio-rate
  variant added by Mike Rawes (mike_rawes[at]yahoo.co.uk)

  This is free software: you can redistribute it and/or modify it
  under the terms of the GNU General Public License as published by
  the Free Software Foundation, either version 3 of the License, or
  (at your option) any later version.

  This software is distributed in the hope that it will be useful,
  but WITHOUT ANY WARRANTY; without even the implied warranty of
  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
  GNU General Public License for more details.

  You should have received a copy of the GNU General Public License
  along with this software.  If not, see <http://www.gnu.org/licenses/>.
*/

#ifndef blop_lp4pole_filter_h
#define blop_lp4pole_filter_h

#include "common.h"

#include <math.h>

typedef struct {
	float f;
	float coeff;
	float fb;
	float in1;
	float in2;
	float in3;
	float in4;
	float inv_nyquist;
	float out1;
	float out2;
	float out3;
	float out4;
	float max_abs_in;
} LP4PoleFilter;

/**
   Allocate a new LP4PoleFilter instance.
   @param sample_rate Intended playback (DAC) rate
   @return Allocated LP4PoleFilter instance
*/
LP4PoleFilter* lp4pole_new(double sample_rate);

/**
   Cleanup an existing LP4PoleFilter instance.
   @param lpf Pointer to LP4PoleFilter instance allocated with initFilter
*/
void lp4pole_cleanup(LP4PoleFilter* lpf);

/**
   Initialise filter.
   @param lpf Pointer to LP4PoleFilter instance allocated with initFilter
*/
void lp4pole_init(LP4PoleFilter* lpf);

/**
   Set up filter coefficients for given LP4Pole instance.
   @param lpf       Pointer to LP4PoleFilter instance
   @param cutoff    Cutoff frequency in Hz
   @param resonance Resonance [Min=0.0, Max=4.0]
*/
static inline void
lp4pole_set_params(LP4PoleFilter* lpf,
                   float          cutoff,
                   float          resonance)
{
	float fsqd;
	float tuning;

	/* Normalise cutoff and find tuning - Magic numbers found empirically :) */
	lpf->f = cutoff * lpf->inv_nyquist;
	tuning = f_clip(3.13f - (lpf->f * 4.24703592f), 1.56503274f, 3.13f);

	/* Clip to bounds */
	lpf->f = f_clip(lpf->f * tuning, lpf->inv_nyquist, 1.16f);

	fsqd       = lpf->f * lpf->f;
	lpf->coeff = fsqd * fsqd * 0.35013f;

	lpf->fb = f_clip(resonance, -1.3f, 4.0f) * (1.0f - 0.15f * fsqd);

	lpf->f = 1.0f - lpf->f;
}

/**
   Run given LP4PoleFilter instance for a single sample.
   @param lpf Pointer to LP4PoleFilter instance
   @param in Input sample
   @return Filtered sample
*/
static inline float
lp4pole_run(LP4PoleFilter* lpf,
            float          in)
{
	const float abs_in = fabsf(16.0f * in); /* ~24dB unclipped headroom */

	lpf->max_abs_in = f_max(lpf->max_abs_in, abs_in);

	in -= lpf->out4 * lpf->fb;
	in *= lpf->coeff;

	lpf->out1 = in + 0.3f * lpf->in1 + lpf->f * lpf->out1; /* Pole 1 */
	lpf->in1  = in;
	lpf->out2 = lpf->out1 + 0.3f * lpf->in2 + lpf->f * lpf->out2;  /* Pole 2 */
	lpf->in2  = lpf->out1;
	lpf->out3 = lpf->out2 + 0.3f * lpf->in3 + lpf->f * lpf->out3;  /* Pole 3 */
	lpf->in3  = lpf->out2;
	lpf->out4 = lpf->out3 + 0.3f * lpf->in4 + lpf->f * lpf->out4;  /* Pole 4 */
	lpf->in4  = lpf->out3;

	/* Simple hard clip to prevent NaN */
	lpf->out4 = f_clip(lpf->out4, -lpf->max_abs_in, lpf->max_abs_in);

	lpf->max_abs_in *= 0.999f;

	return lpf->out4;
}

#endif /* blop_lp4pole_filter_h */
