// Differential oracle: the pinned prior implementation is a compatibility
// reference only. Neither implementation nor these metadata facades grant IPC.
#[derive(Clone,Copy,Debug)]enum Op{Deny(u8,usize,u32),Accept,Full,Retire(usize),Peek}
fn who(index:usize)->(u32,u32){match index{0=>(12,120),1=>(12,121),2=>(17,170),3=>(0,120),4=>(12,0),5=>(0,0),_=>(100+index as u32,1000+index as u32)}}
fn old_kind(kind:u8)->reference::Kind{match kind{0=>reference::Kind::Entropy,1=>reference::Kind::CertificateTime,2=>reference::Kind::Trust,_=>unreachable!()}}
fn new_kind(kind:u8)->source_denial::Kind{match kind{0=>source_denial::Kind::Entropy,1=>source_denial::Kind::CertificateTime,2=>source_denial::Kind::Trust,_=>unreachable!()}}
fn limit(kind:u8)->u32{match kind{0=>entropy_seed::MAX_SEQ,1=>certificate_time::MAX_SEQ,2=>trust_policy::MAX_SEQ,_=>unreachable!()}}
struct Pair{old:reference::Outbox,new:source_denial::Outbox,steps:usize}
impl Pair{
 fn new()->Self{Self{old:reference::Outbox::new(),new:source_denial::Outbox::new(),steps:0}}
 fn peek(&self)->Option<(u32,u32,u64)>{
  let a=self.old.pending_reply().map(|(id,word)|(id.pd,id.task,word));let b=self.new.pending_reply().map(|(id,word)|(id.pd,id.task,word));
  assert_eq!(a,b,"exact pending identity/typed word diverged after {} operations",self.steps);
  assert_eq!(self.old.pending_reply().map(|(id,word)|(id.pd,id.task,word)),a,"old Peek mutated");
  assert_eq!(self.new.pending_reply().map(|(id,word)|(id.pd,id.task,word)),b,"candidate Peek mutated");b
 }
 fn step(&mut self,op:Op){self.peek();match op{
  Op::Deny(kind,index,seq)=>{let(pd,task)=who(index);let a=self.old.deny(old_kind(kind),reference::Identity{pd,task},seq);let b=self.new.deny(new_kind(kind),source_denial::Identity{pd,task},seq);assert_eq!(a,b,"Deny admission differs: {:?}",op);},
  Op::Accept=>{self.old.reply_accepted();self.new.reply_accepted();},
  Op::Full=>{self.old.reply_deferred();self.new.reply_deferred();},
  Op::Retire(index)=>{let(pd,task)=who(index);assert_eq!(self.old.retire(reference::Identity{pd,task}),self.new.retire(source_denial::Identity{pd,task}),"Retire return differs: {:?}",op);},
  Op::Peek=>{self.peek();},
 }self.steps+=1;self.peek();}
}
#[test]fn differential_short_histories(){
 assert_eq!(app_catalog::COUNT,2);let mut histories=0;let mut events=0;
 for kind in 0..3{let high=limit(kind);let alphabet=[Op::Deny(kind,0,1),Op::Deny(kind,0,2),Op::Deny(kind,0,high),Op::Deny(kind,0,0),Op::Deny(kind,0,high+1),Op::Deny((kind+1)%3,1,1),Op::Deny(kind,1,1),Op::Deny(kind,2,1),Op::Deny(kind,3,1),Op::Accept,Op::Full,Op::Retire(0),Op::Retire(1),Op::Peek];
  for mut encoded in 0..alphabet.len().pow(4){let mut pair=Pair::new();for _ in 0..4{pair.step(alphabet[encoded%alphabet.len()]);encoded/=alphabet.len();events+=1;}histories+=1;}
 }
 println!("[denial.differential.short] owners=2 histories={} operations={} depth=4 alphabet=14 protocols=3",histories,events);assert_eq!((histories,events),(115248,460992));
}
fn random(state:&mut u64)->u64{let mut value=*state;value^=value<<13;value^=value>>7;value^=value<<17;*state=value;value}
#[test]fn differential_long_traces(){
 assert!(matches!(app_catalog::COUNT,28|64));let mut operations=0;
 for seed in[0x1234_5678_9abc_def1,0x55aa_55aa_55aa_55aa,0x9e37_79b9_7f4a_7c15]{let mut pair=Pair::new();
  // Fill every partition first, exercise Full cursor wrap, then cached-owner
  // refusal, stale pending replies, retirement and repeated Peek under load.
  for kind in 0..3{for owner in 6..6+app_catalog::COUNT{pair.step(Op::Deny(kind,owner,3));operations+=1;}}
  for _ in 0..6*app_catalog::COUNT{pair.step(Op::Full);operations+=1;}
  let mut state=seed;
  for _ in 0..100000{let choice=random(&mut state);let kind=((choice>>5)%3)as u8;let owner=((choice>>8)as usize)%(app_catalog::COUNT+7);let high=limit(kind);
   let seq=match(choice>>24)%9{0=>0,1=>1,2=>2,3=>3,4=>high-1,5=>high,6=>high+1,7=>1<<31,_=>u32::MAX};
   let op=match choice%16{0..=9=>Op::Deny(kind,owner,seq),10..=11=>Op::Accept,12..=13=>Op::Full,14=>Op::Retire(owner),_=>Op::Peek};pair.step(op);operations+=1;
  }
 }
 println!("[denial.differential.long] owners={} seeds=3 random_operations=300000 total_operations={}",app_catalog::COUNT,operations);assert_eq!(operations,300000+27*app_catalog::COUNT);
}
