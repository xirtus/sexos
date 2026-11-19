#!/usr/bin/env python3
"""Writes Rendezvous's map fixtures (crates/sex-map/tests/fixtures/), all made up:

* tile-14-8800-5373.mvt: one vector tile (Mapbox Vector Tile 2.1, encoded here by
  hand, independently of the Rust decoder) with OpenMapTiles layer names: a river
  (water, water_name), a park (park and landcover), buildings (one with a courtyard),
  roads (transportation: primary, secondary, minor, rail), a street name and a place.
* tile-14-8800-5373.mvt.gz: the same, gzipped.
* search-fountain.json, search-station.json: Nominatim jsonv2 answers.
* route.json: an OSRM answer whose polyline runs from the fountain to the station
  along the roads.

Run it again only to change the fixtures; the files are committed."""
import gzip, json, math, struct
from pathlib import Path
OUT = Path(__file__).resolve().parents[1] / 'crates/sex-map/tests/fixtures'
Z, X, Y = 14, 8800, 5373
EXTENT = 4096

def varint(v):
    out = bytearray()
    while True:
        b = v & 0x7F; v >>= 7
        out.append(b | (0x80 if v else 0))
        if not v: return bytes(out)
def key(n, wire): return varint(n << 3 | wire)
def field_bytes(n, b): return key(n, 2) + varint(len(b)) + b
def field_varint(n, v): return key(n, 0) + varint(v)
def zz(v): return (v << 1) ^ (v >> 31)
def packed(n, vals): return field_bytes(n, b''.join(varint(v) for v in vals))

def geometry(kind, parts):
    cmds, cx, cy = [], 0, 0
    for part in parts:
        pts = part[:-1] if kind == 3 else part   # rings: ClosePath instead of the repeat
        cmds.append(1 | 1 << 3)
        cmds += [zz(pts[0][0] - cx), zz(pts[0][1] - cy)]; cx, cy = pts[0]
        if len(pts) > 1:
            cmds.append(2 | (len(pts) - 1) << 3)
            for x, y in pts[1:]:
                cmds += [zz(x - cx), zz(y - cy)]; cx, cy = x, y
        if kind == 3: cmds.append(7 | 1 << 3)
    return cmds

def value(v):
    if isinstance(v, str): return field_bytes(1, v.encode())
    if isinstance(v, bool): return field_varint(7, int(v))
    if isinstance(v, int): return field_varint(4, v)
    return key(3, 1) + struct.pack('<d', v)

def layer(name, features):
    keys, values, body = [], [], b''
    for fid, (kind, tags, parts) in enumerate(features, 1):
        t = []
        for k, v in tags.items():
            if k not in keys: keys.append(k)
            if v not in values: values.append(v)
            t += [keys.index(k), values.index(v)]
        f = field_varint(1, fid) + packed(2, t) + field_varint(3, kind) + packed(4, geometry(kind, parts))
        body += field_bytes(2, f)
    out = field_varint(15, 2) + field_bytes(1, name.encode()) + body
    for k in keys: out += field_bytes(3, k.encode())
    for v in values: out += field_bytes(4, value(v))
    return out + field_varint(5, EXTENT)

def rect(x0, y0, x1, y1):  # clockwise on screen (y down): the exterior ring
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
def hole(x0, y0, x1, y1):
    return [(x0, y0), (x0, y1), (x1, y1), (x1, y0), (x0, y0)]

FOUNTAIN = (2048, 1800)
STATION = (3000, 3000)
def lat_lon(px, py):
    n = 2 ** Z
    wx, wy = (X + px / EXTENT) / n, (Y + py / EXTENT) / n
    return math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * wy)))), wx * 360 - 180

