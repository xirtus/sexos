# SPDX-License-Identifier: GPL-3.0-only
"""The polyops stream: how API calls leave this Python process.

Polyamour executes the lines with crates/sex-3d/src/script.rs; the language is
described there and in docs/handoff/POLYAMOUR_PYTHON_2026_11_14.md. This module
knows the lexical rules (quoting, numbers, lists, base64 arrays), writes the
stream and reads polyops text back (for the scene snapshot Polyamour hands a
script through POLYAMOUR_SCENE).

Where the lines go: the file named by the POLYAMOUR_OPS environment variable, or
else standard output. In the standard-output case the script's own print()
output is moved to standard error so it cannot mix with commands. The stream
starts with `polyops 1` and is closed at exit with `end`, or with `abort ...`
when the script died of an exception, so Polyamour applies all or nothing.
"""
import atexit
import base64
import os
import struct
import sys

VERSION = 1
# Meshes with more vertices than this are sent as base64 arrays.
BASE64_ABOVE = 64
_SAFE = set("_-.+:/!?%&*<>^~'[]()")


class Raw(str):
    """Text written into a line as it is (handles, keywords, encoded arrays)."""


def quote(s):
    """`s` as one polyops token: bare when safe, else double-quoted."""
    s = str(s)
    if s and s != 'none' and s[0] not in '$#@' and all(c.isalnum() or c in _SAFE for c in s):
        return s
    out = ['"']
    for c in s:
        out.append({'"': '\\"', '\\': '\\\\', '\n': '\\n', '\t': '\\t', '\r': '\\r'}.get(c, c))
    out.append('"')
    return ''.join(out)


def fmt_num(v):
    """A number as polyops text: ints exact, floats with f32 precision."""
    if isinstance(v, bool):
        return '1' if v else '0'
    if isinstance(v, int):
        return str(v)
    f = float(v)
    if f != f or f in (float('inf'), float('-inf')):
        raise ValueError('polyops numbers must be finite, got %r' % (v,))
    return '%.9g' % f


def fmt_list(seq):
    return ','.join(fmt_num(x) for x in seq)


def token(v):
    """Format one argument or option value."""
    if isinstance(v, Raw):
        return str(v)
    if isinstance(v, str):
        return quote(v)
    if isinstance(v, (bool, int, float)):
        return fmt_num(v)
    if hasattr(v, '_ref'):
        return v._ref()
    return fmt_list(v)


def b64_floats(values):
    return Raw('b64:' + base64.b64encode(struct.pack('<%df' % len(values), *values)).decode('ascii'))


def b64_uints(values):
    return Raw('b64:' + base64.b64encode(struct.pack('<%dI' % len(values), *values)).decode('ascii'))


def mesh_options(verts, edges, faces):
    """Options `verts= faces= edges=` for mesh_new / mesh_set (Blender's
    from_pydata arguments). Edges already used by a face are dropped: polyops
    only stores loose edges."""
    verts = [tuple(float(c) for c in v) for v in verts]
    for v in verts:
        if len(v) != 3:
            raise ValueError('a vertex needs 3 coordinates, got %r' % (v,))
    n = len(verts)
    faces = [[int(i) for i in f] for f in faces]
    for f in faces:
        if len(f) < 3:
            raise ValueError('a face needs at least 3 vertices, got %r' % (f,))
        if any(i < 0 or i >= n for i in f):
            raise ValueError('face %r uses a vertex outside 0..%d' % (f, n - 1))
    used = set()
    for f in faces:
        for a, b in zip(f, f[1:] + f[:1]):
            used.add((min(a, b), max(a, b)))
    loose = []
    for e in edges:
        a, b = (int(i) for i in e)
        if not (0 <= a < n and 0 <= b < n):
            raise ValueError('edge %r uses a vertex outside 0..%d' % (tuple(e), n - 1))
        if (min(a, b), max(a, b)) not in used and a != b:
            loose.append((a, b))
    flat = [c for v in verts for c in v]
    opts = {}
    if n > BASE64_ABOVE:
        opts['verts'] = b64_floats(flat)
        if faces:
            opts['faces'] = b64_uints([x for f in faces for x in [len(f)] + f])
        if loose:
            opts['edges'] = b64_uints([x for e in loose for x in e])
    else:
        opts['verts'] = Raw(fmt_list(flat))
        if faces:
            opts['faces'] = Raw(';'.join(','.join(str(i) for i in f) for f in faces))
        if loose:
            opts['edges'] = Raw(';'.join('%d,%d' % e for e in loose))
    return opts


# --- reading -------------------------------------------------------------------

