# SPDX-License-Identifier: GPL-3.0-only
"""Small arrays that write through when an item changes (bpy_prop_array)."""


class FloatArray(list):
    """`obj.prop[1] = x` changes the property: `write(values)` runs after it."""

    _conv = float

    def __init__(self, values, write=None):
        super().__init__(self._conv(v) for v in values)
        self._write = write

    def __setitem__(self, i, v):
        if isinstance(i, slice):
            super().__setitem__(i, [self._conv(x) for x in v])
        else:
            super().__setitem__(i, self._conv(v))
        if self._write is not None:
            self._write(list(self))

    def __repr__(self):
        return 'bpy_prop_array(%s)' % list.__repr__(self)

    def __iadd__(self, other):
        for i, v in enumerate(other):
            list.__setitem__(self, i, self[i] + v)
        if self._write is not None:
            self._write(list(self))
        return self


class BoolArray(FloatArray):
    _conv = bool


class IntArray(FloatArray):
    _conv = int
