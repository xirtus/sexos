# SPDX-License-Identifier: GPL-3.0-only
"""polyops: how the Python API talks to Polyamour.

Every read and write of `bpy` is a polyops command (the language of
crates/sex-3d/src/script.rs, described in
docs/handoff/POLYAMOUR_PYTHON_2026_11_14.md). Two channels carry them:

* LiveChannel: Polyamour started this Python as its kernel
  (polyamour/kernel.py). Each command is a request `@N command ...` on the
  kernel's protocol stream and waits for `@N ok ...` / `@N err MESSAGE`, so a
  script reads back what it just did. Reads (`dump_object`, ...) answer JSON
  and are cached until the next write.
* BatchChannel: a plain `python script.py` (or a system with no pipes between
  programs): commands are written to POLYAMOUR_OPS or standard output and
  applied by Polyamour when the script ends. Reads are answered from a
  snapshot of the scene taken before the run (run_batch), or refused.

The module also knows the lexical rules (quoting, numbers, lists, base64
arrays) and reads polyops text.
"""
import json
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
    """`s` as one polyops token: bare when safe, else double-quoted. A Raw
    text (a batch handle like `$3`) stays as it is."""
    if isinstance(s, Raw):
        return str(s)
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



# --- the channel ----------------------------------------------------------------

# Commands that only read (script/bpy.rs QUERIES plus the core queries): their
# replies are cached until the next write.
QUERIES = frozenset((
    'ids', 'dump_object', 'dump_mesh', 'dump_material', 'dump_tree', 'dump_scene', 'dump_armature',
    'dump_lattice', 'dump_image', 'dump_action', 'eval_mesh', 'op_poll', 'op_list', 'noise',
    'get', 'mesh_get', 'list', 'exists', 'ping',
))


class PolyopsError(RuntimeError):
    """Polyamour refused a request (the message says why)."""


def _line(command, args, opts):
    parts = [command]
    parts += [token(a) for a in args]
    parts += ['%s=%s' % (k.rstrip('_'), token(v)) for k, v in opts.items() if v is not None]
    line = ' '.join(parts)
    if '\n' in line or '\r' in line:
        raise ValueError('a polyops line cannot contain a raw newline')
    return line


def _parse_reply(rest):
    """`ok` reply text -> JSON value (`=json ...`), option dict (`k=v ...`), or list of token texts."""
    if rest.startswith('=json '):
        return json.loads(rest[6:])
    toks = tokenize(rest)
    if toks and all(t[0] is not None for t in toks):
        return {t[0]: t[1] for t in toks}
    return [t[1] for t in toks]


class LiveChannel:
    """Request/reply with a running Polyamour over a pair of line streams."""

    live = True

    def __init__(self, rfile, wfile):
        self._r = rfile
        self._w = wfile
        self._n = 0
        self._cache = {}
        self.generation = 0
        # Objects with local changes to send before the next request
        # (`_flush()` on each), e.g. meshes whose vertices a script moved.
        self.pending = {}
        self._flushing = False
        self.requests = 0

    # -- low level
    def send_line(self, line):
        self._w.write(line + '\n')
        self._w.flush()

    def read_line(self):
        line = self._r.readline()
        if not line:
            raise SystemExit('Polyamour closed the channel')
        return line.rstrip('\r\n')

    def flush(self):
        """Send every pending local change."""
        if self._flushing or not self.pending:
            return
        self._flushing = True
        try:
            while self.pending:
                key = next(iter(self.pending))
                obj = self.pending.pop(key)
                obj._flush()
        finally:
            self._flushing = False

    def invalidate(self):
        self._cache.clear()
        self.generation += 1

    def request(self, command, *args, **opts):
        """Send one polyops command and wait for its reply: a JSON value or a
        list of token texts. Raises PolyopsError when Polyamour refuses it."""
        line = _line(command, args, opts)
        query = command in QUERIES
        if query and line in self._cache:
            return self._cache[line]
        self.flush()
        self._n += 1
        rid = self._n
        self.requests += 1
        self.send_line('@%d %s' % (rid, line))
        while True:
            reply = self.read_line()
            head, _, rest = reply.partition(' ')
            if head == '@%d' % rid:
                break
            # Anything else here is a protocol error on the host side; skip it.
        status, _, rest = rest.partition(' ')
        if not query:
            self.invalidate()
        if status == 'err':
            toks = tokenize(rest)
            raise PolyopsError(toks[0][1] if toks else rest)
        value = _parse_reply(rest)
        if query:
            self._cache[line] = value
        return value

    def query(self, command, *args, **opts):
        return self.request(command, *args, **opts)

    def noise(self, kind, points, **opts):
        flat = [float(c) for p in points for c in p]
        return self.request('noise', kind, b64_floats(flat), **opts)


# Commands that make something and answer its name; on the batch channel
# they get `as=$N` and the handle stands in for the name.
CREATING = frozenset((
    'add_primitive', 'add_mesh', 'add_object', 'add_empty', 'light_add', 'camera_add', 'add_armature',
    'add_lattice', 'mesh_new', 'material_new', 'collection_new', 'object_copy', 'mesh_copy',
    'material_copy', 'armature_new', 'lattice_new',
))