def tokenize(line):
    """Split a line into (key, text, quoted) tuples, as script.rs does."""
    out = []
    i, n = 0, len(line)
    while True:
        while i < n and line[i].isspace():
            i += 1
        if i >= n or line[i] == '#':
            return out
        key, text, quoted = None, [], False
        while i < n and not line[i].isspace():
            c = line[i]
            i += 1
            if c == '"':
                quoted = True
                while True:
                    if i >= n:
                        raise ValueError('unterminated quoted string')
                    c = line[i]
                    i += 1
                    if c == '"':
                        break
                    if c == '\\':
                        if i >= n:
                            raise ValueError('unterminated quoted string')
                        e = line[i]
                        i += 1
                        if e not in '"\\ntr':
                            raise ValueError('unknown escape \\' + e)
                        c = {'n': '\n', 't': '\t', 'r': '\r'}.get(e, e)
                    text.append(c)
            elif c == '=' and key is None and not quoted and ''.join(text).isidentifier() and ''.join(text).isascii():
                key, text = ''.join(text), []
            else:
                text.append(c)
        out.append((key, ''.join(text), quoted))


def floats(text):
    if text.startswith('b64:'):
        raw = base64.b64decode(text[4:])
        return list(struct.unpack('<%df' % (len(raw) // 4), raw))
    return [float(x) for x in text.split(',')] if text.strip() else []


def uints(text):
    if text.startswith('b64:'):
        raw = base64.b64decode(text[4:])
        return list(struct.unpack('<%dI' % (len(raw) // 4), raw))
    return [int(x) for x in text.split(',')] if text.strip() else []


def polygons(text):
    if text.startswith('b64:'):
        flat, out, i = uints(text), [], 0
        while i < len(flat):
            out.append(flat[i + 1:i + 1 + flat[i]])
            i += 1 + flat[i]
        return out
    return [uints(f) for f in text.split(';') if f.strip()]


def read_commands(text):
    """Yield (command, positional texts, options dict) for each command line of
    a polyops program, header and comments skipped."""
    for line in text.splitlines():
        toks = tokenize(line)
        if not toks:
            continue
        if toks[0][1].startswith('@') and not toks[0][2]:
            toks = toks[1:]
        name = toks[0][1]
        if name == 'polyops':
            continue
        pos = [t[1] for t in toks[1:] if t[0] is None]
        opts = {t[0]: t[1] for t in toks[1:] if t[0] is not None}
        yield name, pos, opts


# --- writing -------------------------------------------------------------------

class Stream:
    """The command stream of this process (one per script run)."""

    def __init__(self):
        self._out = None
        self._next_handle = 0
        self._failure = None
        self._closed = False
        self.lines = 0

    def _open(self):
        path = os.environ.get('POLYAMOUR_OPS')
        if path:
            self._out = open(path, 'w', encoding='utf-8', newline='\n')
        else:
            # Commands own standard output; the script's prints go to stderr.
            self._out = sys.stdout
            sys.stdout = sys.stderr
        self._out.write('polyops %d\n' % VERSION)
        atexit.register(self.close)
        previous = sys.excepthook

        def hook(kind, value, tb):
            self._failure = '%s: %s' % (kind.__name__, value)
            previous(kind, value, tb)
        sys.excepthook = hook

    def handle(self):
        """A fresh handle token ($1, $2, ...) to bind with as=."""
        self._next_handle += 1
        return Raw('$%d' % self._next_handle)

    def emit(self, command, *args, **opts):
        """Write one command. Positional args and option values are formatted
        with token(); options whose value is None are left out."""
        if self._closed:
            raise RuntimeError('the polyops stream is closed')
        if self._out is None:
            self._open()
        parts = [command]
        parts += [token(a) for a in args]
        parts += ['%s=%s' % (k.rstrip('_'), token(v)) for k, v in opts.items() if v is not None]
        line = ' '.join(parts)
        if '\n' in line or '\r' in line:
            raise ValueError('a polyops line cannot contain a raw newline')
        self._out.write(line + '\n')
        self.lines += 1

    def flush(self):
        if self._out is not None:
            self._out.flush()

    def close(self):
        """End the stream: `end`, or `abort` when the script raised."""
        if self._closed:
            return
        if self._out is None:
            self._open()
        if self._failure is not None:
            self._out.write('abort %s\n' % quote(self._failure))
        else:
            self._out.write('end\n')
        self._closed = True
        self._out.flush()
        if self._out is not sys.__stdout__ and self._out is not sys.stdout and self._out is not sys.stderr:
            self._out.close()


stream = Stream()
