# SPDX-License-Identifier: GPL-3.0-only
"""mathutils for Polyamour: Vector, Matrix, Quaternion, Euler and Color with
Blender's names, conventions and results (column vectors, `@` for products,
Euler orders as in Blender, right-handed Z-up), plus the `geometry`,
`kdtree`, `bvhtree`, `noise` and `interpolate` submodules.

Values that belong to Polyamour data (`obj.location`, `obj.matrix_world`)
write back when changed in place: `obj.location.x += 1` moves the object.
"""
import math

__all__ = ['Vector', 'Matrix', 'Quaternion', 'Euler', 'Color', 'geometry', 'kdtree', 'bvhtree', 'noise', 'interpolate']

_EPS = 1e-6


def _floats(seq):
    return [float(x) for x in seq]


class _Owned:
    """Write-through support: `_owner(value)` runs after an in-place change."""

    __slots__ = ()

    def _changed(self):
        if self._frozen:
            raise TypeError('%s is frozen, it cannot be changed' % type(self).__name__)
        cb = self._owner
        if cb is not None:
            cb(self)

    def _check_frozen(self):
        if self._frozen:
            raise TypeError('%s is frozen, it cannot be changed' % type(self).__name__)

    @property
    def is_wrapped(self):
        return False

    @property
    def is_frozen(self):
        return self._frozen

    @property
    def owner(self):
        return None

    def freeze(self):
        self._frozen = True
        return self


# ---------------------------------------------------------------------------- Vector

_SWZ = {'x': 0, 'y': 1, 'z': 2, 'w': 3}


