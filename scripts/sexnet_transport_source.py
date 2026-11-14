"""Keep the isolated transport tests on the exact selected production sources."""
from pathlib import Path
import hashlib
import json
import shutil
import tomllib
from check_tls_platform import SMOLTCP_PATCH, check_workspace_patches

VENDOR=SMOLTCP_PATCH['path']
PROVENANCE_SHA256='902a85175ae69e081581d409a8af203b6ae7c3bbf81b2c9a2c64bd73e33fc1b9'
ARCHIVE_SHA256='b6f8b28ad56c6e35524a37dd492af5d1a47e31e1a4d175cd12f89c075f01980f'
CHANGED={'src/iface/interface/mod.rs','src/iface/neighbor.rs'}


def selected_patch(root):
    workspace=tomllib.loads((root/'Cargo.toml').read_text())
    errors=check_workspace_patches(workspace)
    if errors:raise ValueError('; '.join(errors))
    return workspace.get('patch',{}).get('crates-io',{}).get('smoltcp')


def vendor_custody(base):
    """Pinned published closure plus exactly the reviewed two-file delta."""
    base=Path(base)
    if base.is_symlink() or base.resolve()!=base:raise ValueError('noncanonical smoltcp vendor directory')
    p=base/'UPSTREAM.json'
    if p.is_symlink():raise ValueError('smoltcp provenance symlink')
    raw=p.read_bytes()
    if hashlib.sha256(raw).hexdigest()!=PROVENANCE_SHA256:raise ValueError('unreviewed smoltcp provenance')
    receipt=json.loads(raw)
    if (receipt.get('schema')!=1 or receipt.get('name')!='smoltcp' or receipt.get('version')!='0.14.0'
        or receipt.get('archive_sha256')!=ARCHIVE_SHA256 or receipt.get('original_members')!=134
        or set(receipt.get('changed_upstream',[]))!=CHANGED):raise ValueError('noncanonical smoltcp archive identity')
    expected={'UPSTREAM.json'};originals=0;changed=set();hashes={'UPSTREAM.json':PROVENANCE_SHA256}
    for record in receipt['files']:
        name=record['path'];relative=Path(name)
        if relative.is_absolute() or '..'in relative.parts or name in expected:raise ValueError('invalid smoltcp source path')
        expected.add(name);p=base/relative
        if p.is_symlink() or p.resolve()!=p:raise ValueError('smoltcp source symlink')
        data=p.read_bytes();digest=hashlib.sha256(data).hexdigest();hashes[name]=digest
        if len(data)!=record['bytes']or digest!=record['sha256']:raise ValueError('smoltcp source changed: '+name)
        if 'upstream_sha256'in record:
            originals+=1
            if digest!=record['upstream_sha256']:changed.add(name)
    actual={p.relative_to(base).as_posix()for p in base.rglob('*')if p.is_file()or p.is_symlink()}
    if actual!=expected or originals!=134 or changed!=CHANGED:raise ValueError('smoltcp source closure or reviewed delta changed')
    license_record=next(r for r in receipt['files']if r['path']=='LICENSE-0BSD.txt')
    if license_record['sha256']!=license_record['upstream_sha256']:raise ValueError('smoltcp upstream license changed')
    return hashes


def prepare(root,out):
    root=Path(root).resolve();out=Path(out).resolve();selection=selected_patch(root)
    if selection is not None:vendor_custody(root/VENDOR)
    out.mkdir(parents=True)
    shutil.copytree(root/'crates/sex-net/src',out/'src')
    shutil.copyfile(root/'Cargo.lock',out/'Cargo.lock')
    shutil.copytree(root/'crates/sex-net-config',out.parent/'sex-net-config')
    manifest=(root/'crates/sex-net/Cargo.toml').read_text()+'\n[workspace]\n'
    if selection is not None:
        copied=out.parents[1]/VENDOR
        shutil.copytree(root/VENDOR,copied)
        if vendor_custody(copied)!=vendor_custody(root/VENDOR):raise ValueError('isolated smoltcp copy changed')
        manifest+='\n[patch.crates-io]\nsmoltcp = { path = "../../'+VENDOR+'" }\n'
    (out/'Cargo.toml').write_text(manifest)


def verify_selection(root,out,metadata):
    root=Path(root).resolve();out=Path(out).resolve()
    packages=[p for p in metadata['packages']if p['name']=='smoltcp']
    if len(packages)!=1 or packages[0]['version']!='0.14.0':raise ValueError('wrong isolated smoltcp package')
    if selected_patch(root) is not None:
        copied=out.parents[1]/VENDOR
        if packages[0]['source'] is not None or Path(packages[0]['manifest_path']).resolve()!=copied/'Cargo.toml':raise ValueError('wrong isolated smoltcp manifest/source')
        if vendor_custody(copied)!=vendor_custody(root/VENDOR):raise ValueError('isolated smoltcp source custody changed')
    elif packages[0]['source']!='registry+https://github.com/rust-lang/crates.io-index':raise ValueError('nonregistry default smoltcp source')
