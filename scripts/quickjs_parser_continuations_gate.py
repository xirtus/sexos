#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Compare pinned upstream and parser-only QuickJS tools; no network or website shim."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import random
import shutil
import subprocess
import tarfile

PIN = "3c9afc9943323ee9c7dbd123c0cd991448f4b6c2"
ROOT = Path(__file__).resolve().parents[1]
PATCH = ROOT / "crates/sex-js/guest/quickjs-parser-continuations.patch"
CASES = {
    "precedence": """print(JSON.stringify([
      3+4*5, 30/3/2, 21%8*2, 1+2<<3, 40>>1+2, -1>>>2,
      1<2==true, 4>=3!==false, 3&6^4|8, 1|2^3&4,
      2**3**2, (-(-2))**2, 0||1&&2, null??3, (null??0)||4,
      1?2:3?4:5, 8-3-2, 'a'+1+2, 1+2+'a', String(2n+3n*4n)]));""",
    "coercion": """let log=[];let a={valueOf(){log.push('a');return 4}},
      b={valueOf(){log.push('b');return 3}},c={valueOf(){log.push('c');return 2}};
      print(a+b*c,log.join(''));log=[];print(a<b==false,log.join(''));
      log=[];try{print(a+Symbol('x'))}catch(e){print(e.name,log.join(''))}
      log=[];print(a>b>c,log.join(''));""",
    "brands": """class C {#p=1; check(x){return [#p in x, #p in x==true,
      (#p in x)&3, #p in x ? 7 : 9, #p in x && 5];}}
      let c=new C;print(JSON.stringify(c.check(c)),JSON.stringify(c.check({})));
      try{c.check(null)}catch(e){print(e.name)};""",
    "brand_order": """let log=[];class C{#p;check(x){return [#p in x==true,
      #p in (log.push('r'),x), (#p in x)<2, (#p in x)|2]}}
      let c=new C;print(JSON.stringify(c.check(c)),log.join(''));
      try{eval('class D{#p;f(){return #p in (1+*)}}')}catch(e){print(e.name)}
      print(JSON.stringify(c.check({})));""",
    "no_in": """let o={a:1,b:2},out=[];for(let k in o)out.push(k);
      for(let i=0;i<3;i=i+1)out.push(i);for(let x of [2,3])out.push(x);
      for(let k=('a' in o);k;k=false)out.push(k);print(JSON.stringify(out));""",
    "assignments": """let a,b,c=2; a=b=c+3*4;let o={x:1};o.x+=a<<1;
      print(a,b,o.x,((a,b,c)),false?1:2+3*4);""",
    "operators": """let o={x:1};print('x' in o, [] instanceof Array,
      1===1,1!==2,1!=2,1<=2,2>1,3>>>1,3>>1,3<<1,
      2**-1,~3&15,typeof void 0);""",
    "regex_template": """print(/a+b/.test('aaab'),`x${1+2*3}y`,
      `${'a'+(2<<3)}`,({a:1}).a+([2,3][0]**3));""",
    "runtime_recursion": """function f(){return f()}try{f()}catch(e){
      print(e.name,/stack/i.test(e.message))}""",
}


def generated_expressions():
    """Fixed seed: actual emitted syntax plus coercion/exception order, not eval strings."""
    rng = random.Random(0x534558)
    atoms = ["0", "1", "-1", "3.5", "NaN", "Infinity", "undefined", "null",
             "true", "false", "'a'", "'2'", "2n", "Symbol('s')"]
    operators = ["+", "-", "*", "/", "%", "<<", ">>", ">>>", "<", "<=", ">", ">=",
                 "==", "!=", "===", "!==", "&", "^", "|"]
    serial = 0

    def expr(depth):
        nonlocal serial
        if depth == 0 or rng.randrange(4) == 0:
            serial += 1
            maker = "obj" if rng.randrange(3) == 0 else "value"
            return f"{maker}({serial},{rng.choice(atoms)})"
        return f"({expr(depth - 1)} {rng.choice(operators)} {expr(depth - 1)})"

    prefix = """let out=[],log=[];function value(n,v){log.push('v'+n);return v}
      function obj(n,v){log.push('o'+n);return {valueOf(){log.push('c'+n);return v}}}
      function keep(v){return typeof v+':'+String(v)+(Object.is(v,-0)?':-0':'')}
    """
    lines = [prefix]
    for _ in range(128):
        lines.append(f"log=[];try{{out.push([keep({expr(6)}),log.join(',')])}}catch(e){{out.push([e.name,log.join(',')])}}")
    return "\n".join(lines) + "\nprint(JSON.stringify(out));"
SYNTAX = {
    "missing_rhs": "let a=1+(2*);",
    "mix_nullish": "let a=null??1||2;",
    "unary_power": "let a=-2**2;",
    "outside_brand": "let a=#p in {};",
    "brand_rhs": "class C{#p;f(){return #p in }}",
    "brand_nested_rhs": "class C{#p;f(){return #p in (1+*2)}}",
    "no_in_invalid": "for(let a=1 in {}){}",
}


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def run(args, cwd=None, timeout=120, log=None):
    p = subprocess.run([str(x) for x in args], cwd=cwd, capture_output=True, timeout=timeout)
    if log:
        with open(log, "ab") as f:
            f.write(("$ " + " ".join(map(str, args)) + "\n").encode())
            f.write(p.stdout + p.stderr)
    return p


