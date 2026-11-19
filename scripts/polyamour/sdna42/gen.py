"""Generate crates/sex-3d/src/io/blend/sdna42.rs: the subset of Blender 4.2's
SDNA (from sdna42.json, made by mk.py) that the .blend writer emits, and the
DNA struct defaults (from DNA_*_defaults.h) for the structs it fills.
Usage: python3 -I gen.py OUT.rs"""
import json, os, re, sys, math

HERE = os.path.dirname(os.path.abspath(__file__))
d = json.load(open(os.path.join(HERE, 'sdna42.json')))
T, N, S = d['types'], d['names'], d['sizes']
by = {T[s[0]]: s for s in d['structs']}

ROOTS = """FileGlobal Scene ViewLayer LayerCollection Base ToolSettings Object Mesh CustomDataLayer vec3f vec2i vec2f
MIntProperty MBoolProperty MFloatProperty MDeformVert MDeformWeight bDeformGroup Material bNodeTree bNode bNodeSocket
bNodeLink bNodeSocketValueFloat bNodeSocketValueInt bNodeSocketValueBoolean bNodeSocketValueVector
bNodeSocketValueRotation bNodeSocketValueRGBA bNodeSocketValueString bNodeSocketValueMenu bNodeTreeInterfaceSocket bNodeTreeInterfacePanel
World Camera Lamp Collection CollectionObject CollectionChild bArmature Bone BoneCollection BoneCollectionMember
bPose bPoseChannel bAction FCurve BezTriple AnimData Image ImagePackedFile PackedFile ImageTile ColorBand CurveMapping
CurveMap CurveMapPoint Link ListBase ID
NodeShaderMix NodeTexImage NodeTexEnvironment NodeTexNoise NodeTexVoronoi NodeTexWave NodeTexMagic NodeTexBrick
NodeTexGradient NodeTexSky NodeTexChecker NodeMapRange NodeCombSepColor NodeShaderNormalMap NodeShaderTangent
NodeShaderAttribute NodeShaderUVMap NodeShaderVectTransform NodeShaderVertexColor NodeShaderTexIES NodeShaderOutputAOV
NodeShaderTexPointDensity NodeShaderHairPrincipled NodeShaderScript ImageUser
NodeBlurData NodeGlare NodeChroma NodeTwoXYs NodeTwoFloats NodeColorBalance NodeColorCorrection NodeTonemap
NodeBoxMask NodeEllipseMask NodeCMPCombSepColor NodeDBlurData NodeBilateralBlurData NodeKuwaharaData NodeKeyingData
NodeColorspill NodeDefocus NodeLensDist NodeTranslateData NodeSunBeams NodeDenoise NodeAntiAliasingData NodeSetAlpha
NodeConvertColorSpace NodeBokehImage NodeDilateErode TexMapping NodeImageMultiFile NodeImageMultiFileSocket NodeMask
NodeKeyingScreenData NodeCryptomatte CryptomatteEntry NodePlaneTrackDeformData NodeTrackPosData NodeImageLayer
NodeHueSat NodeScriptDict NodeFrame
ModifierData MirrorModifierData SubsurfModifierData ArrayModifierData SolidifyModifierData BevelModifierData
TriangulateModifierData WeldModifierData DisplaceModifierData SmoothModifierData DecimateModifierData RemeshModifierData
ScrewModifierData WireframeModifierData EdgeSplitModifierData BuildModifierData BooleanModifierData SimpleDeformModifierData
CastModifierData WaveModifierData ShrinkwrapModifierData LaplacianSmoothModifierData CorrectiveSmoothModifierData
WeightedNormalModifierData ArmatureModifierData HookModifierData LatticeModifierData WarpModifierData
WeightVGEditModifierData WeightVGMixModifierData WeightVGProximityModifierData NodesModifierData IDProperty IDPropertyData
ClothModifierData ClothSimSettings ClothCollSettings SoftbodyModifierData CollisionModifierData ParticleSystemModifierData
ParticleSystem ParticleSettings FluidModifierData FluidDomainSettings FluidFlowSettings FluidEffectorSettings PointCache
EffectorWeights SoftBody SoftBody_Shared PartDeflect RigidBodyOb RigidBodyCon RigidBodyWorld RigidBodyWorld_Shared
bConstraint bConstraintTarget bLocateLikeConstraint bRotateLikeConstraint bSizeLikeConstraint bTransLikeConstraint
bDistLimitConstraint bLocLimitConstraint bRotLimitConstraint bSizeLimitConstraint bSameVolumeConstraint
bTransformConstraint bChildOfConstraint bTrackToConstraint bDampTrackConstraint bLockTrackConstraint
bStretchToConstraint bMinMaxConstraint bArmatureConstraint bKinematicConstraint
Key KeyBlock Lattice BPoint
bGPdata bGPDlayer bGPDframe bGPDstroke bGPDspoint bGPDlayer_Mask MaterialGPencilStyle
NodeGeometryAttributeCapture NodeGeometryAttributeCaptureItem NodeGeometryCollectionInfo NodeGeometryCurveFill
NodeGeometryCurveFillet NodeGeometryCurvePrimitiveArc NodeGeometryCurvePrimitiveBezierSegment
NodeGeometryCurvePrimitiveCircle NodeGeometryCurvePrimitiveLine NodeGeometryCurvePrimitiveQuad
NodeGeometryCurveResample NodeGeometryCurveSample NodeGeometryCurveSetHandles NodeGeometryCurveSplineType
NodeGeometryCurveToPoints NodeGeometryCurveTrim NodeGeometryDeleteGeometry NodeGeometryDuplicateElements
NodeGeometryExtrudeMesh NodeGeometryInputNamedAttribute NodeGeometryMergeByDistance NodeGeometryMeshCircle
NodeGeometryMeshCone NodeGeometryMeshCylinder NodeGeometryMeshLine NodeGeometryMeshToPoints NodeGeometryObjectInfo
NodeGeometryProximity NodeGeometryRaycast NodeGeometryRepeatInput NodeGeometryRepeatOutput NodeRepeatItem
NodeGeometrySampleIndex NodeGeometrySeparateGeometry NodeGeometrySetCurveHandlePositions NodeGeometrySimulationInput
NodeGeometrySimulationOutput NodeSimulationItem NodeGeometryStoreNamedAttribute NodeGeometrySubdivisionSurface
NodeGeometryViewer NodeAccumulateField NodeSwitch NodeIndexSwitch IndexSwitchItem NodeMenuSwitch NodeEnumDefinition
NodeEnumItem NodeRandomValue NodeFunctionCompare NodeInputBool NodeInputInt NodeInputVector NodeInputColor
NodeInputString NodeInputRotation""".split()

