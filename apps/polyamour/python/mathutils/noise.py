# SPDX-License-Identifier: GPL-3.0-only
"""mathutils.noise on Polyamour's noise: the same gradient noise, fractals
and Voronoi the shader nodes use (crates/sex-3d render/shader/tex.rs).

Inside Polyamour the values come from that Rust code over the live channel;
without one (a plain python3) the same Perlin noise is computed here, so
`noise(p)` agrees either way (to float precision).

Noise bases are accepted by Blender's names; every basis but VORONOI_* and
CELLNOISE uses the gradient noise (`'PERLIN_ORIGINAL'`, `'BLENDER'`...).
"""
import math
import random as _random

from . import Vector

_rng = _random.Random(0)

NOISE_BASES = ('BLENDER', 'PERLIN_ORIGINAL', 'PERLIN_NEW', 'VORONOI_F1', 'VORONOI_F2', 'VORONOI_F3', 'VORONOI_F4',
               'VORONOI_F2F1', 'VORONOI_CRACKLE', 'CELLNOISE')


def _remote(kind, points, **opts):
    """Values from Polyamour (None when there is no live channel)."""
    try:
        from polyamour.polyops import channel
    except Exception:
        return None
    if channel is None or not channel.live:
        return None
    return channel.noise(kind, points, **opts)


# --- the local port of tex.rs's Perlin noise --------------------------------------------

_M = 0xffffffff


def _rot(x, k):
    return ((x << k) | (x >> (32 - k))) & _M


def _final(a, b, c):
    c ^= b; c = (c - _rot(b, 14)) & _M
    a ^= c; a = (a - _rot(c, 11)) & _M
    b ^= a; b = (b - _rot(a, 25)) & _M
    c ^= b; c = (c - _rot(b, 16)) & _M
    a ^= c; a = (a - _rot(c, 4)) & _M
    b ^= a; b = (b - _rot(a, 14)) & _M
    c ^= b; c = (c - _rot(b, 24)) & _M
    return c


def _hash3(x, y, z):
    a = (0xdeadbeef + (3 << 2) + 13) & _M
    b = c = a
    c = (c + (z & _M)) & _M
    b = (b + (y & _M)) & _M
    a = (a + (x & _M)) & _M
    return _final(a, b, c)


def _f32(x):
    import struct
    return struct.unpack('<f', struct.pack('<f', x))[0]


def _fade(t):
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0)


def _grad3(h, x, y, z):
    h &= 15
    u = x if h < 8 else y
    vt = x if h in (12, 14) else z
    v = y if h < 4 else vt
    return (-u if h & 1 else u) + (-v if h & 2 else v)


def _perlin3(x, y, z):
    xi, yi, zi = math.floor(x), math.floor(y), math.floor(z)
    fx, fy, fz = x - xi, y - yi, z - zi
    u, v, w = _fade(fx), _fade(fy), _fade(fz)

    def g(dx, dy, dz):
        return _grad3(_hash3(xi + dx, yi + dy, zi + dz), fx - dx, fy - dy, fz - dz)

    def bimix(v0, v1, v2, v3):
        return (1 - v) * (v0 * (1 - u) + v1 * u) + v * (v2 * (1 - u) + v3 * u)
    a = bimix(g(0, 0, 0), g(1, 0, 0), g(0, 1, 0), g(1, 1, 0))
    b = bimix(g(0, 0, 1), g(1, 0, 1), g(0, 1, 1), g(1, 1, 1))
    return (1 - w) * a + w * b


def _wrap(p):
    return math.fmod(p, 100000.0) + (0.5 if abs(p) >= 1000000.0 else 0.0)


def _snoise(p):
    return 0.9820 * _perlin3(_wrap(p[0]), _wrap(p[1]), _wrap(p[2]))


def _local(kind, pts, octaves=1, lacunarity=2.0, H=1.0, offset=0.0, gain=1.0):
    out = []
    for p in pts:
        if kind == 'perlin':
            out.append(_snoise(p))
        elif kind == 'turbulence':
            amp, s, f = 1.0, 0.0, 1.0
            for _ in range(int(octaves)):
                s += amp * abs(_snoise([c * f for c in p]))
                amp *= 0.5
                f *= 2.0
            out.append(s)
        else:
            # fBm (the other fractals fall back to it locally).
            amp, s, f = 1.0, 0.0, 1.0
            rough = lacunarity ** -H
            for _ in range(max(1, int(octaves))):
                s += amp * _snoise([c * f for c in p])
                amp *= rough
                f *= lacunarity
            out.append(s)
    return out


def _eval(kind, position, **opts):
    p = [float(x) for x in position][:3]
    while len(p) < 3:
        p.append(0.0)
    r = _remote(kind, [p], **opts)
    if r is None:
        r = _local(kind, [p], **opts)
    return r[0]