def tile():
    layers = [
        layer('water', [(3, {'class': 'river'}, [[(-64, -64), (1200, -64), (1200, 4160), (-64, 4160), (-64, -64)]])]),
        layer('landcover', [(3, {'class': 'grass', 'subclass': 'park'}, [rect(1400, 300, 2400, 1400)])]),
        layer('park', [(3, {'class': 'park', 'name': 'Mill Park'}, [rect(1400, 300, 2400, 1400)])]),
        layer('building', [
            (3, {'render_height': 20}, [rect(2600, 2400, 2900, 2900)]),
            (3, {'render_height': 12}, [rect(3200, 2400, 3900, 3100), hole(3400, 2600, 3700, 2900)]),
            (3, {'render_height': 9}, [rect(3200, 3300, 3700, 3800)]),
        ]),
        layer('transportation', [
            (2, {'class': 'primary'}, [[(-64, 2048), (4160, 2048)]]),
            (2, {'class': 'secondary'}, [[(3000, -64), (3000, 4160)]]),
            (2, {'class': 'minor'}, [[(2048, 1500), (2048, 2048)], [(1300, 3500), (2900, 3500)]]),
            (2, {'class': 'rail'}, [[(1250, 3900), (4160, 3900)]]),
        ]),
        layer('transportation_name', [(2, {'class': 'primary', 'name': 'Main Street'}, [[(1300, 2048), (2900, 2048)]])]),
        layer('water_name', [(1, {'class': 'river', 'name': 'Blue River'}, [[(600, 1000)]])]),
        layer('place', [(1, {'class': 'neighbourhood', 'name': 'Fountain Square'}, [[FOUNTAIN]])]),
    ]
    return b''.join(field_bytes(3, l) for l in layers)

def polyline(points, precision=5):
    out, plat, plon = '', 0, 0
    for lat, lon in points:
        la, lo = round(lat * 10 ** precision), round(lon * 10 ** precision)
        for d in (la - plat, lo - plon):
            v = ~(d << 1) if d < 0 else d << 1
            while v >= 0x20: out += chr((0x20 | (v & 0x1F)) + 63); v >>= 5
            out += chr(v + 63)
        plat, plon = la, lo
    return out

def main():
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / f'tile-{Z}-{X}-{Y}.mvt').write_bytes(tile())
    # As tile servers send it: gzip (fixed mtime, so the file is reproducible).
    (OUT / f'tile-{Z}-{X}-{Y}.mvt.gz').write_bytes(gzip.compress(tile(), mtime=0))
    f, s = lat_lon(*FOUNTAIN), lat_lon(*STATION)
    place = lambda name, ll, cat, typ, pid: {'place_id': pid, 'licence': 'Fixture data, made up', 'osm_type': 'node', 'osm_id': pid,
        'lat': f'{ll[0]:.7f}', 'lon': f'{ll[1]:.7f}', 'category': cat, 'type': typ, 'place_rank': 30, 'importance': 0.3,
        'addresstype': cat, 'name': name.split(',')[0], 'display_name': name,
        'boundingbox': [f'{ll[0]-0.0005:.7f}', f'{ll[0]+0.0005:.7f}', f'{ll[1]-0.0005:.7f}', f'{ll[1]+0.0005:.7f}']}
    (OUT / 'search-fountain.json').write_text(json.dumps([place('Fountain Square, Mill Quarter, Sometown', f, 'amenity', 'fountain', 1001)], indent=1) + '\n')
    (OUT / 'search-station.json').write_text(json.dumps([place('Old Mill Station, Mill Quarter, Sometown', s, 'railway', 'station', 1002),
        place('Old Mill Station Bus Stop, Sometown', lat_lon(3100, 3050), 'highway', 'bus_stop', 1003)], indent=1) + '\n')
    way = [f, lat_lon(2048, 2048), lat_lon(3000, 2048), s]
    route = {'code': 'Ok', 'routes': [{'geometry': polyline(way), 'legs': [{'steps': [], 'summary': 'Main Street', 'weight': 95.1, 'duration': 95.1, 'distance': 812.4}],
        'weight_name': 'routability', 'weight': 95.1, 'duration': 95.1, 'distance': 812.4}],
        'waypoints': [{'hint': '', 'distance': 1.0, 'name': '', 'location': [f[1], f[0]]}, {'hint': '', 'distance': 1.0, 'name': 'Main Street', 'location': [s[1], s[0]]}]}
    (OUT / 'route.json').write_text(json.dumps(route, indent=1) + '\n')
    print({'tile': f'{Z}/{X}/{Y}', 'fountain': f, 'station': s})

if __name__ == '__main__':
    main()