PRIMS = {'char', 'uchar', 'short', 'ushort', 'int', 'long', 'ulong', 'float', 'double', 'int64_t', 'uint64_t', 'void', 'int8_t', 'bool'}

need = []
seen = set()
def visit(t):
    if t in seen:
        return
    if t not in by:
        raise SystemExit(f'no struct {t}')
    seen.add(t)
    for ti, ni in by[t][1]:
        tn, nm = T[ti], N[ni]
        if not nm.startswith('*') and not nm.startswith('(') and tn in by:
            visit(tn)
    need.append(t)
for r in ROOTS:
    visit(r)

# Types referenced by the needed structs (members, including pointer targets).
types = []
for t in need:
    if t not in types:
        types.append(t)
    for ti, ni in by[t][1]:
        if T[ti] not in types:
            types.append(T[ti])
size_of = {T[i]: S[i] for i in range(len(T))}

# ---------------------------------------------------------------- defaults
STRUCT_RENAME = {}
ELEM_RENAME = {}
src = open(os.path.join(HERE, 'rename.h')).read()
for old, new in re.findall(r'^DNA_STRUCT_RENAME\((\w+), (\w+)\)', src, re.M):
    STRUCT_RENAME[new] = old
for s, old, new in re.findall(r'^DNA_STRUCT_RENAME_ELEM\((\w+), (\w+), (\w+)\)', src, re.M):
    ELEM_RENAME[(STRUCT_RENAME.get(s, s), new)] = old

consts = {'M_PI': math.pi, 'M_SQRT1_3': math.sqrt(1 / 3), 'DEFAULT_SENSOR_WIDTH': 36.0, 'DEFAULT_SENSOR_HEIGHT': 24.0,
          'INT_MAX': 2147483647, 'SHRT_MAX': 32767, 'FLT_MAX': 3.4028234663852886e38, 'NULL': 0, 'nullptr': 0, 'true': 1, 'false': 0,
          'M_PI_2': math.pi / 2, 'M_PI_4': math.pi / 4}
hdrs = ''
for dd in ('dna', 'defs'):
    for f in sorted(os.listdir(os.path.join(HERE, dd))):
        if f.endswith('.h'):
            hdrs += open(os.path.join(HERE, dd, f), errors='replace').read() + '\n'
hdrs_nc = re.sub(r'/\*.*?\*/', '', hdrs, flags=re.S)
hdrs_nc = re.sub(r'//[^\n]*', '', hdrs_nc)
raw_consts = {}
for body in re.findall(r'enum\s*\w*\s*(?::\s*\w+\s*)?\{(.*?)\}', hdrs_nc, re.S):
    prev = -1
    for item in body.split(','):
        item = item.strip()
        if not item:
            continue
        m = re.match(r'^(\w+)\s*(?:=\s*(.+))?$', item, re.S)
        if not m:
            continue
        name, expr = m.group(1), m.group(2)
        raw_consts[name] = expr if expr is not None else f'__prev__+1:{prev}'
        prev = name