class Vector(_Owned):
    __slots__ = ('_v', '_owner', '_frozen')

    def __init__(self, seq=(0.0, 0.0, 0.0), _owner=None):
        if isinstance(seq, (int, float)):
            raise TypeError('Vector(): expected a sequence of numbers')
        v = _floats(seq)
        if len(v) < 2:
            raise ValueError('Vector(): a vector needs 2 or more values, got %d' % len(v))
        self._v = v
        self._owner = _owner
        self._frozen = False

    # --- construction
    @classmethod
    def Fill(cls, size, fill=0.0):
        return cls([fill] * size)

    @classmethod
    def Range(cls, start, stop=None, step=1):
        if stop is None:
            start, stop = 0, start
        return cls(list(range(start, stop, step)))

    @classmethod
    def Linspace(cls, start, stop, size):
        if size < 2:
            raise ValueError('Vector.Linspace(): size must be 2 or more')
        return cls([start + (stop - start) * i / (size - 1) for i in range(size)])

    @classmethod
    def Repeat(cls, vector, size):
        v = list(vector)
        return cls([v[i % len(v)] for i in range(size)])

    def _set(self, values):
        self._v[:] = _floats(values)
        self._changed()

    # --- sequence protocol
    def __len__(self):
        return len(self._v)

    def __iter__(self):
        return iter(list(self._v))

    def __getitem__(self, i):
        if isinstance(i, slice):
            return tuple(self._v[i])
        return self._v[i]

    def __setitem__(self, i, value):
        self._check_frozen()
        if isinstance(i, slice):
            vals = _floats(value)
            if len(self._v[i]) != len(vals):
                raise ValueError('Vector slice assignment: size mismatch')
            self._v[i] = vals
        else:
            self._v[i] = float(value)
        self._changed()

    def __getattr__(self, name):
        if name.startswith('_') or not name or len(name) > 4 or any(c not in _SWZ for c in name):
            raise AttributeError("'Vector' object has no attribute '%s'" % name)
        idx = [_SWZ[c] for c in name]
        if any(i >= len(self._v) for i in idx):
            raise AttributeError("Vector.%s: the vector has %d values" % (name, len(self._v)))
        if len(idx) == 1:
            return self._v[idx[0]]
        return Vector([self._v[i] for i in idx])

    def __setattr__(self, name, value):
        if name in Vector.__slots__:
            object.__setattr__(self, name, value)
            return
        if not name or len(name) > 4 or any(c not in _SWZ for c in name):
            raise AttributeError("'Vector' object has no attribute '%s'" % name)
        idx = [_SWZ[c] for c in name]
        if len(set(idx)) != len(idx) or any(i >= len(self._v) for i in idx):
            raise AttributeError('Vector.%s cannot be assigned' % name)
        self._check_frozen()
        if len(idx) == 1:
            self._v[idx[0]] = float(value)
        else:
            vals = _floats(value)
            if len(vals) != len(idx):
                raise ValueError('Vector.%s = value: expected %d values' % (name, len(idx)))
            for i, x in zip(idx, vals):
                self._v[i] = x
        self._changed()

    # --- properties
    @property
    def length(self):
        return math.sqrt(sum(x * x for x in self._v))

    @length.setter
    def length(self, value):
        l = self.length
        if l > 0:
            self._set([x * value / l for x in self._v])

    magnitude = length

    @property
    def length_squared(self):
        return sum(x * x for x in self._v)

    # --- arithmetic
    def _other(self, o):
        if isinstance(o, Vector):
            ov = o._v
        else:
            ov = _floats(o)
        if len(ov) != len(self._v):
            raise ValueError('vectors must have the same size (%d and %d)' % (len(self._v), len(ov)))
        return ov

    def __add__(self, o):
        if isinstance(o, (int, float)):
            return NotImplemented
        return Vector([a + b for a, b in zip(self._v, self._other(o))])

    __radd__ = __add__

    def __sub__(self, o):
        if isinstance(o, (int, float)):
            return NotImplemented
        return Vector([a - b for a, b in zip(self._v, self._other(o))])

    def __rsub__(self, o):
        return Vector([b - a for a, b in zip(self._v, self._other(o))])

    def __mul__(self, o):
        if isinstance(o, (int, float)):
            return Vector([a * o for a in self._v])
        if isinstance(o, (Vector, list, tuple)):
            return Vector([a * b for a, b in zip(self._v, self._other(o))])
        return NotImplemented

    def __rmul__(self, o):
        return self.__mul__(o)

    def __truediv__(self, o):
        if isinstance(o, (int, float)):
            if o == 0:
                raise ZeroDivisionError('Vector division by zero')
            return Vector([a / o for a in self._v])
        return NotImplemented

    def __matmul__(self, o):
        if isinstance(o, Vector):
            return self.dot(o)
        if isinstance(o, Matrix):
            # Row vector times matrix.
            n = len(self._v)
            if o.row_count != n and not (n == 3 and o.row_count == 4):
                raise ValueError('vector @ matrix: sizes do not match')
            v = self._v + ([1.0] if o.row_count == n + 1 else [])
            out = [sum(v[r] * o._m[r][c] for r in range(len(v))) for c in range(o.col_count)]
            return Vector(out[:n] if len(out) > n else out)
        return NotImplemented

    def __rmatmul__(self, o):
        if isinstance(o, Matrix):
            return o.__matmul__(self)
        if isinstance(o, Quaternion):
            return o.__matmul__(self)
        return NotImplemented

    def __iadd__(self, o):
        self._set([a + b for a, b in zip(self._v, self._other(o))])
        return self

    def __isub__(self, o):
        self._set([a - b for a, b in zip(self._v, self._other(o))])
        return self

    def __imul__(self, o):
        if isinstance(o, (int, float)):
            self._set([a * o for a in self._v])
        elif isinstance(o, (Matrix, Quaternion)):
            raise TypeError('in-place multiplication by a matrix: use vec = mat @ vec')
        else:
            self._set([a * b for a, b in zip(self._v, self._other(o))])
        return self

    def __itruediv__(self, o):
        self._set([a / o for a in self._v])
        return self

    def __neg__(self):
        return Vector([-a for a in self._v])

    def __pos__(self):
        return Vector(self._v)

    def __abs__(self):
        return self.length

    def __eq__(self, o):
        if isinstance(o, Vector):
            return self._v == o._v
        if isinstance(o, (list, tuple)):
            return len(o) == len(self._v) and all(a == b for a, b in zip(self._v, o))
        return NotImplemented

    def __ne__(self, o):
        r = self.__eq__(o)
        return r if r is NotImplemented else not r

    def __lt__(self, o):
        return self.length < Vector(o).length

    def __le__(self, o):
        return self.length <= Vector(o).length

    def __gt__(self, o):
        return self.length > Vector(o).length

    def __ge__(self, o):
        return self.length >= Vector(o).length

    def __hash__(self):
        if not self._frozen:
            raise TypeError("unhashable type: 'Vector' (freeze() it first)")
        return hash(tuple(self._v))

    def __repr__(self):
        return 'Vector((%s))' % ', '.join('%.4f' % x for x in self._v)

    def __str__(self):
        return '<Vector (%s)>' % ', '.join('%.4f' % x for x in self._v)

    def __copy__(self):
        return Vector(self._v)

    def __deepcopy__(self, memo):
        return Vector(self._v)

    def __reduce__(self):
        return (Vector, (tuple(self._v),))

    # --- methods
    def copy(self):
        return Vector(self._v)

    def to_tuple(self, precision=-1):
        if precision < 0:
            return tuple(self._v)
        return tuple(round(x, precision) for x in self._v)

    def to_2d(self):
        return Vector((self._v + [0.0, 0.0])[:2])

    def to_3d(self):
        return Vector((self._v + [0.0, 0.0, 0.0])[:3])

    def to_4d(self):
        v = (self._v + [0.0, 0.0, 0.0])[:3]
        return Vector(v + [self._v[3] if len(self._v) > 3 else 1.0])

    def resized(self, size):
        return Vector((self._v + [0.0] * size)[:size])

    def resize(self, size):
        self._v[:] = (self._v + [0.0] * size)[:size]

    def resize_2d(self):
        self.resize(2)

    def resize_3d(self):
        self.resize(3)

    def resize_4d(self):
        self.resize(4)

    def zero(self):
        self._set([0.0] * len(self._v))

    def negate(self):
        self._set([-x for x in self._v])

    def normalized(self):
        l = self.length
        return Vector([x / l for x in self._v]) if l > 0 else Vector(self._v)

    def normalize(self):
        l = self.length
        if l > 0:
            self._set([x / l for x in self._v])

    def dot(self, o):
        return sum(a * b for a, b in zip(self._v, self._other(o)))

    def cross(self, o):
        b = self._other(o)
        a = self._v
        if len(a) == 2:
            return a[0] * b[1] - a[1] * b[0]
        if len(a) != 3:
            raise ValueError('Vector.cross(): only 2D and 3D vectors')
        return Vector((a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]))

    def angle(self, other, fallback=None):
        b = Vector(self._other(other))
        la, lb = self.length, b.length
        if la == 0 or lb == 0:
            if fallback is not None:
                return fallback
            raise ValueError('Vector.angle(other): zero length vectors have no valid angle')
        d = max(-1.0, min(1.0, self.dot(b) / (la * lb)))
        return math.acos(d)

    def angle_signed(self, other, fallback=None):
        if len(self._v) != 2:
            raise ValueError('Vector.angle_signed(): only 2D vectors')
        b = self._other(other)
        a = self._v
        if (a[0] == 0 and a[1] == 0) or (b[0] == 0 and b[1] == 0):
            if fallback is not None:
                return fallback
            raise ValueError('Vector.angle_signed(other): zero length vectors have no valid angle')
        return math.atan2(a[0] * b[1] - a[1] * b[0], a[0] * b[0] + a[1] * b[1]) * -1.0

    def lerp(self, other, factor):
        b = self._other(other)
        return Vector([a + (y - a) * factor for a, y in zip(self._v, b)])

    def slerp(self, other, factor, fallback=None):
        b = Vector(self._other(other))
        la, lb = self.length, b.length
        if la == 0 or lb == 0:
            if fallback is not None:
                return fallback
            raise ValueError('Vector.slerp(): zero length vectors')
        a1, b1 = self / la, b / lb
        d = max(-1.0, min(1.0, a1.dot(b1)))
        if abs(d) > 1 - 1e-6:
            if fallback is not None and d < 0:
                return fallback
            return self.lerp(b, factor)
        th = math.acos(d)
        s = math.sin(th)
        wa, wb = math.sin((1 - factor) * th) / s, math.sin(factor * th) / s
        return Vector([x * wa + y * wb for x, y in zip(a1._v, b1._v)]) * (la + (lb - la) * factor)

    def project(self, other):
        b = Vector(self._other(other))
        d = b.length_squared
        return b * (self.dot(b) / d) if d else Vector([0.0] * len(self._v))

    def reflect(self, mirror):
        n = Vector(self._other(mirror)).normalized()
        return self - n * (2 * self.dot(n))

    def orthogonal(self):
        if len(self._v) == 2:
            return Vector((-self._v[1], self._v[0]))
        x, y, z = self._v[:3]
        if abs(x) < abs(y) and abs(x) < abs(z):
            return Vector((0.0, -z, y))
        if abs(y) < abs(z):
            return Vector((-z, 0.0, x))
        return Vector((-y, x, 0.0))

    def rotate(self, other):
        if isinstance(other, (Euler, Quaternion)):
            m = other.to_matrix()
        else:
            m = other.to_3x3() if len(other) == 4 else other
        self._set((m @ Vector(self._v[:3]))._v + self._v[3:])

    def rotation_difference(self, other):
        a = Vector(self._v[:3]).normalized()
        b = Vector(self._other(other)[:3]).normalized()
        d = a.dot(b)
        if d < -1 + 1e-7:
            axis = a.orthogonal().normalized()
            return Quaternion(axis, math.pi)
        c = a.cross(b)
        q = Quaternion((1 + d, c.x, c.y, c.z))
        q.normalize()
        return q

    def to_track_quat(self, track='Z', up='Y'):
        """Blender's vec_to_quat: the rotation that points `track` along this vector."""
        axes = {'X': 0, 'Y': 1, 'Z': 2, '-X': 3, '-Y': 4, '-Z': 5,
                'TRACK_NEGATIVE_X': 3, 'TRACK_NEGATIVE_Y': 4, 'TRACK_NEGATIVE_Z': 5}
        t = axes[track.upper()] if isinstance(track, str) else track
        u = {'X': 0, 'Y': 1, 'Z': 2}[up.upper()]
        if t % 3 == u:
            raise ValueError('Vector.to_track_quat(): track and up axis must differ')
        v = Vector(self._v[:3])
        if v.length < 1e-10:
            return Quaternion()
        d = v.normalized()
        if t >= 3:
            d = -d
        ta = t % 3
        # Build an orthonormal frame: track axis along d, up axis as close as possible to world up.
        # The up axis leans towards world Z (world Y when tracking along Z).
        world_up = Vector((0.0, 0.0, 1.0))
        up_v = world_up - d * world_up.dot(d)
        if up_v.length < 1e-6:
            world_up = Vector((0.0, 1.0, 0.0))
            up_v = world_up - d * world_up.dot(d)
        up_v.normalize()
        cols = [None, None, None]
        cols[ta] = d
        cols[u] = up_v
        rest = 3 - ta - u
        # Right-handed: col[rest] = col[(rest+1)%3] x col[(rest+2)%3].
        cols[rest] = cols[(rest + 1) % 3].cross(cols[(rest + 2) % 3]).normalized()
        m = Matrix([[cols[c][r] for c in range(3)] for r in range(3)])
        return m.to_quaternion()


