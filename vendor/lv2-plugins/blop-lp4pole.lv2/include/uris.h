/*
  Common URIs used by plugins.
  Copyright 2012 David Robillard

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

#ifndef blop_uris_h
#define blop_uris_h

#include <lv2/atom/atom.h>
#include <lv2/core/lv2.h>
#include <lv2/morph/morph.h>
#include <lv2/urid/urid.h>

#include <string.h>

typedef struct {
	LV2_URID atom_URID;
	LV2_URID lv2_AudioPort;
	LV2_URID lv2_CVPort;
	LV2_URID lv2_ControlPort;
	LV2_URID morph_currentType;
} URIs;

static inline void
map_uris(URIs*                     uris,
         const LV2_Feature* const* features)
{
	LV2_URID_Map* map = NULL;
	for (int i = 0; features[i]; ++i) {
		if (!strcmp(features[i]->URI, LV2_URID__map)) {
			map = (LV2_URID_Map*)features[i]->data;
			break;
		}
	}

	if (map) {
		uris->atom_URID         = map->map(map->handle, LV2_ATOM__URID);
		uris->lv2_AudioPort     = map->map(map->handle, LV2_CORE__AudioPort);
		uris->lv2_CVPort        = map->map(map->handle, LV2_CORE__CVPort);
		uris->lv2_ControlPort   = map->map(map->handle, LV2_CORE__ControlPort);
		uris->morph_currentType = map->map(map->handle, LV2_MORPH__currentType);
	} else {
		memset(uris, 0, sizeof(*uris));
	}
}

#endif /* blop_uris_h */