def build(source, output, cc):
    build_dir = source / "build"
    build_dir.mkdir()
    objects = []
    flags = [cc, "-O2", "-DNDEBUG", "-D_GNU_SOURCE", "-I", source]
    for name in ["cutils", "libregexp", "libunicode", "quickjs", "xsum", "quickjs-libc"]:
        obj = build_dir / (name + ".o")
        p = run(flags + ["-c", source / (name + ".c"), "-o", obj], log=output)
        assert p.returncode == 0, f"compile {name}: {p.stderr.decode(errors='replace')}"
        objects.append(obj)
    for tool, files in [("qjsc", ["qjsc.c"]), ("qjs", ["qjs.c", "gen/repl.c", "gen/standalone.c"])]:
        p = run(flags + [source / n for n in files] + objects + ["-lm", "-lpthread", "-o", build_dir / tool], log=output)
        assert p.returncode == 0, f"link {tool}: {p.stderr.decode(errors='replace')}"
    return build_dir


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--quickjs", type=Path, required=True, help="local pinned upstream git repository")
    ap.add_argument("--out", type=Path, required=True, help="new immutable evidence directory")
    ap.add_argument("--cc", default="cc")
    ap.add_argument("--reuse-tools-from", type=Path, help="reuse paired tools only after all archived source files match")
    ap.add_argument("--external", type=Path, help="optional external diagnostic script; never copied into repository")
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    cc = shutil.which(args.cc)
    assert cc, "C compiler missing"
    archive = run(["git", "-C", args.quickjs, "archive", PIN])
    assert archive.returncode == 0, "pinned QuickJS source unavailable"
    tools = []
    for variant in ["original", "patched"]:
        source = args.out / variant
        source.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive.stdout)) as tar:
            tar.extractall(source, filter="data")
        if variant == "patched":
            p = subprocess.run(["patch", "-s", "-d", str(source), "-p1"], input=PATCH.read_bytes(), capture_output=True)
            assert p.returncode == 0, p.stderr.decode(errors="replace")
        if args.reuse_tools_from:
            previous = args.reuse_tools_from / variant
            for src in source.rglob("*"):
                if src.is_file():
                    old = previous / src.relative_to(source)
                    assert old.exists() and digest(old) == digest(src), (variant, "reused source mismatch", src)
            tool_dir = source / "build"
            tool_dir.mkdir()
            for n in ["qjs", "qjsc"]:
                shutil.copy2(previous / "build" / n, tool_dir / n)
            shutil.copy2(args.reuse_tools_from / (variant + "-build.log"), args.out / (variant + "-build.log"))
            tools.append(tool_dir)
        else:
            tools.append(build(source, args.out / (variant + "-build.log"), cc))
    fixtures = args.out / "fixtures"
    fixtures.mkdir()
    cases = dict(CASES)
    cases["generated_128"] = generated_expressions()
    for depth in [4, 16, 32]:
        cases[f"nested_{depth}"] = "print(" + "(" * depth + "1+2*3" + ")" * depth + ");"
    receipt = {"pin": PIN, "patch_sha256": digest(PATCH), "compiler": cc,
               "compiler_version": run([cc, "--version"]).stdout.decode(),
               "tools": [{n: digest(t / n) for n in ["qjs", "qjsc"]} for t in tools], "cases": []}
    if args.reuse_tools_from:
        receipt["reused_tools"] = {"path": str(args.reuse_tools_from), "all_archived_source_files_verified": True}
    all_cases = [(n, s, False) for n, s in cases.items()] + [(n, s, True) for n, s in SYNTAX.items()]
    if args.external:
        receipt["external"] = {"path": str(args.external), "sha256": digest(args.external), "bytes": args.external.stat().st_size,
                               "scope": "fresh external capture; not asserted byte-identical to guest response"}
        all_cases.append(("external", args.external.read_text(), False))
    for name, text, invalid in all_cases:
        folder = fixtures / name
        folder.mkdir()
        (folder / "fixture.js").write_text(text)
        compiled = [run([t / "qjsc", "-b", "-C", "-o", f"{i}.bin", "fixture.js"], cwd=folder, timeout=10) for i, t in enumerate(tools)]
        assert compiled[0].returncode == compiled[1].returncode, (name, "compile status mismatch")
        assert bool(compiled[0].returncode) == invalid, (name, "unexpected compile result", compiled[0].stderr)
        assert compiled[0].stderr == compiled[1].stderr, (name, "syntax diagnostic mismatch")
        row = {"name": name, "source_sha256": digest(folder / "fixture.js"), "compile_status": compiled[0].returncode}
        if not invalid:
            assert (folder / "0.bin").read_bytes() == (folder / "1.bin").read_bytes(), (name, "serialized bytecode mismatch")
            executed = [run([t / "qjs", "fixture.js"], cwd=folder, timeout=10) for t in tools]
            assert [(p.returncode, p.stdout, p.stderr) for p in executed[:1]] == [(p.returncode, p.stdout, p.stderr) for p in executed[1:]], (name, "result/exception mismatch")
            assert executed[0].returncode in ([0, 1] if name == "external" else [0]), (name, "unexpected runtime failure")
            row.update(bytecode_sha256=digest(folder / "0.bin"), result_status=executed[0].returncode,
                       stdout=executed[0].stdout.decode(errors="replace"), stderr=executed[0].stderr.decode(errors="replace"))
        receipt["cases"].append(row)
    receipt["pass"] = True
    (args.out / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(f"[PASS] QuickJS parser bytecode/results/exceptions: {len(all_cases)} fixtures")


if __name__ == "__main__":
    main()
