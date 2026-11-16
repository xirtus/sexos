"""Actual private snapshot/output model; no real filesystem or permission mock."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest
ROOT = Path(__file__).resolve().parents[2]


class JobSnapshots(unittest.TestCase):
    def test_actual_immutable_job_model(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / 'src').mkdir()
            (root / 'Cargo.toml').write_text('''[package]
name="restricted-job-test"
version="0.0.0"
edition="2021"
[dependencies]
sha2={version="=0.10.9",default-features=false,features=["force-soft"]}
zeroize={version="=1.9.1",default-features=false,features=["alloc"]}
[workspace]
''')
            source = 'extern crate self as sex_pdx;\n#[path="' + str(ROOT / 'crates/sex-pdx/src/restricted_helper.rs') + '"] pub mod restricted_helper;\n#[path="' + str(ROOT / 'servers/sexfiles/src/restricted_job.rs') + '"] mod job;\n' + r'''
#[cfg(test)]mod tests{
use super::*;use job::*;use restricted_helper::{Subject,Binding};
fn parent()->Subject{Subject{pd:42,task:99}}fn child()->Subject{Subject{pd:43,task:100}}
fn request()->Vec<u8>{let mut data=Vec::new();data.extend_from_slice(b"MOANREQ1");data.extend_from_slice(&[1;16]);for v in [48000u32,1,2,0]{data.extend_from_slice(&v.to_le_bytes());}data.extend_from_slice(&0.25f32.to_le_bytes());data.extend_from_slice(&(-0.5f32).to_le_bytes());data}
fn capture(id:u64,data:&[u8],limit:usize)->Source{Source::capture(Object{volume:1,id},data.len() as u64,limit,|at,out|{let at=at as usize;let n=out.len().min(data.len()-at);out[..n].copy_from_slice(&data[at..at+n]);Ok(n)}).unwrap()}
fn make()->Job{Job::new(7,parent(),9,[1;16],capture(1,b"module",MAX_PLUGIN),capture(2,&request(),MAX_REQUEST)).unwrap()}
fn words(hash:[u8;32])->[u64;4]{core::array::from_fn(|i|u64::from_le_bytes(hash[i*8..i*8+8].try_into().unwrap()))}
fn binding(job:&Job)->Binding{Binding{generation:1,coordinator_epoch:1,provider_epoch:1,previous:Subject{pd:43,task:43},child:child(),parent:parent(),cookie:9,nonce:[0x0101010101010101;2],plugin_hash:words(job.plugin_hash()),input_hash:words(job.input_hash()),ticket:7}}
#[test]fn identity_exact_hashes_and_immutable_snapshot(){
 let mut original=request();let source=capture(2,&original,MAX_REQUEST);original[40]=99;
 let mut job=Job::new(7,parent(),9,[1;16],capture(1,b"module",MAX_PLUGIN),source).unwrap();let b=binding(&job);
 let mut wrong=b;wrong.plugin_hash[0]^=1;assert_eq!(job.bind(wrong),Err(Error::Identity));job.bind(b).unwrap();
 assert_eq!(job.read(child(),b,false,Blob::Request,0,48).err(),Some(Error::Identity));
 assert_eq!(job.read(Subject{pd:43,task:101},b,true,Blob::Request,0,48).err(),Some(Error::Identity));
 assert_eq!(job.read(child(),b,true,Blob::Request,0,48).unwrap(),request());
 assert_eq!(job.create(child(),b,true,Blob::Plugin),Err(Error::ReadOnly));assert_eq!(job.create(child(),b,true,Blob::Request),Err(Error::ReadOnly));
}
#[test]fn append_bounds_seal_and_parent_result_no_substitution(){
 let mut job=make();let b=binding(&job);job.bind(b).unwrap();job.create(child(),b,true,Blob::Output).unwrap();job.truncate(child(),b,true,Blob::Output,0).unwrap();
 assert_eq!(job.write(child(),b,true,Blob::Output,u64::MAX,1,|_|panic!("out-of-bound read")),Err(Error::Bounds));
 assert_eq!(job.write(child(),b,true,Blob::Output,0,49,|_|panic!("oversize read")),Err(Error::Bounds));
 assert_eq!(job.seal(child(),b,true),Err(Error::State));let mut out=request();out[..8].copy_from_slice(b"MOANPCM1");
 job.write(child(),b,true,Blob::Output,0,out.len(),|i|out[i]).unwrap();let hash=job.seal(child(),b,true).unwrap();
 assert_eq!(job.write(child(),b,true,Blob::Output,0,1,|_|0),Err(Error::State));assert!(job.result(parent(),7,[1;16]).is_err());
 job.published();assert_eq!(job.result(parent(),7,[1;16]).unwrap().0,out);assert_eq!(job.result(parent(),7,[1;16]).unwrap().1,hash);
 assert_eq!(job.result(Subject{pd:42,task:100},7,[1;16]).err(),Some(Error::Identity));assert!(job.result(parent(),8,[1;16]).is_err());assert!(job.result(parent(),7,[2;16]).is_err());
 job.retire_child();assert_eq!(job.bind(b),Err(Error::Identity));assert!(job.read(child(),b,true,Blob::Plugin,0,1).is_err());assert_eq!(job.result(parent(),7,[1;16]).unwrap().0,out);
}
#[test]fn allocations_are_bounded_alias_and_malformed_input_refused(){
 assert_eq!(Source::capture(Object{volume:1,id:1},MAX_PLUGIN as u64+1,MAX_PLUGIN,|_,_|panic!("read before bounds")).err(),Some(Error::Bounds));
 assert_eq!(Source::capture(Object{volume:1,id:1},4,4,|_,out|Ok(out.len()+1)).err(),Some(Error::Bounds));
 assert_eq!(Source::capture(Object{volume:1,id:1},4,4,|_,out|{out.fill(1);Ok(out.len())}).err(),Some(Error::Bounds));
 assert_eq!(Job::new(7,parent(),9,[1;16],capture(2,b"module",MAX_PLUGIN),capture(2,&request(),MAX_REQUEST)).err(),Some(Error::Alias));
 let mut input=request();input[40..44].copy_from_slice(&f32::NAN.to_le_bytes());assert_eq!(Job::new(7,parent(),9,[1;16],capture(1,b"module",MAX_PLUGIN),capture(2,&input,MAX_REQUEST)).err(),Some(Error::Audio));
 let mut input=request();input[28..32].copy_from_slice(&u32::MAX.to_le_bytes());assert!(Job::new(7,parent(),9,[1;16],capture(1,b"module",MAX_PLUGIN),capture(2,&input,MAX_REQUEST)).is_err());
}
#[test]fn malformed_output_and_read_budget_cannot_publish(){
 let mut job=make();let b=binding(&job);job.bind(b).unwrap();
 for _ in 0..2{assert_eq!(job.read(child(),b,true,Blob::Plugin,0,6).unwrap(),b"module");}assert_eq!(job.read(child(),b,true,Blob::Plugin,0,1).err(),Some(Error::Budget));
 job.create(child(),b,true,Blob::Output).unwrap();let mut out=request();out[..8].copy_from_slice(b"MOANPCM1");out[8]=2;
 job.write(child(),b,true,Blob::Output,0,out.len(),|i|out[i]).unwrap();assert_eq!(job.seal(child(),b,true),Err(Error::Header));assert!(job.publication_bytes().is_err());job.published();assert!(job.result(parent(),7,[1;16]).is_err());
}
}
'''
            (root / 'src/lib.rs').write_text(source)
            env = os.environ.copy()
            env['CARGO_TARGET_DIR'] = str(ROOT / 'target/restricted-job-tests')
            result = subprocess.run(['cargo', 'test', '--offline', '--manifest-path', str(root / 'Cargo.toml'), '--lib'], cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
