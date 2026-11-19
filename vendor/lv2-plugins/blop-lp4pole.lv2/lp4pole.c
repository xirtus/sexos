/*
  An LV2 plugin simulating a 4 pole low pass resonant filter.
  Copyright 2011 David Robillard
  Copyright 2003 Mike Rawes

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
#include "uris.h"

#include <lv2/core/lv2.h>
#include <lv2/options/options.h>
#include <lv2/urid/urid.h>

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define LP4POLE_CUTOFF    0
#define LP4POLE_RESONANCE 1
#define LP4POLE_INPUT     2
#define LP4POLE_OUTPUT    3

typedef struct {
	const float*   cutoff;
	const float*   resonance;
	const float*   input;
	float*         output;
	LP4PoleFilter* lpf;
	uint32_t       cutoff_is_cv;
	uint32_t       resonance_is_cv;
	URIs           uris;
} Lp4pole;

static void
cleanup(LV2_Handle instance)
{
	Lp4pole* plugin = (Lp4pole*)instance;

	lp4pole_cleanup(plugin->lpf);

	free(instance);
}

static void
connect_port(LV2_Handle instance,
             uint32_t   port,
             void*      data)
{
	Lp4pole* plugin = (Lp4pole*)instance;

	switch (port) {
	case LP4POLE_CUTOFF:
		plugin->cutoff = (const float*)data;
		break;
	case LP4POLE_RESONANCE:
		plugin->resonance = (const float*)data;
		break;
	case LP4POLE_INPUT:
		plugin->input = (const float*)data;
		break;
	case LP4POLE_OUTPUT:
		plugin->output = (float*)data;
		break;
	}
}

static uint32_t
options_get(LV2_Handle          instance,
            LV2_Options_Option* options)
{
	const Lp4pole* plugin = (const Lp4pole*)instance;
	uint32_t       ret    = 0;
	for (LV2_Options_Option* o = options; o->key; ++o) {
		if (o->context != LV2_OPTIONS_PORT) {
			ret |= LV2_OPTIONS_ERR_BAD_SUBJECT;
		} else if (o->key != plugin->uris.morph_currentType) {
			ret |= LV2_OPTIONS_ERR_BAD_KEY;
		} else if (o->subject == LP4POLE_CUTOFF) {
			o->size = sizeof(LV2_URID);
			o->type = plugin->uris.atom_URID;
			o->value =
			    (plugin->cutoff_is_cv ? &plugin->uris.lv2_CVPort
			                          : &plugin->uris.lv2_ControlPort);
		} else if (o->subject == LP4POLE_RESONANCE) {
			o->size = sizeof(LV2_URID);
			o->type = plugin->uris.atom_URID;
			o->value =
				(plugin->resonance_is_cv ? &plugin->uris.lv2_CVPort
				                         : &plugin->uris.lv2_ControlPort);
		} else {
			ret |= LV2_OPTIONS_ERR_BAD_SUBJECT;
		}
	}
	return ret;
}

static uint32_t
options_set(LV2_Handle                instance,
            const LV2_Options_Option* options)
{
	Lp4pole* plugin = (Lp4pole*)instance;
	uint32_t ret    = 0;
	for (const LV2_Options_Option* o = options; o->key; ++o) {
		if (o->context != LV2_OPTIONS_PORT) {
			ret |= LV2_OPTIONS_ERR_BAD_SUBJECT;
		} else if (o->key != plugin->uris.morph_currentType) {
			ret |= LV2_OPTIONS_ERR_BAD_KEY;
		} else if (o->type != plugin->uris.atom_URID) {
			ret |= LV2_OPTIONS_ERR_BAD_VALUE;
		} else {
			LV2_URID port_type = *(const LV2_URID*)(o->value);
			if (port_type != plugin->uris.lv2_ControlPort &&
			    port_type != plugin->uris.lv2_CVPort) {
				ret |= LV2_OPTIONS_ERR_BAD_VALUE;
				continue;
			}

			switch (o->subject) {
			case LP4POLE_CUTOFF:
				plugin->cutoff_is_cv = (port_type == plugin->uris.lv2_CVPort);
				break;
			case LP4POLE_RESONANCE:
				plugin->resonance_is_cv = (port_type == plugin->uris.lv2_CVPort);
				break;
			default:
				ret |= LV2_OPTIONS_ERR_BAD_SUBJECT;
			}
		}
	}
	return ret;
}

static LV2_Handle
instantiate(const LV2_Descriptor*     descriptor,
            double                    sample_rate,
            const char*               bundle_path,
            const LV2_Feature* const* features)
{
	Lp4pole* plugin = (Lp4pole*)malloc(sizeof(Lp4pole));

	if (plugin) {
		plugin->lpf = lp4pole_new(sample_rate);
		if (!plugin->lpf) {
			free(plugin);
			return NULL;
		}
		plugin->cutoff_is_cv    = 0;
		plugin->resonance_is_cv = 0;
		map_uris(&plugin->uris, features);
	}

	return (LV2_Handle)plugin;
}

static void
activate(LV2_Handle instance)
{
	Lp4pole* plugin = (Lp4pole*)instance;

	lp4pole_init(plugin->lpf);
}

static void
run(LV2_Handle instance,
    uint32_t   sample_count)
{
	Lp4pole* plugin = (Lp4pole*)instance;

	/* Cutoff Frequency (array of floats of length 1 or sample_count) */
	const float* cutoff = plugin->cutoff;

	/* Resonance (array of floats of length 1 or sample_count) */
	const float* resonance = plugin->resonance;

	/* Input (array of floats of length sample_count) */
	const float* input = plugin->input;

	/* Output (pointer to float value) */
	float* output = plugin->output;

	/* Instance data */
	LP4PoleFilter* lpf = plugin->lpf;

	for (uint32_t s = 0; s < sample_count; ++s) {
		const float co  = cutoff[s * plugin->cutoff_is_cv];
		const float res = resonance[s * plugin->resonance_is_cv];
		const float in  = input[s];

		/* TODO: There is no branching in this function.
		   Would it actually be faster to check if co or res has changed?
		*/
		lp4pole_set_params(lpf, co, res);

		output[s] = lp4pole_run(lpf, in);
	}
}

static const void*
extension_data(const char* uri)
{
	static const LV2_Options_Interface options = { options_get, options_set };
	if (!strcmp(uri, LV2_OPTIONS__interface)) {
		return &options;
	}
	return NULL;
}

static const LV2_Descriptor descriptor = {
	"http://drobilla.net/plugins/blop/lp4pole",
	instantiate,
	connect_port,
	activate,
	run,
	NULL,
	cleanup,
	extension_data,
};

LV2_SYMBOL_EXPORT const LV2_Descriptor*
lv2_descriptor(uint32_t index)
{
	switch (index) {
	case 0:  return &descriptor;
	default: return NULL;
	}
}
