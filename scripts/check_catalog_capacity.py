#!/usr/bin/env python3
"""Compile expanded shared catalog consumers; no BOOT authority claim."""
import json
import re
from pathlib import Path
import subprocess
import tempfile


def main():
    root=Path(__file__).resolve().parents[1]
    source=(root/'crates/sex-pdx/src/app_catalog.rs').read_text()
    match=re.search(r'pub const INCARNATIONS: \[Incarnation; \d+\] = \[',source);assert match
    start=match.start();end=source.index('\n];',start)+3
    with tempfile.TemporaryDirectory(prefix='sexos-catalog-capacity-')as temporary:
        out=Path(temporary)
        for size in (7,64,65):
            rows='\n'.join(f'quil({i},{100+i},{1000+i},None,{1000+4*i},{1001+4*i},'+('None'if i==0 else f'Some({1002+4*i})')+f',{1003+4*i}),'for i in range(size))
            catalog=out/f'catalog-{size}.rs';catalog.write_text(source[:start]+f'pub const INCARNATIONS: [Incarnation; {size}] = [\n'+rows+'\n];'+source[end:])
            paths=[('app_catalog',catalog),('lifecycle',root/'crates/sex-pdx/src/app_lifecycle.rs'),('resources',root/'kernel/src/app_resource_state.rs'),('display',root/'servers/sexdisplay/src/surface_lifecycle.rs'),('document',root/'crates/sex-pdx/src/document_buffer.rs')]
            fixture=out/f'fixture-{size}.rs'
            fixture.write_text('#![allow(dead_code)]\nextern crate self as sex_pdx;\n'+'\n'.join(f'#[path={json.dumps(str(path))}]pub mod {name};'for name,path in paths)+f'''
#[test]fn expanded_consumers_remain_distinct_and_bounded(){{
 assert_eq!(resources::MAX_APPS,{size});assert_eq!(display::CATALOG_CAPACITY,{size});
 assert_eq!(lifecycle::SLOTS.len(),{size});assert_eq!(document::CLIENT_COUNT,{size});
 assert_eq!(app_catalog::QUIL_TARGETS.len(),{size});let mut bits=0u64;
 for i in 0..{size}{{
  assert_eq!(lifecycle::SLOTS[i],1000+4*i);assert_eq!(app_catalog::QUIL_TARGETS[i],100+i as u32);
  assert_eq!(document::PROVIDER_SLOTS[i],64+i as u64);let id=document::region_id(i);assert!(id!=9&&id!=10);
  for earlier in 0..i{{assert_ne!(id,document::region_id(earlier));}}
  let bit=app_catalog::native_pixel_bit(100+i as u32,1000+i as u64);assert_eq!(bit,1u64<<i);
  assert_eq!(bits&bit,0);bits|=bit;assert_eq!(app_catalog::native_pixel_bit(100+i as u32,9999),0);
 }}
 assert_eq!(document::region_id(0),6);assert_eq!(document::region_id(1),7);assert_eq!(document::region_id(2),8);assert_eq!(document::region_id(3),0x2000);
}}
''')
            binary=out/f'test-{size}';compiled=subprocess.run(['rustc','--edition=2021','--test',str(fixture),'-o',str(binary)],text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            if size==65:
                if compiled.returncode==0 or 'COUNT <= PRESENTATION_BITS'not in compiled.stdout:raise RuntimeError('explicit capacity guard must reject65 rows\n'+compiled.stdout)
                print('[catalog.capacity.control] rows=65 compile_rejected=1')
            else:
                if compiled.returncode:raise RuntimeError(compiled.stdout)
                subprocess.run([str(binary)],check=True);print(f'[catalog.capacity] rows={size} shared_consumers=4 distinct_mask_bits=1 document_namespace=1')


if __name__=='__main__':main()