for name, expr in re.findall(r'^\s*#\s*define\s+([A-Z_][A-Z0-9_]*)\s+([^\n\\]+)$', hdrs_nc, re.M):
    if name.startswith('_DNA_DEFAULT'):
        continue
    raw_consts.setdefault(name, expr.strip())

def ev(expr, depth=0):
    if depth > 20:
        raise ValueError(expr)
    e = ' '.join(expr.split())
    if e.startswith('__prev__+1:'):
        p = e.split(':', 1)[1]
        return 0 if p == '-1' else ev_name(p, depth + 1) + 1
    e = re.sub(r'(\d+\.\d*|\.\d+|\d+)(?:e([-+]?\d+))?[fF]\b', lambda m: m.group(0)[:-1], e)
    e = re.sub(r'\(\s*(float|int|short|char|uint|unsigned int)\s*\)', '', e)
    e = re.sub(r'\b(\d+)[uU][lL]*\b', r'\1', e)
    e = e.replace('DEG2RADF', 'DEG2RAD').replace('&&', ' and ').replace('||', ' or ')
    def repl(m):
        w = m.group(0)
        if w == 'DEG2RAD':
            return '__deg'
        if re.match(r'^\d', w):
            return w
        return repr(ev_name(w, depth + 1))
    e = re.sub(r'\b[A-Za-z_]\w*\b', repl, e)
    e = e.replace('__deg', '(lambda x: x*%r)' % (math.pi / 180))
    v = eval(e, {}, {})
    return v

def ev_name(w, depth):
    if w in consts:
        return consts[w]
    if w in raw_consts:
        v = ev(raw_consts[w], depth)
        consts[w] = v
        return v
    raise ValueError(w)

macros = {}
for m in re.finditer(r'#define\s+(_DNA_DEFAULTS?_\w+)\s*\\?\s*\n?(.*?)(?=\n#|\n\s*\n/\*|\Z)', hdrs, re.S):
    body = m.group(2)
    body = re.sub(r'\\\n', '\n', body)
    body = re.sub(r'("(?:[^"\\]|\\.)*")|/\*.*?\*/|//[^\n]*', lambda k: k.group(1) or '', body, flags=re.S)
    macros[m.group(1)] = body.strip()

def split_top(s):
    out, depth, cur = [], 0, ''
    for ch in s:
        if ch in '{(':
            depth += 1
        elif ch in '})':
            depth -= 1
        if ch == ',' and depth == 0:
            out.append(cur)
            cur = ''
        else:
            cur += ch
    if cur.strip():
        out.append(cur)
    return [x.strip() for x in out if x.strip()]

def expand(v):
    v = v.strip()
    while v in macros:
        v = macros[v]
    return v

def flat_nums(v):
    v = expand(v)
    if v.startswith('{'):
        out = []
        for x in split_top(v[1:-1]):
            out += flat_nums(x)
        return out
    return [ev(v)]

defaults = []
skipped = []
def walk(struct, prefix, body):
    body = expand(body)
    assert body.startswith('{'), body[:60]
    for item in split_top(body.strip()[1:-1]):
        m = re.match(r'^\.([\w.]+)\s*=\s*(.*)$', item, re.S)
        if not m:
            skipped.append((struct, prefix, item[:40]))
            continue
        path, val = m.group(1), expand(m.group(2))
        parts = path.split('.')
        parts[0] = ELEM_RENAME.get((struct, parts[0]), parts[0])
        full = prefix + '.'.join(parts)
        if val.startswith('{') and re.match(r'^\{\s*\.', val):
            sub = member_struct(struct, parts[0]) if len(parts) == 1 else None
            walk(sub or struct, full + '.', val)
            continue
        if val.startswith('"'):
            defaults.append((struct, full, 'S', val[1:-1]))
            continue
        try:
            nums = flat_nums(val)
        except Exception as ex:
            skipped.append((struct, full, str(ex)[:40]))
            continue
        if any(x != 0 for x in nums):
            defaults.append((struct, full, 'N', nums))

def member_struct(struct, member):
    s = by.get(struct)
    if not s:
        return None
    for ti, ni in s[1]:
        nm = re.match(r'[\*\(]*(\w+)', N[ni]).group(1)
        if nm == member and not N[ni].startswith('*'):
            return T[ti]
    return None

# Nested defaults: walk resolves sub-struct names by member type so element
# renames inside nested structs apply too.
def walk_root(name, macro):
    walk_struct(name, '', macros[macro])

