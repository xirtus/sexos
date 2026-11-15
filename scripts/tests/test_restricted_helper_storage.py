"""Real sexfs snapshot/new-object publication, including adversarial replacement."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest
ROOT = Path(__file__).resolve().parents[2]


class SystemJobStorage(unittest.TestCase):
    def test_actual_filesystem_identity_and_snapshot(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / 'src').mkdir()
            (root / 'Cargo.toml').write_text('''[package]
name="restricted-storage-test"
version="0.0.0"
edition="2021"
[dependencies]
sha2={version="=0.10.9",default-features=false,features=["force-soft"]}
zeroize={version="=1.9.1",default-features=false,features=["alloc"]}
sexfs={path="''' + str(ROOT / 'crates/sexfs') + '''"}
[workspace]
''')
            code = 'extern crate self as sex_pdx;\n'
            for module, path in [('restricted_helper','crates/sex-pdx/src/restricted_helper.rs'), ('restricted_job','servers/sexfiles/src/restricted_job.rs'), ('storage','servers/sexfiles/src/restricted_storage.rs')]:
                code += '#[path="' + str(ROOT / path) + '"] mod ' + module + ';\n'
            code += r'''
#[cfg(test)]mod tests{
use super::*;use sexfs::*;use restricted_job::{Job,Blob,Error as JobError};use restricted_helper::{Subject,Binding};
struct Disk(Vec<[u8;BLOCK]>);impl Device for Disk{
fn blocks(&self)->u64{self.0.len() as u64}fn read(&mut self,b:u64,out:&mut[u8;BLOCK])->Result<(),()>{*out=self.0[b as usize];Ok(())}
fn write(&mut self,b:u64,data:&[u8;BLOCK])->Result<(),()>{self.0[b as usize]=*data;Ok(())}fn flush(&mut self)->Result<(),()>{Ok(())}}
fn fs()->Fs<'static,Disk>{let n=2048;Fs::format(Disk(vec![[0;BLOCK];n]),Memory{cache:Box::leak(vec![[0;BLOCK];64].into_boxed_slice()),slots:Box::leak(vec![Slot::EMPTY;64].into_boxed_slice()),used:Box::leak(vec![0;n/64].into_boxed_slice()),pending:Box::leak(vec![0;n/64].into_boxed_slice()),scratch:Box::leak(Box::new([0;BLOCK]))}).unwrap()}
fn request()->Vec<u8>{let mut b=Vec::new();b.extend_from_slice(b"MOANREQ1");b.extend_from_slice(&[1;16]);for n in [48000u32,1,2,0]{b.extend_from_slice(&n.to_le_bytes());}for n in [0.25f32,-0.5]{b.extend_from_slice(&n.to_le_bytes());}b}
fn setup()->Fs<'static,Disk>{let mut fs=fs();fs.mkdir(b"/renders",0).unwrap();let p=fs.create(b"/plugin",0).unwrap();fs.write(p,0,b"original-plugin",0).unwrap();let i=fs.create(b"/input",0).unwrap();fs.write(i,0,&request(),0).unwrap();fs.commit().unwrap();fs}
fn capture(fs:&mut Fs<'static,Disk>)->(storage::Storage,Job){storage::Storage::capture(fs,[b"@/plugin",b"@/input",b"@/renders/output"],[b"/plugin",b"/input",b"/renders/output"],7,Subject{pd:42,task:99},9,[1;16]).unwrap()}
fn ready(job:&mut Job)->Vec<u8>{let words=|h:[u8;32]|core::array::from_fn(|i|u64::from_le_bytes(h[i*8..i*8+8].try_into().unwrap()));let child=Subject{pd:43,task:100};let b=Binding{generation:1,coordinator_epoch:1,provider_epoch:1,previous:child,child,parent:Subject{pd:42,task:99},cookie:9,nonce:[0x0101010101010101;2],plugin_hash:words(job.plugin_hash()),input_hash:words(job.input_hash()),ticket:7};job.bind(b).unwrap();assert_eq!(job.read(child,b,true,Blob::Plugin,0,99).unwrap(),b"original-plugin");job.create(child,b,true,Blob::Output).unwrap();let mut output=request();output[..8].copy_from_slice(b"MOANPCM1");job.write(child,b,true,Blob::Output,0,output.len(),|i|output[i]).unwrap();job.seal(child,b,true).unwrap();output}
#[test]fn input_mutation_cannot_change_captured_bytes_new_output_matches_private_seal(){
 let mut fs=setup();let (disk,mut job)=capture(&mut fs);let plugin=fs.open(b"/plugin").unwrap().id;fs.truncate(plugin,0,1).unwrap();fs.write(plugin,0,b"replacement",1).unwrap();fs.commit().unwrap();let output=ready(&mut job);
 disk.publish(&mut fs,job.publication_bytes().unwrap(),2,||true).unwrap();job.published();let stat=fs.open(b"/renders/output").unwrap();let mut data=vec![0;stat.size as usize];fs.read(stat.id,0,&mut data).unwrap();assert_eq!(data,output);assert_eq!(job.result(Subject{pd:42,task:99},7,[1;16]).unwrap().0,output);
}
#[test]fn occupied_or_replaced_directory_never_redirects_writes(){
 let mut fs=setup();let (disk,mut job)=capture(&mut fs);ready(&mut job);let id=fs.create(b"/renders/output",1).unwrap();fs.write(id,0,b"foreign",1).unwrap();fs.commit().unwrap();assert_eq!(disk.publish(&mut fs,job.publication_bytes().unwrap(),2,||true),Err(JobError::Alias));let mut data=[0;7];fs.read(id,0,&mut data).unwrap();assert_eq!(&data,b"foreign");
 fs.remove_path(b"/renders/output").unwrap();fs.remove_path(b"/renders").unwrap();fs.mkdir(b"/renders",2).unwrap();fs.commit().unwrap();assert_eq!(disk.publish(&mut fs,job.publication_bytes().unwrap(),3,||true),Err(JobError::Identity));assert!(fs.open(b"/renders/output").is_err());
}
#[test]fn revoked_lease_never_successfully_publishes_and_no_alias_normalization(){
 let mut fs=setup();let (disk,mut job)=capture(&mut fs);ready(&mut job);let mut checks=0;
 assert_eq!(disk.publish(&mut fs,job.publication_bytes().unwrap(),2,||{checks+=1;checks<3}),Err(JobError::Identity));assert!(fs.open(b"/renders/output").is_err());assert!(job.result(Subject{pd:42,task:99},7,[1;16]).is_err());
 for path in [b"relative".as_slice(),b"/a/../b",b"/a//b",b"/a/./b",b"/a/",b"/space here"]{assert!(!storage::canonical(path));}
 let mismatch=storage::Storage::capture(&mut fs,[b"@/other",b"@/input",b"@/renders/output"],[b"/plugin",b"/input",b"/renders/output"],8,Subject{pd:42,task:99},9,[1;16]);assert_eq!(mismatch.err(),Some(JobError::Bounds));
 let result=storage::Storage::capture(&mut fs,[b"@/media/plug",b"@/input",b"@/renders/output"],[b"/media/plug",b"/input",b"/renders/output"],8,Subject{pd:42,task:99},9,[1;16]);assert_eq!(result.err(),Some(JobError::Unsupported));
}
}
'''
            code = code.replace('->Result<(),()>', '->core::result::Result<(),()>')
            (root / 'src/lib.rs').write_text(code)
            env = os.environ.copy()
            env['CARGO_TARGET_DIR'] = str(ROOT / 'target/restricted-storage-tests')
            result = subprocess.run(['cargo', 'test', '--offline', '--manifest-path', str(root / 'Cargo.toml'), '--lib'], cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
