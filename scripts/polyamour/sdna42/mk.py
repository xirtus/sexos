"""A Python port of Blender 4.2's makesdna (source/blender/makesdna/intern/makesdna.cc):
parse the DNA headers the way makesdna does and compute the SDNA (names,
types, type sizes for 64-bit, structs) including the static (file) names from
dna_rename_defs.h. Writes sdna42.json."""
import json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))

def load_renames():
    src = open(os.path.join(HERE, 'rename.h')).read()
    struct_static_from_alias = {}
    for old, new in re.findall(r'^DNA_STRUCT_RENAME\((\w+), (\w+)\)', src, re.M):
        struct_static_from_alias[new] = old
    alias_to_static_struct = dict(struct_static_from_alias)
    for a, b in [("uint8_t", "uchar"), ("int16_t", "short"), ("uint16_t", "ushort"), ("int32_t", "int"), ("uint32_t", "int")]:
        struct_static_from_alias[a] = b
    elem = {}
    for s, old, new in re.findall(r'^DNA_STRUCT_RENAME_ELEM\((\w+), (\w+), (\w+)\)', src, re.M):
        st = alias_to_static_struct.get(s, s)
        elem[(st, new)] = old
    return struct_static_from_alias, elem

STRUCT_RENAME, ELEM_RENAME = load_renames()

types, sizes, aligns = [], [], []
names = []
structs = []  # (type index, [(type, name)])

def add_type(s, size=0):
    if not s or '*' in s:
        raise SystemExit(f'bad type {s!r}')
    s = STRUCT_RENAME.get(s, s)
    if s in types:
        i = types.index(s)
        if size:
            sizes[i] = size
            aligns[i] = size
        return i
    types.append(s)
    sizes.append(size)
    aligns.append(size)
    return len(types) - 1

def strip_id(n):
    # DNA_elem_id_strip: the identifier without pointer/array decorations.
    m = re.match(r'[\*\(]*(\w+)', n)
    return m.group(1)

def elem_static(strct, full):
    ident = strip_id(full)
    st = ELEM_RENAME.get((types[strct], ident))
    if st is None:
        return full
    return full.replace(ident, st, 1)

def add_name(s):
    if s.startswith('(*'):
        isfunc = '(' in s[1:]
        i = s.index(')')
        buf = s[:i]
        if isfunc:
            if buf.startswith('(*headdraw') or buf.startswith('(*windraw'):
                name = buf + ')(void)'
            else:
                name = buf + ')()'
        else:
            name = buf + ')()'
            # multidimensional array pointer, e.g. float (*verts)[3]
        s = name
    if s in names:
        return names.index(s)
    names.append(s)
    return len(names) - 1

def preprocess(data):
    t = bytearray(data + b' ')
    n = len(data)
    comment = False
    for i in range(n):
        if t[i] == ord('/') and t[i + 1] == ord('/'):
            comment = True
        elif t[i] == ord('\n'):
            comment = False
        if comment or t[i] < 32 or t[i] > 128:
            t[i] = 32
    out = bytearray()
    comment = False
    i = 0
    skip_brace = False
    def ident_at(i, word):
        w = word.encode()
        if t[i:i + len(w)] != w:
            return False
        before = t[i - 1] if i > 0 else 32
        after = t[i + len(w)] if i + len(w) < len(t) else 32
        isid = lambda c: chr(c).isalnum() or c == ord('_')
        return not isid(before) and not isid(after)
    while i < n:
        if t[i] == ord('/') and t[i + 1] == ord('*'):
            comment = True
            t[i] = t[i + 1] = 32
        if t[i] == ord('*') and t[i + 1] == ord('/'):
            comment = False
            t[i] = t[i + 1] = 32
        c = t[i]
        if comment:
            pass
        elif c == 32 and t[i + 1] == 32:
            pass
        elif i > 0 and t[i - 1] == ord('*') and c == 32:
            pass
        elif ident_at(i, 'DNA_DEPRECATED'):
            i += 13
        elif ident_at(i, 'DNA_DEFINE_CXX_METHODS'):
            i += 21
            skip_brace = True
        elif skip_brace:
            if c == ord(')'):
                skip_brace = False
        elif c == ord('#') and re.match(rb'# *ifdef __cplusplus', bytes(t[i:i + 40])):
            m = re.compile(rb'# *endif').search(bytes(t), i + 1)
            if m is None:
                raise SystemExit('unterminated cplusplus block')
            i = m.end() - 1
        else:
            out.append(c)
        i += 1
    return bytes(out)

