#!/usr/bin/env python3
"""Freeze a sealed GREEN build's exact inputs and native checker sources."""
import argparse
import hashlib
import json
import pathlib
import shutil
import tempfile


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def local(root, name):
    path=(root/name).resolve(strict=True)
    if pathlib.Path(name).is_absolute() or not path.is_relative_to(root) or not path.is_file():
        raise ValueError('source outside repository: '+name)
    return path


def freeze(root, build, out, iso, kernel):
    root=pathlib.Path(root).resolve(strict=True);build=pathlib.Path(build).resolve(strict=True)
    out=pathlib.Path(out).resolve();out.mkdir(parents=True,exist_ok=True)
    names=['source-binding.json','source','tested.iso','tested.kernel']
    if any((out/name).exists()for name in names):raise FileExistsError('existing evidence must not be overwritten')
    provenance=json.loads((build/'provenance.json').read_text())
    sources=provenance.get('production_source_sha256')
    if (provenance.get('build_gate')!='PASS' or provenance.get('source_image_binding')!='sealed_build_this_run'
        or provenance.get('sources_unchanged_during_build') is not True or not isinstance(sources,dict) or not sources
        or '| **FINAL_SCORE** | **GREEN_MASTER** |' not in (build/'report.md').read_text()):
        raise ValueError('sealed GREEN build with unchanged production source manifest required')
    iso=pathlib.Path(iso).resolve(strict=True);kernel=pathlib.Path(kernel).resolve(strict=True)
    for path,expected in [(iso,provenance.get('iso_sha256')),(kernel,provenance.get('kernel_sha256'))]:
        if not expected or digest(path)!=expected:raise ValueError('tested image/kernel differs from build: '+str(path))
    paths={name:local(root,name)for name in sources}
    for name,path in paths.items():
        if digest(path)!=sources[name]:raise ValueError('production source changed since build: '+name)
    runner_names={str(p.relative_to(root)) for directory,suffixes in [('scripts',{'.py','.sh'}),('tests',{'.rs','.py','.sh'})]
                  for p in (root/directory).rglob('*') if p.is_file() and p.suffix in suffixes and '__pycache__'not in p.parts}
    # Architectural host gates need their declarative inventory too. It is
    # checker input, independently hashed rather than misrepresented as kernel
    # production source. Frozen gates can now run without the mutable checkout.
    runner_names.update(str(p.relative_to(root))for p in (root/'spec').glob('*.sexspec')if p.is_file())
    for directory in ['tests','scripts/tests/fixtures']:
        runner_names.update(str(p.relative_to(root))for p in (root/directory).rglob('*')
                            if p.is_file() and '__pycache__'not in p.parts)
    runners={name:local(root,name) for name in runner_names}
    runner_hashes={name:digest(path)for name,path in runners.items()}
    binding=dict(iso_sha256=digest(iso),kernel_sha256=digest(kernel),
                 architecture_sha256=sources.get('ARCHITECTURE.md'),sources=sources,
                 runner_sha256=runner_hashes,build_provenance=provenance,
                 scope='sealed production inputs unchanged across build; scripts and host-test sources recorded separately')
    # Validate copies before publishing the binding. The final JSON is written
    # last, so a partial copy can never masquerade as accepted source evidence.
    with tempfile.TemporaryDirectory(prefix='.freeze-',dir=out)as temporary:
        stage=pathlib.Path(temporary);snapshot=stage/'source';snapshot.mkdir()
        for name,path in {**paths,**runners}.items():
            target=snapshot/name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(path,target)
            # Some gates execute their checker directly after testing -x.
            # Preserve modes so an archive is a runnable source snapshot.
            shutil.copymode(path,target)
            expected=runner_hashes[name]if name in runners else sources[name]
            if digest(target)!=expected:raise ValueError('source changed while copying: '+name)
        for path,name in [(iso,'tested.iso'),(kernel,'tested.kernel')]:
            shutil.copyfile(path,stage/name)
            if digest(stage/name)!=digest(path):raise ValueError('artifact changed while copying')
        # Check against the sealed digest again, including after a concurrent
        # change to the original artifact during the copy above.
        if digest(stage/'tested.iso')!=binding['iso_sha256'] or digest(stage/'tested.kernel')!=binding['kernel_sha256']:
            raise ValueError('artifact changed while freezing')
        for name in names[1:]:(stage/name).rename(out/name)
        (out/'source-binding.json').write_text(json.dumps(binding,indent=2,sort_keys=True)+'\n')
    return binding


def main():
    p=argparse.ArgumentParser();p.add_argument('--root',type=pathlib.Path,default=pathlib.Path(__file__).resolve().parents[1]);p.add_argument('--build',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--iso',type=pathlib.Path,required=True);p.add_argument('--kernel',type=pathlib.Path,required=True);a=p.parse_args()
    result=freeze(a.root,a.build,a.out,a.iso,a.kernel)
    print('[evidence.freeze] PASS production_files='+str(len(result['sources']))+' iso_sha256='+result['iso_sha256'])
if __name__=='__main__':main()
