"""Host coverage for non-ASCII native grid cells, including installed fallbacks.

The temporary host program links the locked sex-font crate. It uses the same
public Mono/fallback raster API as Display, without importing its pixel shader.
"""
import json
import atexit
from pathlib import Path
import subprocess
import tempfile
from functools import lru_cache

ROOT = Path(__file__).resolve().parents[1]
_hosts = {}

HOST = r'''
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (w,h,c): (usize,usize,u32) = (a[0].parse().unwrap(),a[1].parse().unwrap(),a[2].parse().unwrap());
    assert!(w>0 && w<=192 && h>0 && h<=144 && char::from_u32(c).is_some());
    let font=sex_font::Font::new(sex_font::GEIST_MONO).unwrap();
    let mut acc=vec![0;sex_font::raster::scratch_len(w,h)];
    let mut out=vec![0;w*h];
    let found=sex_font::draw_cell(&font,c,w,h,&mut acc,&mut out)
        || sex_font::fallback::Fallback::embedded().unwrap().draw_cell(c,w,h,&mut acc,&mut out);
    println!("{}",if found {"found"} else {"missing"});
    if found { println!("{}",out.iter().map(u8::to_string).collect::<Vec<_>>().join(",")); }
}
'''


def host(root):
    root = Path(root).resolve()
    if root not in _hosts:
        result = subprocess.run(['cargo', 'build', '--locked', '--manifest-path',
                                 str(root/'Cargo.toml'), '-p', 'sex-font', '--lib',
                                 '--message-format=json'], check=True, capture_output=True, text=True, timeout=120)
        libraries = []
        for line in result.stdout.splitlines():
            item = json.loads(line)
            if item.get('reason') == 'compiler-artifact' and item['target']['name'] == 'sex_font':
                libraries += [Path(f) for f in item['filenames'] if f.endswith('.rlib')]
        if len(libraries) != 1:
            raise ValueError('missing unique locked host font library')
        directory = tempfile.TemporaryDirectory(prefix='sexos-scalar-atlas-')
        source = Path(directory.name)/'atlas.rs'
        executable = Path(directory.name)/'atlas'
        source.write_text(HOST)
        build = subprocess.run(['rustc', '--edition=2024', str(source), '--extern',
                        'sex_font='+str(libraries[0]), '-L', 'dependency='+str(libraries[0].parent),
                        '-L', 'dependency='+str(libraries[0].parent/'deps'),
                        '-o', str(executable)], capture_output=True, text=True, timeout=60)
        if build.returncode:
            directory.cleanup()
            raise ValueError('host scalar atlas compilation failed: '+build.stderr)
        atexit.register(directory.cleanup)
        _hosts[root] = directory, executable
    return _hosts[root][1]


@lru_cache(maxsize=1024)
def coverage(scalar, width, height, root=ROOT):
    if not (0 <= scalar <= 0x10ffff and not 0xd800 <= scalar <= 0xdfff
            and 1 <= width <= 192 and 1 <= height <= 144):
        raise ValueError('invalid scalar atlas request')
    lines = subprocess.run([str(host(root)), str(width), str(height), str(scalar)],
                           check=True, capture_output=True, text=True, timeout=15).stdout.splitlines()
    if lines == ['missing']:
        return None
    if len(lines) != 2 or lines[0] != 'found':
        raise ValueError('malformed scalar atlas receipt')
    cell = bytes(map(int, lines[1].split(',')))
    if len(cell) != width*height:
        raise ValueError('incomplete scalar atlas coverage')
    return cell