def convert(path):
    md = bytearray(preprocess(open(path, 'rb').read()))
    n = len(md)
    skip = False
    for p in range(n):
        if md[p] == ord('#') and p + 2 < n and md[p + 1] == 32 and md[p + 2] == ord('#'):
            skip = True
        if md[p] != ord('{'):
            continue
        md[p] = 0
        if skip:
            skip = False
            continue
        if md[p - 1] == 32:
            md[p - 1] = 0
        q = p - 2
        while md[q] != 32:
            q -= 1
        q += 1
        word = bytes(md[q:md.index(0, q)]).decode()
        if bytes(md[q - 7:q - 1]) != b'struct' or (q - 8 >= 0 and (chr(md[q - 8]).isalnum() or md[q - 8] == ord('_'))):
            continue
        strct = add_type(word)
        members = []
        # Body up to the matching '}' (DNA structs have no nested braces).
        end = md.index(ord('}'), p + 1)
        body = bytes(md[p + 1:end]).decode()
        for decl in body.split(';'):
            toks = [x for x in re.split(r'[ ,]', decl) if x]
            # makesdna splits on ' ' and ',' only; a declaration is "type name, name"
            while toks and toks[0] in ('struct', 'unsigned', 'const'):
                toks.pop(0)
            if not toks:
                continue
            ty = toks[0]
            if ty in ('long', 'ulong'):
                raise SystemExit(f'long in {path}')
            ti = add_type(ty)
            # Function pointers: "(*func)(args)" may have been split on spaces/commas.
            rest = ' '.join(toks[1:])
            parts = []
            depth = 0
            cur = ''
            for ch in decl[decl.index(ty) + len(ty):]:
                if ch == '(':
                    depth += 1
                elif ch == ')':
                    depth -= 1
                if ch == ',' and depth == 0:
                    parts.append(cur)
                    cur = ''
                else:
                    cur += ch
            parts.append(cur)
            for nm in parts:
                nm = nm.replace(' ', '')
                if not nm:
                    continue
                members.append((ti, add_name(elem_static(strct, nm))))
        structs.append((strct, members))

for t, s in [("char", 1), ("uchar", 1), ("short", 2), ("ushort", 2), ("int", 4), ("long", 4), ("ulong", 4), ("float", 4), ("double", 8), ("int64_t", 8), ("uint64_t", 8), ("void", 0), ("int8_t", 1)]:
    add_type(t, s)
FIRST = len(types)

for h in open(os.path.join(HERE, 'dna_list.txt')).read().split():
    convert(os.path.join(HERE, 'dna', h))

def array_len(n):
    m = 1
    for d in re.findall(r'\[(\d+)\]', n):
        m *= int(d)
    return m

# Sizes for 64-bit, iterating like calculate_struct_sizes.
unknown = len(structs)
while unknown:
    last = unknown
    unknown = 0
    for st, ms in structs:
        if sizes[st]:
            continue
        size = 0
        align = 0
        ok = True
        for ti, ni in ms:
            nm = names[ni]
            if nm.startswith("*") or nm[1:2] == "*":
                mul = array_len(nm) if nm.endswith(']') else 1
                assert size % 8 == 0, (types[st], nm, size)
                size += 8 * mul
                align = max(align, 8)
            elif sizes[ti]:
                mul = array_len(nm) if nm.endswith(']') else 1
                if ti >= FIRST:
                    assert size % 8 == 0, ('struct align', types[st], nm, size)
                if sizes[ti] > 4 and ti < FIRST:
                    assert size % 8 == 0, ('align8', types[st], nm)
                if sizes[ti] > 3:
                    assert size % 4 == 0, ('align4', types[st], nm)
                if sizes[ti] == 2:
                    assert size % 2 == 0, ('align2', types[st], nm)
                size += mul * sizes[ti]
                align = max(align, aligns[ti])
            else:
                ok = False
                break
        if not ok:
            unknown += 1
            continue
        sizes[st] = size
        aligns[st] = align
        assert size % align == 0, ('size', types[st], size, align)
    if unknown == last:
        break
if unknown:
    print('unknown structs:', [types[s] for s, _ in structs if not sizes[s]], file=sys.stderr)
    sys.exit(1)

json.dump({'names': names, 'types': types, 'sizes': sizes, 'structs': structs}, open(os.path.join(HERE, 'sdna42.json'), 'w'))
print(len(names), 'names', len(types), 'types', len(structs), 'structs')