def walk_struct(struct, prefix, body, root=None):
    root = root or struct
    body = expand(body)
    for item in split_top(body.strip()[1:-1]):
        m = re.match(r'^\.([\w.]+)\s*=\s*(.*)$', item, re.S)
        if not m:
            skipped.append((struct, prefix, item[:40]))
            continue
        parts = m.group(1).split('.')
        val = expand(m.group(2))
        cur, path = struct, []
        for p in parts[:-1]:
            p = ELEM_RENAME.get((cur, p), p)
            path.append(p)
            cur = member_struct(cur, p) or cur
        last = ELEM_RENAME.get((cur, parts[-1]), parts[-1])
        path.append(last)
        full = prefix + '.'.join(path)
        sub = member_struct(cur, last)
        if val.startswith('{') and sub and re.match(r'^\{\s*\.', val):
            walk_struct(sub, full + '.', val, root)
        elif val.startswith('{') and sub and val.startswith('{_DNA'):
            walk_struct(sub, full + '.', val[1:-1], root)
        elif val.startswith('"'):
            defaults.append((root, full, 'S', val[1:-1]))
        else:
            try:
                nums = flat_nums(val)
            except Exception as ex:
                skipped.append((root, full, str(ex)[:40]))
                continue
            if any(x != 0 for x in nums):
                defaults.append((root, full, 'N', nums))

ROOT_DEFAULTS = [('Scene', '_DNA_DEFAULT_Scene'), ('ToolSettings', '_DNA_DEFAULT_ToolSettings'), ('Object', '_DNA_DEFAULT_Object'),
                 ('Camera', '_DNA_DEFAULT_Camera'), ('Lamp', '_DNA_DEFAULT_Light'), ('World', '_DNA_DEFAULT_World'),
                 ('Material', '_DNA_DEFAULT_Material'), ('Mesh', '_DNA_DEFAULT_Mesh'), ('Collection', '_DNA_DEFAULT_Collection'),
                 ('Image', '_DNA_DEFAULT_Image'), ('bArmature', '_DNA_DEFAULT_bArmature'), ('bAction', '_DNA_DEFAULT_bAction')]
# Every other written struct with a `_DNA_DEFAULT_<name>` macro (modifiers,
# Lattice, ParticleSettings, the cloth and fluid settings, ...).
covered = {st for st, _ in ROOT_DEFAULTS} | {'Light'}
for t in need:
    if t not in covered and f'_DNA_DEFAULT_{t}' in macros:
        ROOT_DEFAULTS.append((t, f'_DNA_DEFAULT_{t}'))
for st, mac in ROOT_DEFAULTS:
    if mac in macros:
        walk_struct(st, '', macros[mac])
    else:
        print('no macro', mac, file=sys.stderr)

for s in skipped:
    print('skip', s, file=sys.stderr)

def num(x):
    if isinstance(x, bool):
        x = int(x)
    if isinstance(x, int):
        return f'{x}.0'
    r = repr(float(x))
    return r if ('.' in r or 'e' in r) else r + '.0'

out = ['// SPDX-License-Identifier: GPL-3.0-only',
       '//! Generated from Blender 4.2\'s DNA headers (GPL-2+) by a port of',
       '//! `makesdna`: the structs the `.blend` writer emits (closed over by-value',
       '//! members) with their static (file) member names, every type they refer',
       '//! to with its 64-bit size, and the DNA struct defaults of',
       '//! `DNA_*_defaults.h` for the ID structs the writer fills. Do not edit.',
       '',
       '/// `(type, size)` of every type the structs below refer to.',
       'pub(super) const TYPES: &[(&str, u16)] = &[']
for t in types:
    out.append(f'    ("{t}", {size_of[t]}),')
out.append('];')
out.append('')
out.append('/// Structs in dependency order: `(name, &[(type, member declarator)])`.')
out.append('pub(super) const STRUCTS: &[(&str, &[(&str, &str)])] = &[')
for t in need:
    mem = ', '.join(f'("{T[ti]}", "{N[ni]}")' for ti, ni in by[t][1])
    out.append(f'    ("{t}", &[{mem}]),')
out.append('];')
out.append('')
out.append('/// A default member value.')
out.append('pub(super) enum Dv {')
out.append('    N(&\'static [f64]),')
out.append('    S(&\'static str),')
out.append('}')
out.append('')
out.append('/// `(struct, member path, value)` DNA defaults.')
out.append('pub(super) const DEFAULTS: &[(&str, &str, Dv)] = &[')
for st, path, kind, v in defaults:
    if kind == 'S':
        out.append(f'    ("{st}", "{path}", Dv::S("{v}")),')
    else:
        out.append(f'    ("{st}", "{path}", Dv::N(&[{", ".join(num(x) for x in v)}])),')
out.append('];')
open(sys.argv[1], 'w').write('\n'.join(out) + '\n')
print(len(need), 'structs', len(types), 'types', len(defaults), 'defaults', len(skipped), 'skipped', file=sys.stderr)