def _vec(kind, position, **opts):
    p = [float(x) for x in position][:3]
    while len(p) < 3:
        p.append(0.0)
    pts = [p, [p[0] + 13.5, p[1] + 13.5, p[2] + 13.5], [p[0] - 13.5, p[1] - 13.5, p[2] - 13.5]]
    r = _remote(kind, pts, **opts)
    if r is None:
        r = _local(kind, pts, **opts)
    return Vector(r)


# --- Blender's functions ---------------------------------------------------------------

def seed_set(seed):
    _rng.seed(seed)


def random():
    return _rng.random()


def random_unit_vector(size=3):
    while True:
        v = Vector([_rng.uniform(-1, 1) for _ in range(size)])
        l = v.length
        if 0 < l <= 1:
            return v / l


def random_vector(size=3):
    return Vector([_rng.uniform(-1, 1) for _ in range(size)])


def noise(position, noise_basis='PERLIN_ORIGINAL'):
    if str(noise_basis).startswith('VORONOI'):
        return voronoi(position)[0][0]
    if noise_basis == 'CELLNOISE':
        return cell(position)
    return _eval('perlin', position)


def noise_vector(position, noise_basis='PERLIN_ORIGINAL'):
    return _vec('perlin', position)


def turbulence(position, octaves, hard, noise_basis='PERLIN_ORIGINAL', amplitude_scale=0.5, frequency_scale=2.0):
    return _eval('turbulence', position, octaves=octaves)


def turbulence_vector(position, octaves, hard, noise_basis='PERLIN_ORIGINAL', amplitude_scale=0.5, frequency_scale=2.0):
    return _vec('turbulence', position, octaves=octaves)


def fractal(position, H, lacunarity, octaves, noise_basis='PERLIN_ORIGINAL'):
    return _eval('fbm', position, octaves=octaves, lacunarity=lacunarity, H=H)


def multi_fractal(position, H, lacunarity, octaves, noise_basis='PERLIN_ORIGINAL'):
    return _eval('multi_fractal', position, octaves=octaves, lacunarity=lacunarity, H=H)


def hetero_terrain(position, H, lacunarity, octaves, offset, noise_basis='PERLIN_ORIGINAL'):
    return _eval('hetero_terrain', position, octaves=octaves, lacunarity=lacunarity, H=H, offset=offset)


def ridged_multi_fractal(position, H, lacunarity, octaves, offset, gain, noise_basis='PERLIN_ORIGINAL'):
    return _eval('ridged_multi_fractal', position, octaves=octaves, lacunarity=lacunarity, H=H, offset=offset, gain=gain)


def hybrid_multi_fractal(position, H, lacunarity, octaves, offset, gain, noise_basis='PERLIN_ORIGINAL'):
    return _eval('hybrid_multi_fractal', position, octaves=octaves, lacunarity=lacunarity, H=H, offset=offset, gain=gain)


def variable_lacunarity(position, distortion, noise_type1='PERLIN_ORIGINAL', noise_type2='PERLIN_ORIGINAL'):
    p = Vector(position).to_3d()
    off = noise_vector(p + Vector((5.2, 1.3, 7.7))) * distortion
    return noise(p + off)


def cell(position):
    p = [math.floor(float(x)) for x in list(position)[:3]] + [0, 0, 0]
    return _hash3(int(p[0]), int(p[1]), int(p[2])) / 4294967295.0


def cell_vector(position):
    p = [math.floor(float(x)) for x in list(position)[:3]] + [0, 0, 0]
    x, y, z = int(p[0]), int(p[1]), int(p[2])
    return Vector([_hash3(x + k * 7, y, z) / 4294967295.0 * 2 - 1 for k in range(3)])


def voronoi(position, distance_metric='DISTANCE', exponent=2.5):
    """(distances of the 4 nearest feature points, their positions)."""
    p = Vector(position).to_3d()
    base = [math.floor(c) for c in p]
    feats = []
    for dx in (-1, 0, 1):
        for dy in (-1, 0, 1):
            for dz in (-1, 0, 1):
                c = (base[0] + dx, base[1] + dy, base[2] + dz)
                off = [_hash3(c[0] + k * 101, c[1], c[2]) / 4294967295.0 for k in range(3)]
                fp = Vector((c[0] + off[0], c[1] + off[1], c[2] + off[2]))
                d = fp - p
                if distance_metric == 'MANHATTAN':
                    dist = sum(abs(x) for x in d)
                elif distance_metric == 'CHEBYCHEV':
                    dist = max(abs(x) for x in d)
                elif distance_metric == 'MINKOVSKY':
                    dist = sum(abs(x) ** exponent for x in d) ** (1.0 / exponent)
                elif distance_metric == 'DISTANCE_SQUARED':
                    dist = d.length_squared
                else:
                    dist = d.length
                feats.append((dist, fp))
    feats.sort(key=lambda f: f[0])
    return [f[0] for f in feats[:4]], [f[1] for f in feats[:4]]
