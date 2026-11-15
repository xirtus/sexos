"""Canonical staged identity and compiled native opener; no mock-runtime claim."""
import ast
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
import tomllib
ROOT=Path(__file__).resolve().parents[2]
GATE=ROOT/'scripts/shuttle_gate.py'

def function(source,signature):
    at=source.index(signature);depth=0;began=False
    for i in range(at,len(source)):
        if source[i]=='{':depth+=1;began=True
        elif source[i]=='}':
            depth-=1
            if began and depth==0:return source[at:i+1]
    raise AssertionError('unclosed production function')

class ShuttleGazeCompatibility(unittest.TestCase):
    def test_only_canonical_gaze_artifact_constructs_native_fixture(self):
        node=next(n for n in ast.parse(GATE.read_text()).body if isinstance(n,ast.Assign)and any(isinstance(t,ast.Name)and t.id=='FILES'for t in n.targets))
        with tempfile.TemporaryDirectory()as td:
            root=Path(td);stage=root/'iso_root/programs';stage.mkdir(parents=True)
            # Synthetic bytes test source selection only, never a native ELF claim.
            (stage/'shuttle').write_bytes(b'starter');(stage/'gaze').write_bytes(b'canonical-gaze')
            actual=eval(compile(ast.Expression(node.value),str(GATE),'eval'),{'ROOT':root,'b':io.BytesIO(b'picture')})
            self.assertEqual(actual,{'/shuttle':b'starter','/gaze':b'canonical-gaze','/pic.png':b'picture'})
            (stage/'gaze').unlink();(stage/'view').write_bytes(b'old-view')
            with self.assertRaises(FileNotFoundError):eval(compile(ast.Expression(node.value),str(GATE),'eval'),{'ROOT':root,'b':io.BytesIO(b'picture')})

    def test_build_title_and_gate_explicit_launch_identity_agree(self):
        spec=tomllib.loads((ROOT/'sexos_build_spec.toml').read_text())
        stages=[s for s in spec['stage']if s.get('id')=='build_view']
        self.assertEqual(len(stages),1)
        self.assertEqual(stages[0]['dest_artifact'],'iso_root/programs/gaze')
        self.assertEqual(stages[0]['manifest'],'apps/view/Cargo.toml')
        actual=(ROOT/'apps/view/src/main.rs').read_text()
        self.assertIn('window.title(b"Gaze")',actual)
        self.assertIn('[view.ready] target={} task={}',actual)
        source=GATE.read_text()
        self.assertIn("fetched('/gaze','/bin/gaze')",source)
        self.assertIn('run shuttle -a gaze',source)
        self.assertIn('run shuttle -h gaze @pic.png',source)
        self.assertNotIn('/bin/view',source)
        self.assertNotIn('run shuttle -a view',source)
        self.assertIn(r'\[view.ready\] target={m[1]} task={m[2]}',source)

    def test_actual_native_open_run_start_path_selects_only_gaze(self):
        sdk=(ROOT/'crates/sex-app/src/lib.rs').read_text()
        funcs='\n'.join(function(sdk,s)for s in('pub fn run(name:','fn start(name:','pub fn open(path:'))
        prefix='''extern crate self as sex_pdx;
#[path = LOW_LAUNCH] pub mod launch;
#[path = STORE] pub mod program_store;
mod package {pub const GRANT_TOKEN:&[u8]=b"+packages";}
pub mod fs {pub const ARGS_MAX:usize=256;pub const NAME:u64=5;}
pub const SLOT_SHELL:u64=6;
#[derive(Default)]struct State{prepared:Vec<(Vec<u8>,Vec<u8>)>,sent:Vec<(u64,u64,u64,u64,u64)>,denied:bool}
static STATE:std::sync::Mutex<State>=std::sync::Mutex::new(State{prepared:Vec::new(),sent:Vec::new(),denied:false});
fn pdx_call_checked(slot:u64,op:u64,a:u64,b:u64,c:u64)->Result<u64,u64>{STATE.lock().unwrap().sent.push((slot,op,a,b,c));Ok(0)}
mod files {pub type Error=u64;pub mod direct {pub fn prepare(p:&[u8],a:&[u8])->Result<u32,u64>{let mut s=crate::STATE.lock().unwrap();s.prepared.push((p.to_vec(),a.to_vec()));if s.denied{Err(7)}else{Ok(19)}}}}
mod native {use crate::{files,launch as l,program_store as store};#[derive(Debug,PartialEq)]pub enum Error{Files(files::Error),NoProgram,Shell}
'''.replace('LOW_LAUNCH',json.dumps(str(ROOT/'crates/sex-pdx/src/launch.rs'))).replace('STORE',json.dumps(str(ROOT/'crates/sex-pdx/src/program_store.rs')))
        suffix='''}
fn reset(){*STATE.lock().unwrap()=State::default();}
#[test]fn image_types_select_canonical_program_and_only_the_held_file(){
 for ext in ["png","JPG","jpeg","webp","GIF","bmp"] {reset();let p=format!("/picture.{ext}");native::open(p.as_bytes(),77).unwrap();let s=STATE.lock().unwrap();assert_eq!(s.prepared,vec![(b"/bin/gaze".to_vec(),[b"@".as_slice(),p.as_bytes()].concat())]);assert_eq!(s.sent,vec![(SLOT_SHELL,launch::OP_LAUNCH,19,0,77)]);}
}
#[test]fn file_denial_is_propagated_without_shell_launch(){reset();STATE.lock().unwrap().denied=true;assert_eq!(native::open(b"/picture.png",77),Err(native::Error::Files(7)));assert!(STATE.lock().unwrap().sent.is_empty());}
#[test]fn invalid_or_unknown_paths_have_no_prepare_or_launch_side_effect(){
 for p in [b"/picture.nope".as_slice(),b"/../picture.png",b"/x y.png",b"relative.png",b"/x\\0.png"]{reset();assert!(native::open(p,77).is_err());let s=STATE.lock().unwrap();assert!(s.prepared.is_empty()&&s.sent.is_empty());}
}
'''
        with tempfile.TemporaryDirectory()as td:
            src=Path(td)/'native.rs';binary=Path(td)/'native';src.write_text(prefix+funcs+suffix)
            result=subprocess.run(['rustc','--edition=2021','-A','dead_code','--test',str(src),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            result=subprocess.run([str(binary),'--test-threads=1'],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stdout+result.stderr)

if __name__=='__main__':unittest.main()
