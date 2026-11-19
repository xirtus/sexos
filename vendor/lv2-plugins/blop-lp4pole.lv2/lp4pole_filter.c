/*
  lp4pole filter admin.
  Copyright 2011 David Robillard
  Copyright 2003 Mike Rawes

  See lp4pole_filter.h for history

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

#include "lp4pole_filter.h"

#include <stdlib.h>

LP4PoleFilter*
lp4pole_new(double sample_rate)
{
	LP4PoleFilter* lpf;

	lpf = (LP4PoleFilter*)malloc(sizeof(LP4PoleFilter));

	if (lpf) {
		lpf->inv_nyquist = 2.0f / sample_rate;
		lp4pole_init(lpf);
	}

	return lpf;
}

void
lp4pole_cleanup(LP4PoleFilter* lpf)
{
	if (lpf) {
		free(lpf);
	}
}

void
lp4pole_init(LP4PoleFilter* lpf)
{
	lpf->in1        = lpf->in2 = lpf->in3 = lpf->in4 = 0.0f;
	lpf->out1       = lpf->out2 = lpf->out3 = lpf->out4 = 0.0f;
	lpf->max_abs_in = 0.0f;
}