# ---------------------------------------------------------------------------- Matrix

class _MatrixRow:
    """`mat[i]`: a row that writes back into the matrix."""


class Matrix(_Owned):
    __slots__ = ('_m', '_owner', '_frozen')

    def __init__(self, rows=None, _owner=None):
        if rows is None:
            self._m = [[1.0 if r == c else 0.0 for c in range(4)] for r in range(4)]
        else:
            m = [_floats(r) for r in rows]
            if not m or not 2 <= len(m) <= 4 or any(len(r) != len(m[0]) for r in m) or not 2 <= len(m[0]) <= 4:
                raise ValueError('Matrix(): expects 2 to 4 rows of 2 to 4 numbers each')
            self._m = m
        self._owner = _owner
        self._frozen = False

    @property
    def row_count(self):
        return len(self._m)

    @property
    def col_count(self):
        return len(self._m[0])

    # --- constructors
    @classmethod
    def Identity(cls, size):
        return cls([[1.0 if r == c else 0.0 for c in range(size)] for r in range(size)])

    @classmethod
    def Diagonal(cls, vector):
        v = _floats(vector)
        return cls([[v[r] if r == c else 0.0 for c in range(len(v))] for r in range(len(v))])

    @classmethod
    def Translation(cls, vector):
        v = _floats(vector)
        m = cls.Identity(4)
        for i in range(min(3, len(v))):
            m._m[i][3] = v[i]
        return m

    @classmethod
    def Rotation(cls, angle, size, axis=None):
        c, s = math.cos(angle), math.sin(angle)
        if size == 2:
            m = [[c, -s], [s, c]]
            return cls(m)
        if isinstance(axis, str):
            a = axis.upper()
            if a == 'X':
                m = [[1, 0, 0], [0, c, -s], [0, s, c]]
            elif a == 'Y':
                m = [[c, 0, s], [0, 1, 0], [-s, 0, c]]
            elif a == 'Z':
                m = [[c, -s, 0], [s, c, 0], [0, 0, 1]]
            else:
                raise ValueError("Matrix.Rotation(): axis must be 'X', 'Y', 'Z' or a vector")
        else:
            if axis is None:
                raise ValueError('Matrix.Rotation(): a 3D or 4D matrix needs an axis')
            x, y, z = Vector(axis).normalized()._v[:3]
            t = 1 - c
            m = [[t * x * x + c, t * x * y - s * z, t * x * z + s * y],
                 [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
                 [t * x * z - s * y, t * y * z + s * x, t * z * z + c]]
        out = cls(m)
        return out.to_4x4() if size == 4 else out

    @classmethod
    def Scale(cls, factor, size, axis=None):
        if axis is None:
            m = cls.Identity(size)
            for i in range(min(size, 3)):
                m._m[i][i] = float(factor)
            return m
        n = Vector(axis).normalized()._v
        k = float(factor) - 1.0
        dim = 2 if size == 2 else 3
        m = [[(1.0 if r == c else 0.0) + k * n[r] * n[c] for c in range(dim)] for r in range(dim)]
        out = cls(m)
        return out.to_4x4() if size == 4 else out

    @classmethod
    def OrthoProjection(cls, axis, size):
        if isinstance(axis, str):
            keep = {'X': (0, 1, 1), 'Y': (1, 0, 1), 'Z': (1, 1, 0), 'XY': (1, 1, 0), 'XZ': (1, 0, 1), 'YZ': (0, 1, 1)}[axis.upper()]
            m = cls.Identity(size)
            for i in range(min(size, 3)):
                m._m[i][i] = float(keep[i])
            return m
        n = Vector(axis).normalized()._v
        dim = 2 if size == 2 else 3
        out = cls([[(1.0 if r == c else 0.0) - n[r] * n[c] for c in range(dim)] for r in range(dim)])
        return out.to_4x4() if size == 4 else out

    @classmethod
    def Shear(cls, plane, size, factor):
        m = cls.Identity(size)
        f = factor if isinstance(factor, (list, tuple, Vector)) else (factor, factor)
        if size == 2:
            m._m[0][1] = float(f[0] if isinstance(f, (list, tuple, Vector)) else f)
            return m
        if plane == 'XY':
            m._m[0][2], m._m[1][2] = f[0], f[1]
        elif plane == 'XZ':
            m._m[0][1], m._m[2][1] = f[0], f[1]
        elif plane == 'YZ':
            m._m[1][0], m._m[2][0] = f[0], f[1]
        return m

    @classmethod
    def LocRotScale(cls, location, rotation, scale):
        m = Matrix.Identity(3)
        if rotation is not None:
            if isinstance(rotation, (Quaternion, Euler)):
                m = rotation.to_matrix()
            else:
                m = Matrix(rotation).to_3x3()
        if scale is not None:
            s = _floats(scale)
            m = Matrix([[m._m[r][c] * s[c] for c in range(3)] for r in range(3)])
        m = m.to_4x4()
        if location is not None:
            m.translation = location
        return m

    # --- sequence protocol
    def __len__(self):
        return len(self._m)

    def __getitem__(self, i):
        if isinstance(i, tuple):
            return self._m[i[0]][i[1]]
        if isinstance(i, slice):
            return tuple(self[r] for r in range(len(self._m))[i])
        r = range(len(self._m))[i]

        def write(v, r=r):
            self._m[r] = list(v._v)
            self._changed()
        return Vector(self._m[r], _owner=write)

    def __setitem__(self, i, value):
        self._check_frozen()
        if isinstance(i, tuple):
            self._m[i[0]][i[1]] = float(value)
        else:
            v = _floats(value)
            if len(v) != self.col_count:
                raise ValueError('matrix row: expected %d values' % self.col_count)
            self._m[i] = v
        self._changed()

    def __iter__(self):
        return iter([self[i] for i in range(len(self._m))])

    @property
    def row(self):
        return [self[i] for i in range(len(self._m))]

    @property
    def col(self):
        out = []
        for c in range(self.col_count):
            def write(v, c=c):
                for r in range(len(self._m)):
                    self._m[r][c] = v._v[r]
                self._changed()
            out.append(Vector([self._m[r][c] for r in range(len(self._m))], _owner=write))
        return out

    @property
    def translation(self):
        def write(v):
            for i in range(3):
                self._m[i][3] = v._v[i]
            self._changed()
        if self.row_count < 4 or self.col_count < 4:
            raise AttributeError('Matrix.translation: only for 4x4 matrices')
        return Vector([self._m[0][3], self._m[1][3], self._m[2][3]], _owner=write)

    @translation.setter
    def translation(self, v):
        self._check_frozen()
        v = _floats(v)
        for i in range(3):
            self._m[i][3] = v[i]
        self._changed()

    @property
    def median_scale(self):
        s = self.to_scale()
        return sum(s) / 3.0

    @property
    def is_negative(self):
        return self.to_3x3().determinant() < 0

    @property
    def is_orthogonal(self):
        m = self.to_3x3()
        cols = [Vector([m._m[r][c] for r in range(3)]) for c in range(3)]
        return all(abs(cols[a].dot(cols[b])) < 1e-5 for a in range(3) for b in range(a + 1, 3))

    @property
    def is_orthogonal_axis_vectors(self):
        return self.is_orthogonal

    @property
    def is_identity(self):
        n = len(self._m)
        return n == self.col_count and all(abs(self._m[r][c] - (1.0 if r == c else 0.0)) < 1e-6 for r in range(n) for c in range(n))

    # --- arithmetic
    def __matmul__(self, o):
        if isinstance(o, Matrix):
            if self.col_count != o.row_count:
                raise ValueError('matrix @ matrix: %dx%d and %dx%d do not match' % (self.row_count, self.col_count, o.row_count, o.col_count))
            return Matrix([[sum(self._m[r][k] * o._m[k][c] for k in range(self.col_count)) for c in range(o.col_count)] for r in range(self.row_count)])
        if isinstance(o, (Vector, list, tuple)):
            v = _floats(o)
            n = len(v)
            if n == self.col_count:
                return Vector([sum(self._m[r][k] * v[k] for k in range(n)) for r in range(self.row_count)])
            if n == 3 and self.col_count == 4:
                v4 = v + [1.0]
                out = [sum(self._m[r][k] * v4[k] for k in range(4)) for r in range(4)]
                w = out[3]
                if w != 0 and w != 1:
                    return Vector([out[0] / w, out[1] / w, out[2] / w])
                return Vector(out[:3])
            if n == 2 and self.col_count == 3:
                v3 = v + [1.0]
                return Vector([sum(self._m[r][k] * v3[k] for k in range(3)) for r in range(2)])
            raise ValueError('matrix @ vector: a %dx%d matrix and a %d vector do not match' % (self.row_count, self.col_count, n))
        if isinstance(o, Quaternion):
            return self @ o.to_matrix().to_4x4() if self.row_count == 4 else self @ o.to_matrix()
        return NotImplemented

    def __imatmul__(self, o):
        r = self @ o
        self._m = r._m
        self._changed()
        return self

    def __mul__(self, o):
        if isinstance(o, (int, float)):
            return Matrix([[x * o for x in row] for row in self._m])
        if isinstance(o, Matrix):
            return Matrix([[a * b for a, b in zip(r1, r2)] for r1, r2 in zip(self._m, o._m)])
        return NotImplemented

    def __rmul__(self, o):
        if isinstance(o, (int, float)):
            return self * o
        return NotImplemented

    def __add__(self, o):
        return Matrix([[a + b for a, b in zip(r1, r2)] for r1, r2 in zip(self._m, o._m)])

    def __sub__(self, o):
        return Matrix([[a - b for a, b in zip(r1, r2)] for r1, r2 in zip(self._m, o._m)])

    def __neg__(self):
        return self * -1.0

    def __invert__(self):
        return self.inverted()

    def __eq__(self, o):
        if isinstance(o, Matrix):
            return self._m == o._m
        return NotImplemented

    def __ne__(self, o):
        r = self.__eq__(o)
        return r if r is NotImplemented else not r

    def __hash__(self):
        if not self._frozen:
            raise TypeError("unhashable type: 'Matrix' (freeze() it first)")
        return hash(tuple(tuple(r) for r in self._m))

    def __repr__(self):
        return 'Matrix((%s))' % ',\n        '.join('(%s)' % ', '.join('%.4f' % x for x in r) for r in self._m)

    def __str__(self):
        return '<Matrix %dx%d (%s)>' % (self.row_count, self.col_count, ')\n            ('.join(', '.join('%.4f' % x for x in r) for r in self._m))

    def __copy__(self):
        return self.copy()

    def __deepcopy__(self, memo):
        return self.copy()

    def __reduce__(self):
        return (Matrix, (tuple(tuple(r) for r in self._m),))

    # --- methods
    def copy(self):
        return Matrix(self._m)

    def _assign(self, m):
        self._m = [list(r) for r in m._m]
        self._changed()

    def zero(self):
        self._assign(Matrix([[0.0] * self.col_count for _ in self._m]))
        return self

    def identity(self):
        self._assign(Matrix.Identity(len(self._m)))

    def transposed(self):
        return Matrix([[self._m[r][c] for r in range(self.row_count)] for c in range(self.col_count)])

    def transpose(self):
        self._assign(self.transposed())

    def determinant(self):
        return _det(self._m)

    def adjugated(self):
        n = len(self._m)
        if n == 2:
            a, b = self._m[0]
            c, d = self._m[1]
            return Matrix([[d, -b], [-c, a]])
        cof = [[(-1) ** (r + c) * _det([row[:c] + row[c + 1:] for k, row in enumerate(self._m) if k != r]) for c in range(n)] for r in range(n)]
        return Matrix(cof).transposed()

    def adjugate(self):
        self._assign(self.adjugated())

    def inverted(self, fallback=None):
        if self.row_count != self.col_count:
            raise ValueError('Matrix.inverted(): only square matrices')
        d = self.determinant()
        if abs(d) < 1e-30:
            if fallback is not None:
                return fallback
            raise ValueError('Matrix.inverted(): matrix does not have an inverse')
        return self.adjugated() * (1.0 / d)

    def invert(self, fallback=None):
        if fallback is not None and abs(self.determinant()) < 1e-30:
            self._assign(fallback)
            return
        self._assign(self.inverted())

    def inverted_safe(self):
        d = self.determinant()
        if abs(d) > 1e-30:
            return self.inverted()
        m = self.copy()
        n = len(m._m)
        eps = 1e-5
        for i in range(n):
            m._m[i][i] += eps
        return m.inverted(Matrix.Identity(n))

    def invert_safe(self):
        self._assign(self.inverted_safe())

    def normalized(self):
        m = self.copy()
        for c in range(min(3, self.col_count)):
            col = Vector([m._m[r][c] for r in range(min(3, self.row_count))])
            l = col.length
            if l:
                for r in range(min(3, self.row_count)):
                    m._m[r][c] /= l
        return m

    def normalize(self):
        self._assign(self.normalized())

    def to_2x2(self):
        return Matrix([row[:2] for row in self._m[:2]])

    def to_3x3(self):
        rows = [(r + [0.0, 0.0, 0.0])[:3] for r in self._m[:3]]
        while len(rows) < 3:
            rows.append([1.0 if c == len(rows) else 0.0 for c in range(3)])
        if len(self._m) == 2:
            rows[2] = [0.0, 0.0, 1.0]
        return Matrix(rows)

    def to_4x4(self):
        m = Matrix.Identity(4)
        for r in range(min(4, self.row_count)):
            for c in range(min(4, self.col_count)):
                m._m[r][c] = self._m[r][c]
        return m

    def resize_4x4(self):
        self._m = self.to_4x4()._m

    def to_translation(self):
        return Vector([self._m[0][3], self._m[1][3], self._m[2][3]])

    def to_scale(self):
        m = self.to_3x3()
        s = [Vector([m._m[r][c] for r in range(3)]).length for c in range(3)]
        if m.determinant() < 0:
            s = [-x for x in s]
        return Vector(s)

    def to_quaternion(self):
        m = self.to_3x3().normalized()._m
        return _mat3_to_quat(m)

    def to_euler(self, order='XYZ', euler_compat=None):
        m = self.to_3x3().normalized()._m
        e = _mat3_to_euler(m, order)
        if euler_compat is not None:
            e = _euler_compat(e, list(euler_compat), m, order)
        return Euler(e, order)

    def decompose(self):
        loc = self.to_translation()
        m3 = self.to_3x3()
        scale = m3.to_scale()
        rot = Matrix([[m3._m[r][c] / (scale[c] if scale[c] else 1.0) for c in range(3)] for r in range(3)])
        return loc, _mat3_to_quat(rot._m), scale

    def lerp(self, other, factor):
        return Matrix([[a + (b - a) * factor for a, b in zip(r1, r2)] for r1, r2 in zip(self._m, other._m)])

    def rotate(self, other):
        r = other.to_matrix() if isinstance(other, (Euler, Quaternion)) else other.to_3x3()
        self._assign((r @ self.to_3x3()).to_4x4() if len(self._m) == 4 else r @ self)


def _det(m):
    n = len(m)
    if n == 1:
        return m[0][0]
    if n == 2:
        return m[0][0] * m[1][1] - m[0][1] * m[1][0]
    return sum((-1) ** c * m[0][c] * _det([row[:c] + row[c + 1:] for row in m[1:]]) for c in range(n))


def _mat3_to_quat(m):
    """Blender's mat3_normalized_to_quat (m is row-major)."""
    tr = m[0][0] + m[1][1] + m[2][2]
    if tr > 0:
        s = 2.0 * math.sqrt(1.0 + tr)
        w = 0.25 * s
        x = (m[2][1] - m[1][2]) / s
        y = (m[0][2] - m[2][0]) / s
        z = (m[1][0] - m[0][1]) / s
    elif m[0][0] > m[1][1] and m[0][0] > m[2][2]:
        s = 2.0 * math.sqrt(1.0 + m[0][0] - m[1][1] - m[2][2])
        w = (m[2][1] - m[1][2]) / s
        x = 0.25 * s
        y = (m[0][1] + m[1][0]) / s
        z = (m[0][2] + m[2][0]) / s
    elif m[1][1] > m[2][2]:
        s = 2.0 * math.sqrt(1.0 + m[1][1] - m[0][0] - m[2][2])
        w = (m[0][2] - m[2][0]) / s
        x = (m[0][1] + m[1][0]) / s
        y = 0.25 * s
        z = (m[1][2] + m[2][1]) / s
    else:
        s = 2.0 * math.sqrt(1.0 + m[2][2] - m[0][0] - m[1][1])
        w = (m[1][0] - m[0][1]) / s
        x = (m[0][2] + m[2][0]) / s
        y = (m[1][2] + m[2][1]) / s
        z = 0.25 * s
    if w < 0:
        w, x, y, z = -w, -x, -y, -z
    q = Quaternion((w, x, y, z))
    q.normalize()
    return q


# Blender's euler order table: axes (i, j, k) and parity.
_ORDERS = {'XYZ': (0, 1, 2, 0), 'XZY': (0, 2, 1, 1), 'YXZ': (1, 0, 2, 1),
           'YZX': (1, 2, 0, 0), 'ZXY': (2, 0, 1, 0), 'ZYX': (2, 1, 0, 1)}


def _order(order):
    o = str(order).upper()
    if o not in _ORDERS:
        raise ValueError("Euler order must be one of 'XYZ', 'XZY', 'YXZ', 'YZX', 'ZXY', 'ZYX', not %r" % (order,))
    return o


def _euler_to_mat3(e, order):
    """Blender's eulO_to_mat3; returns a row-major 3x3 list."""
    i, j, k, parity = _ORDERS[_order(order)]
    if parity:
        ti, tj, th = -e[i], -e[j], -e[k]
    else:
        ti, tj, th = e[i], e[j], e[k]
    ci, cj, ch = math.cos(ti), math.cos(tj), math.cos(th)
    si, sj, sh = math.sin(ti), math.sin(tj), math.sin(th)
    cc, cs, sc, ss = ci * ch, ci * sh, si * ch, si * sh
    # col-major mat[col][row]
    mat = [[0.0] * 3 for _ in range(3)]
    mat[i][i] = cj * ch
    mat[j][i] = sj * sc - cs
    mat[k][i] = sj * cc + ss
    mat[i][j] = cj * sh
    mat[j][j] = sj * ss + cc
    mat[k][j] = sj * cs - sc
    mat[i][k] = -sj
    mat[j][k] = cj * si
    mat[k][k] = cj * ci
    return [[mat[c][r] for c in range(3)] for r in range(3)]


def _mat3_to_euler2(m, order):
    i, j, k, parity = _ORDERS[_order(order)]
    mat = [[m[r][c] for r in range(3)] for c in range(3)]  # col-major
    cy = math.hypot(mat[i][i], mat[i][j])
    if cy > 16.0 * 1.1920929e-07:
        e1 = [0.0] * 3
        e2 = [0.0] * 3
        e1[i] = math.atan2(mat[j][k], mat[k][k])
        e1[j] = math.atan2(-mat[i][k], cy)
        e1[k] = math.atan2(mat[i][j], mat[i][i])
        e2[i] = math.atan2(-mat[j][k], -mat[k][k])
        e2[j] = math.atan2(-mat[i][k], -cy)
        e2[k] = math.atan2(-mat[i][j], -mat[i][i])
    else:
        e1 = [0.0] * 3
        e1[i] = math.atan2(-mat[k][j], mat[j][j])
        e1[j] = math.atan2(-mat[i][k], cy)
        e1[k] = 0.0
        e2 = list(e1)
    if parity:
        e1 = [-x for x in e1]
        e2 = [-x for x in e2]
    return e1, e2


def _mat3_to_euler(m, order):
    e1, e2 = _mat3_to_euler2(m, order)
    if sum(abs(x) for x in e1) > sum(abs(x) for x in e2):
        return e2
    return e1


def _compat(e, old):
    """Blender's compatible_eul."""
    pi_thresh = 5.1
    pi_x2 = 2 * math.pi
    e = list(e)
    for i in range(3):
        d = e[i] - old[i]
        if d > pi_thresh:
            e[i] -= math.floor(d / pi_x2 + 0.5) * pi_x2
        elif d < -pi_thresh:
            e[i] += math.floor(-d / pi_x2 + 0.5) * pi_x2
    for i in range(3):
        d = e[i] - old[i]
        if abs(d) > 3.2:
            e[i] += -pi_x2 if d > 0 else pi_x2
    return e


def _euler_compat(e, old, m, order):
    e1, e2 = _mat3_to_euler2(m, order)
    e1 = _compat(e1, old)
    e2 = _compat(e2, old)
    d1 = sum(abs(a - b) for a, b in zip(e1, old))
    d2 = sum(abs(a - b) for a, b in zip(e2, old))
    return e2 if d1 > d2 else e1


# ---------------------------------------------------------------------------- Quaternion

class Quaternion(_Owned):
    __slots__ = ('_q', '_owner', '_frozen')

    def __init__(self, seq=(1.0, 0.0, 0.0, 0.0), angle=None, _owner=None):
        if angle is not None:
            axis = Vector(seq).normalized()
            h = angle / 2.0
            s = math.sin(h)
            self._q = [math.cos(h), axis[0] * s, axis[1] * s, axis[2] * s]
        else:
            q = _floats(seq)
            if len(q) == 3:
                # Exponential map.
                v = Vector(q)
                ang = v.length
                if ang == 0:
                    q = [1.0, 0.0, 0.0, 0.0]
                else:
                    s = math.sin(ang / 2) / ang
                    q = [math.cos(ang / 2)] + [x * s for x in q]
            if len(q) != 4:
                raise ValueError('Quaternion(): expected 4 values (w, x, y, z)')
            self._q = q
        self._owner = _owner
        self._frozen = False

    def _set(self, q):
        self._q[:] = _floats(q)
        self._changed()

    def _comp(i):
        def get(self):
            return self._q[i]

        def put(self, v):
            self._check_frozen()
            self._q[i] = float(v)
            self._changed()
        return property(get, put)

    w = _comp(0)
    x = _comp(1)
    y = _comp(2)
    z = _comp(3)
    del _comp

    def __len__(self):
        return 4

    def __iter__(self):
        return iter(list(self._q))

    def __getitem__(self, i):
        if isinstance(i, slice):
            return tuple(self._q[i])
        return self._q[i]

    def __setitem__(self, i, v):
        self._check_frozen()
        if isinstance(i, slice):
            self._q[i] = _floats(v)
        else:
            self._q[i] = float(v)
        self._changed()

    @property
    def magnitude(self):
        return math.sqrt(sum(x * x for x in self._q))

    @property
    def angle(self):
        q = self.normalized()._q
        return 2.0 * math.acos(max(-1.0, min(1.0, q[0])))

    @angle.setter
    def angle(self, a):
        axis = self.axis
        self._set(Quaternion(axis, a)._q)

    @property
    def axis(self):
        q = self.normalized()._q
        s = math.sqrt(max(0.0, 1.0 - q[0] * q[0]))
        if s < 1e-7:
            return Vector((1.0, 0.0, 0.0))
        return Vector((q[1] / s, q[2] / s, q[3] / s))

    @axis.setter
    def axis(self, v):
        self._set(Quaternion(v, self.angle)._q)

    def copy(self):
        return Quaternion(self._q)

    __copy__ = copy

    def __deepcopy__(self, memo):
        return self.copy()

    def __reduce__(self):
        return (Quaternion, (tuple(self._q),))

    def __repr__(self):
        return 'Quaternion((%s))' % ', '.join('%.4f' % x for x in self._q)

    def __str__(self):
        return '<Quaternion (w=%.4f, x=%.4f, y=%.4f, z=%.4f)>' % tuple(self._q)

    def __eq__(self, o):
        if isinstance(o, Quaternion):
            return self._q == o._q
        return NotImplemented

    def __ne__(self, o):
        r = self.__eq__(o)
        return r if r is NotImplemented else not r

    def __hash__(self):
        if not self._frozen:
            raise TypeError("unhashable type: 'Quaternion'")
        return hash(tuple(self._q))

    def __neg__(self):
        return Quaternion([-x for x in self._q])

    def __add__(self, o):
        return Quaternion([a + b for a, b in zip(self._q, o._q)])

    def __sub__(self, o):
        return Quaternion([a - b for a, b in zip(self._q, o._q)])

    def __mul__(self, o):
        if isinstance(o, (int, float)):
            return Quaternion([x * o for x in self._q])
        if isinstance(o, Quaternion):
            return Quaternion([a * b for a, b in zip(self._q, o._q)])
        return NotImplemented

    def __rmul__(self, o):
        if isinstance(o, (int, float)):
            return self * o
        return NotImplemented

    def __truediv__(self, o):
        return Quaternion([x / o for x in self._q])

    def __matmul__(self, o):
        if isinstance(o, Quaternion):
            a, b = self._q, o._q
            return Quaternion((
                a[0] * b[0] - a[1] * b[1] - a[2] * b[2] - a[3] * b[3],
                a[0] * b[1] + a[1] * b[0] + a[2] * b[3] - a[3] * b[2],
                a[0] * b[2] - a[1] * b[3] + a[2] * b[0] + a[3] * b[1],
                a[0] * b[3] + a[1] * b[2] - a[2] * b[1] + a[3] * b[0]))
        if isinstance(o, (Vector, list, tuple)):
            v = Vector(o)
            if len(v) != 3:
                raise ValueError('quaternion @ vector: a 3D vector is needed')
            return self.to_matrix() @ v
        return NotImplemented

    def __imatmul__(self, o):
        self._set((self @ o)._q)
        return self

    def dot(self, o):
        return sum(a * b for a, b in zip(self._q, Quaternion(o)._q))

    def cross(self, o):
        return self @ o

    def normalized(self):
        m = self.magnitude
        return Quaternion([x / m for x in self._q]) if m else Quaternion()

    def normalize(self):
        m = self.magnitude
        if m:
            self._set([x / m for x in self._q])

    def conjugated(self):
        w, x, y, z = self._q
        return Quaternion((w, -x, -y, -z))

    def conjugate(self):
        self._set(self.conjugated()._q)

    def inverted(self):
        d = sum(x * x for x in self._q)
        if d == 0:
            raise ValueError('Quaternion.inverted(): zero quaternion')
        return Quaternion([x / d for x in self.conjugated()._q])

    def invert(self):
        self._set(self.inverted()._q)

    def identity(self):
        self._set((1.0, 0.0, 0.0, 0.0))
        return self

    def negate(self):
        self._set([-x for x in self._q])
        return self

    def to_matrix(self):
        w, x, y, z = self._q
        # Blender scales by sqrt(2) internally; for unit quaternions this is the usual matrix.
        return Matrix((
            (1 - 2 * (y * y + z * z), 2 * (x * y - w * z), 2 * (x * z + w * y)),
            (2 * (x * y + w * z), 1 - 2 * (x * x + z * z), 2 * (y * z - w * x)),
            (2 * (x * z - w * y), 2 * (y * z + w * x), 1 - 2 * (x * x + y * y))))

    def to_euler(self, order='XYZ', euler_compat=None):
        return self.normalized().to_matrix().to_euler(order, euler_compat)

    def to_axis_angle(self):
        return self.axis, self.angle

    def to_exponential_map(self):
        axis, angle = self.to_axis_angle()
        return axis * angle

    def to_swing_twist(self, axis):
        a = {'X': 1, 'Y': 2, 'Z': 3}[axis.upper()]
        q = self.normalized()._q
        t = [q[0], 0.0, 0.0, 0.0]
        t[a] = q[a]
        twist = Quaternion(t).normalized()
        swing = self @ twist.conjugated()
        ang = 2.0 * math.atan2(q[a], q[0])
        return swing, ang

    def rotation_difference(self, other):
        return self.normalized().conjugated() @ Quaternion(other).normalized()

    def slerp(self, other, factor):
        a = self.normalized()._q
        b = Quaternion(other).normalized()._q
        d = sum(x * y for x, y in zip(a, b))
        if d < 0:
            d = -d
            b = [-x for x in b]
        if d > 0.9995:
            r = Quaternion([x + (y - x) * factor for x, y in zip(a, b)])
            r.normalize()
            return r
        th = math.acos(d)
        s = math.sin(th)
        wa = math.sin((1 - factor) * th) / s
        wb = math.sin(factor * th) / s
        return Quaternion([x * wa + y * wb for x, y in zip(a, b)])

    def rotate(self, other):
        q = other.to_quaternion() if not isinstance(other, Quaternion) else other
        self._set((q @ self)._q)

    def make_compatible(self, other):
        if self.dot(other) < 0:
            self.negate()


# ---------------------------------------------------------------------------- Euler

class Euler(_Owned):
    __slots__ = ('_e', '_order', '_owner', '_frozen')

    def __init__(self, angles=(0.0, 0.0, 0.0), order='XYZ', _owner=None):
        e = _floats(angles)
        if len(e) != 3:
            raise ValueError('Euler(): expected 3 angles')
        self._e = e
        self._order = _order(order)
        self._owner = _owner
        self._frozen = False

    def _set(self, e):
        self._e[:] = _floats(e)
        self._changed()

    def _comp(i):
        def get(self):
            return self._e[i]

        def put(self, v):
            self._check_frozen()
            self._e[i] = float(v)
            self._changed()
        return property(get, put)

    x = _comp(0)
    y = _comp(1)
    z = _comp(2)
    del _comp

    @property
    def order(self):
        return self._order

    @order.setter
    def order(self, o):
        self._order = _order(o)
        self._changed()

    def __len__(self):
        return 3

    def __iter__(self):
        return iter(list(self._e))

    def __getitem__(self, i):
        if isinstance(i, slice):
            return tuple(self._e[i])
        return self._e[i]

    def __setitem__(self, i, v):
        self._check_frozen()
        if isinstance(i, slice):
            self._e[i] = _floats(v)
        else:
            self._e[i] = float(v)
        self._changed()

    def __getattr__(self, name):
        if name.startswith('_') or len(name) < 2 or len(name) > 3 or any(c not in 'xyz' for c in name):
            raise AttributeError("'Euler' object has no attribute '%s'" % name)
        return Vector([self._e[_SWZ[c]] for c in name])

    def __eq__(self, o):
        if isinstance(o, Euler):
            return self._e == o._e and self._order == o._order
        return NotImplemented

    def __ne__(self, o):
        r = self.__eq__(o)
        return r if r is NotImplemented else not r

    def __hash__(self):
        if not self._frozen:
            raise TypeError("unhashable type: 'Euler'")
        return hash((tuple(self._e), self._order))

    def __add__(self, o):
        return Euler([a + b for a, b in zip(self._e, o)], self._order)

    def __sub__(self, o):
        return Euler([a - b for a, b in zip(self._e, o)], self._order)

    def __mul__(self, o):
        if isinstance(o, (int, float)):
            return Euler([a * o for a in self._e], self._order)
        return NotImplemented

    __rmul__ = __mul__

    def __iadd__(self, o):
        self._set([a + b for a, b in zip(self._e, o)])
        return self

    def __isub__(self, o):
        self._set([a - b for a, b in zip(self._e, o)])
        return self

    def __imul__(self, o):
        self._set([a * o for a in self._e])
        return self

    def __neg__(self):
        return Euler([-a for a in self._e], self._order)

    def __repr__(self):
        return "Euler((%s), '%s')" % (', '.join('%.4f' % x for x in self._e), self._order)

    def __str__(self):
        return "<Euler (x=%.4f, y=%.4f, z=%.4f), order='%s'>" % (self._e[0], self._e[1], self._e[2], self._order)

    def copy(self):
        return Euler(self._e, self._order)

    __copy__ = copy

    def __deepcopy__(self, memo):
        return self.copy()

    def __reduce__(self):
        return (Euler, (tuple(self._e), self._order))

    def to_tuple(self, precision=-1):
        return tuple(self._e) if precision < 0 else tuple(round(x, precision) for x in self._e)

    def to_matrix(self):
        return Matrix(_euler_to_mat3(self._e, self._order))

    def to_quaternion(self):
        return _mat3_to_quat(_euler_to_mat3(self._e, self._order))

    def zero(self):
        self._set((0.0, 0.0, 0.0))

    def rotate_axis(self, axis, angle):
        a = axis.upper()
        r = Matrix.Rotation(angle, 3, a)
        m = self.to_matrix() @ r
        self._set(m.to_euler(self._order, self)._e)

    def rotate(self, other):
        r = other.to_matrix() if isinstance(other, (Euler, Quaternion)) else Matrix(other).to_3x3()
        m = r @ self.to_matrix()
        self._set(m.to_euler(self._order, self)._e)

    def make_compatible(self, other):
        self._set(_compat(self._e, list(other)))


# ---------------------------------------------------------------------------- Color

class Color(_Owned):
    __slots__ = ('_c', '_owner', '_frozen')

    def __init__(self, rgb=(0.0, 0.0, 0.0), _owner=None):
        c = _floats(rgb)
        if len(c) != 3:
            raise ValueError('Color(): expected 3 values (r, g, b)')
        self._c = c
        self._owner = _owner
        self._frozen = False

    def _set(self, c):
        self._c[:] = _floats(c)
        self._changed()

    def _comp(i):
        def get(self):
            return self._c[i]

        def put(self, v):
            self._check_frozen()
            self._c[i] = float(v)
            self._changed()
        return property(get, put)

    r = _comp(0)
    g = _comp(1)
    b = _comp(2)
    del _comp

    @property
    def hsv(self):
        return _rgb_to_hsv(*self._c)

    @hsv.setter
    def hsv(self, v):
        self._set(_hsv_to_rgb(*v))

    def _hsvc(i):
        def get(self):
            return self.hsv[i]

        def put(self, v):
            h = list(self.hsv)
            h[i] = float(v)
            self.hsv = h
        return property(get, put)

    h = _hsvc(0)
    s = _hsvc(1)
    v = _hsvc(2)
    del _hsvc

    def __len__(self):
        return 3

    def __iter__(self):
        return iter(list(self._c))

    def __getitem__(self, i):
        if isinstance(i, slice):
            return tuple(self._c[i])
        return self._c[i]

    def __setitem__(self, i, v):
        self._check_frozen()
        if isinstance(i, slice):
            self._c[i] = _floats(v)
        else:
            self._c[i] = float(v)
        self._changed()

    def __eq__(self, o):
        if isinstance(o, Color):
            return self._c == o._c
        if isinstance(o, (list, tuple)):
            return list(o) == self._c
        return NotImplemented

    def __ne__(self, o):
        r = self.__eq__(o)
        return r if r is NotImplemented else not r

    def __hash__(self):
        if not self._frozen:
            raise TypeError("unhashable type: 'Color'")
        return hash(tuple(self._c))

    def __add__(self, o):
        return Color([a + b for a, b in zip(self._c, o)])

    def __sub__(self, o):
        return Color([a - b for a, b in zip(self._c, o)])

    def __mul__(self, o):
        if isinstance(o, (int, float)):
            return Color([a * o for a in self._c])
        return NotImplemented

    __rmul__ = __mul__

    def __truediv__(self, o):
        return Color([a / o for a in self._c])

    def __iadd__(self, o):
        self._set([a + b for a, b in zip(self._c, o)])
        return self

    def __isub__(self, o):
        self._set([a - b for a, b in zip(self._c, o)])
        return self

    def __imul__(self, o):
        self._set([a * o for a in self._c])
        return self

    def __neg__(self):
        return Color([-a for a in self._c])

    def __repr__(self):
        return 'Color((%s))' % ', '.join('%.4f' % x for x in self._c)

    def __str__(self):
        return '<Color (r=%.4f, g=%.4f, b=%.4f)>' % tuple(self._c)

    def copy(self):
        return Color(self._c)

    __copy__ = copy

    def __deepcopy__(self, memo):
        return self.copy()

    def __reduce__(self):
        return (Color, (tuple(self._c),))

    def from_scene_linear_to_srgb(self):
        return Color([_lin_to_srgb(c) for c in self._c])

    def from_srgb_to_scene_linear(self):
        return Color([_srgb_to_lin(c) for c in self._c])

    def from_scene_linear_to_xyz_d65(self):
        r, g, b = self._c
        return Color((0.4124 * r + 0.3576 * g + 0.1805 * b, 0.2126 * r + 0.7152 * g + 0.0722 * b, 0.0193 * r + 0.1192 * g + 0.9505 * b))

    def from_xyz_d65_to_scene_linear(self):
        x, y, z = self._c
        return Color((3.2406 * x - 1.5372 * y - 0.4986 * z, -0.9689 * x + 1.8758 * y + 0.0415 * z, 0.0557 * x - 0.2040 * y + 1.0570 * z))

    def from_scene_linear_to_aces(self):
        return self.copy()

    def from_aces_to_scene_linear(self):
        return self.copy()


def _lin_to_srgb(c):
    return c * 12.92 if c < 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055 if c > 0 else 0.0


def _srgb_to_lin(c):
    return c / 12.92 if c < 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def _rgb_to_hsv(r, g, b):
    """Blender's rgb_to_hsv (h in 0..1)."""
    mx, mn = max(r, g, b), min(r, g, b)
    v = mx
    d = mx - mn
    s = d / mx if mx > 0 else 0.0
    if s == 0 or d == 0:
        return (0.0, 0.0, v)
    if r == mx:
        h = (g - b) / d
    elif g == mx:
        h = 2.0 + (b - r) / d
    else:
        h = 4.0 + (r - g) / d
    h /= 6.0
    if h < 0:
        h += 1.0
    return (h, s, v)


def _hsv_to_rgb(h, s, v):
    h = (h % 1.0) * 6.0
    i = int(h)
    f = h - i
    p, q, t = v * (1 - s), v * (1 - s * f), v * (1 - s * (1 - f))
    return [(v, t, p), (q, v, p), (p, v, t), (p, q, v), (t, p, v), (v, p, q)][i % 6]


from . import geometry, interpolate, kdtree, bvhtree, noise  # noqa: E402,F401