class BatchChannel:
    """One-way: commands are written to a stream that Polyamour applies after
    the script ends (for systems without pipes between programs). Nothing can
    be read back, so reading scene data raises a clear error."""

    live = False

    def __init__(self, out=None, path=None, snapshot=None):
        self._out = out
        self._path = path
        # Replies to reads, as Polyamour gave them before the run started:
        # {(command, arg, ...): reply} (see run_batch).
        self._snapshot = snapshot or {}
        self.generation = 0
        self.pending = {}
        self._failure = None
        self._closed = False
        self._next_handle = 0
        self.requests = 0

    def _open(self):
        path = self._path or os.environ.get('POLYAMOUR_OPS')
        if path:
            self._out = open(path, 'w', encoding='utf-8', newline='\n')
        else:
            self._out = sys.stdout
            sys.stdout = sys.stderr
        self._out.write('polyops %d\n' % VERSION)
        atexit.register(self.close)
        previous = sys.excepthook

        def hook(kind, value, tb):
            self._failure = '%s: %s' % (kind.__name__, value)
            previous(kind, value, tb)
        sys.excepthook = hook

    def comment(self, tag, text):
        """A comment line in the stream (printed text in a batch run)."""
        if self._closed:
            return
        if self._out is None:
            self._open()
        self._out.write('%s %s\n' % (tag, quote(text) if text else '""'))

    def handle(self):
        self._next_handle += 1
        return Raw('$%d' % self._next_handle)

    def flush(self):
        while self.pending:
            key = next(iter(self.pending))
            self.pending.pop(key)._flush()

    def invalidate(self):
        self.generation += 1

    def request(self, command, *args, **opts):
        if command in QUERIES:
            key = tuple(t[1] for t in tokenize(_line(command, args, opts)))
            if key in self._snapshot:
                return self._snapshot[key]
            raise PolyopsError(
                "reading '%s' needs a live Polyamour; this Python was started without one "
                "(scripts that only create and change things still work, and read the "
                "scene as it was when they started)" % command)
        if self._closed:
            raise RuntimeError('the polyops stream is closed')
        self.flush()
        if self._out is None:
            self._open()
        made = None
        if command in CREATING and 'as_' not in opts and 'as' not in opts:
            made = self.handle()
            opts = dict(opts, as_=made)
        self._out.write(_line(command, args, opts) + '\n')
        self.requests += 1
        self.generation += 1
        if made is not None:
            return [made]
        if command == 'op':
            return ['FINISHED']
        return []

    query = request

    def noise(self, kind, points, **opts):
        return None

    def close(self):
        if self._closed:
            return
        self.flush()
        if self._out is None:
            self._open()
        self._out.write('abort %s\n' % quote(self._failure) if self._failure else 'end\n')
        self._closed = True
        self._out.flush()
        if self._out not in (sys.__stdout__, sys.stdout, sys.stderr):
            self._out.close()


# The process's channel: kernel.py installs a LiveChannel; a plain `python
# script.py` gets the batch stream on first use.
channel = None


def run_batch(ops, snapshot, script, args=()):
    """Run `script` on the batch channel: commands to the file `ops`, reads
    from the JSON `snapshot` ([[request tokens], reply] pairs written by
    Polyamour before the run). Everything comes as arguments, since a SexOS
    program has no environment. An uncaught exception ends the stream with
    `abort "line N: Kind: message"`, N being the script's line."""
    global channel
    import runpy
    import traceback
    snap = {}
    if snapshot:
        try:
            with open(snapshot, encoding='utf-8') as f:
                for key, value in json.load(f):
                    snap[tuple(key)] = value
        except (OSError, ValueError):
            pass
    channel = BatchChannel(path=ops, snapshot=snap)
    # What the script prints rides in the command file as comment lines, so
    # it reaches Polyamour where programs have no pipes (SexOS).
    out, err = sys.stdout, sys.stderr
    sys.stdout = _BatchPrinter(channel, '#>')
    sys.stderr = _BatchPrinter(channel, '#!')
    sys.argv = [script] + list(args)
    folder = os.path.dirname(os.path.abspath(script))
    if folder not in sys.path:
        sys.path.insert(0, folder)
    try:
        runpy.run_path(script, run_name='__main__')
    except SystemExit as e:
        if e.code not in (None, 0):
            channel._failure = 'SystemExit: %s' % (e.code,)
    except BaseException as e:  # noqa: B902 - reported, then the stream aborts
        line = 0
        for fr in traceback.extract_tb(e.__traceback__):
            if fr.filename == script:
                line = fr.lineno
        traceback.print_exc()
        channel._failure = ('line %d: ' % line if line else '') + '%s: %s' % (type(e).__name__, e)
    sys.stdout.flush()
    sys.stderr.flush()
    sys.stdout, sys.stderr = out, err
    channel.close()


class _BatchPrinter:
    """A text stream writing whole lines as `#> "text"` comments."""

    def __init__(self, channel, tag):
        self._c = channel
        self._tag = tag
        self._buf = ''

    def write(self, s):
        self._buf += str(s)
        while '\n' in self._buf:
            line, self._buf = self._buf.split('\n', 1)
            self._c.comment(self._tag, line)
        return len(s)

    def flush(self):
        if self._buf:
            self._c.comment(self._tag, self._buf)
            self._buf = ''

    def isatty(self):
        return False


def get():
    global channel
    if channel is None:
        channel = BatchChannel()
    return channel


def request(command, *args, **opts):
    return get().request(command, *args, **opts)
