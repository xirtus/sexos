// SPDX-License-Identifier: GPL-3.0-only
//! Source-bound x86 stack/copy model. This does not execute the syscall stub,
//! PKRU, SYSRET, scheduler, interrupt entry, or native IPC delivery.
use std::collections::BTreeMap;
include!(env!("SYSCALL_RETURN_SOURCE"));
const OUTPUTS:[&str;5]=["rdi","rsi","rdx","r10","r8"];

fn saved_names(asm:&[&str])->Vec<String>{
    let end=asm.iter().position(|&s|s=="mov rbp, rsp").unwrap();let mut names=Vec::new();let mut rax_count=0;
    for instruction in &asm[..end]{if let Some(reg)=instruction.strip_prefix("push "){
        let name=if reg=="qword ptr gs:[24]"{"user_rsp"}else if reg=="rax"{rax_count+=1;if rax_count==1{"rax"}else{assert_eq!(rax_count,2);"pkru"}}else{reg};names.push(name.to_string());
    }}assert_eq!(rax_count,2);names.reverse();names
}
fn offset(instruction:&str,prefix:&str,suffix:&str)->usize{instruction.strip_prefix(prefix).unwrap().strip_suffix(suffix).unwrap().parse().unwrap()}
fn field_offset(name:&str)->usize{match name{
    "rax"=>core::mem::offset_of!(SyscallRegs,rax),"rdi"=>core::mem::offset_of!(SyscallRegs,rdi),"rsi"=>core::mem::offset_of!(SyscallRegs,rsi),"rdx"=>core::mem::offset_of!(SyscallRegs,rdx),"r10"=>core::mem::offset_of!(SyscallRegs,r10),"r8"=>core::mem::offset_of!(SyscallRegs,r8),"r9"=>core::mem::offset_of!(SyscallRegs,r9),"rcx"=>core::mem::offset_of!(SyscallRegs,rcx),"r11"=>core::mem::offset_of!(SyscallRegs,r11),_=>panic!("unknown SyscallRegs field")}}
fn inputs(selector:u64,number:u64)->BTreeMap<String,u64>{
    let mut regs=BTreeMap::new();for (n,name)in ["rax","rdi","rsi","rdx","r10","r8","r9","rcx","r11","rbp","rbx","r12","r13","r14","r15","pkru","user_rsp"].iter().enumerate(){regs.insert(name.to_string(),0x1234_0000+n as u64);}
    regs.insert("rdi".into(),selector);regs.insert("rax".into(),number);regs
}
fn model(asm:&[&str],initial:&BTreeMap<String,u64>,handler:impl FnOnce(&mut SyscallRegs)->u64)->BTreeMap<String,u64>{
    let names=saved_names(asm);let mut frame:Vec<u64>=names.iter().map(|n|initial[n]).collect();
    let base=asm.iter().position(|&s|s=="mov rbp, rsp").unwrap();let call=asm.iter().position(|&s|s=="call syscall_handler").unwrap();let mut words=Vec::new();
    for instruction in &asm[base+1..call]{if instruction.starts_with("push "){let at=offset(instruction,"push qword ptr [rbp + ","]");assert_eq!(at%8,0);words.push(frame[at/8]);}}
    words.reverse();assert_eq!(words.len()*8,core::mem::size_of::<SyscallRegs>());
    // repr(C), nine u64 fields, verified against every actual build push below.
    let mut args=unsafe{core::ptr::read(words.as_ptr().cast::<SyscallRegs>())};let returned=handler(&mut args);
    let modified=unsafe{core::slice::from_raw_parts((&args as *const SyscallRegs).cast::<u64>(),words.len())};
    let discard=asm.iter().enumerate().skip(call+1).find(|(_,s)|s.starts_with("add rsp, ")).unwrap().0;
    let copies=&asm[call+1..discard];assert_eq!(copies.len()%2,0);
    for pair in copies.chunks_exact(2){let source=offset(pair[0],"mov rcx, [rsp + ","]");let destination=offset(pair[1],"mov [rbp + " ,"], rcx");assert_eq!((source%8,destination%8),(0,0));frame[destination/8]=modified[source/8];}
    assert_eq!(offset(asm[discard],"add rsp, ",""),core::mem::size_of::<SyscallRegs>());
    let mut registers=initial.clone();registers.insert("rax".into(),returned);let mut sp=0usize;
    for instruction in &asm[discard+1..]{
        if let Some(name)=instruction.strip_prefix("pop "){
            let value=frame[sp/8];sp+=8;registers.insert(name.to_string(),value);
        }else if let Some(name)=instruction.strip_prefix("push "){
            sp=sp.checked_sub(8).unwrap();frame[sp/8]=registers[name];
        }else if instruction.starts_with("add rsp, "){sp+=offset(instruction,"add rsp, ","");}
        else if *instruction=="mov rax, rdi"{registers.insert("rax".into(),registers["rdi"]);}
        else if *instruction=="xor ecx, ecx"{registers.insert("rcx".into(),0);}
        else if *instruction=="xor edx, edx"{registers.insert("rdx".into(),0);}
        else{assert!(instruction.starts_with("cmp byte ptr ")||matches!(*instruction,"je 93f"|"wrpkru"|"93:"|"swapgs"|"sysretq"),"unmodeled return instruction {instruction}");}
    }
    assert_eq!(sp,names.len()*8);registers
}
fn receive_outputs(regs:&mut SyscallRegs)->u64{regs.rdi=901;regs.rsi=3;regs.rdx=0x100_0000_0001;regs.r10=u64::MAX;regs.r8=0xfeed_abb0;0x201}

#[test]fn actual_repr_c_and_pushes_define_all_nine_struct_fields_and_saved_rdi_slot(){
    assert_eq!(core::mem::size_of::<SyscallRegs>(),72);assert_eq!(core::mem::align_of::<SyscallRegs>(),8);let names=saved_names(ENTRY_ASM);
    assert_eq!(names.iter().position(|s|s=="rdi").unwrap()*8,96);assert_eq!(field_offset("rdi"),8);
    let base=ENTRY_ASM.iter().position(|&s|s=="mov rbp, rsp").unwrap();let call=ENTRY_ASM.iter().position(|&s|s=="call syscall_handler").unwrap();let built:Vec<_>=ENTRY_ASM[base+1..call].iter().filter(|s|s.starts_with("push ")).map(|s|names[offset(s,"push qword ptr [rbp + ","]")/8].as_str()).rev().collect();
    for (n,name)in built.iter().enumerate(){assert_eq!(field_offset(name),n*8);}assert_eq!(built,["rax","rdi","rsi","rdx","r10","r8","r9","rcx","r11"]);
}
#[test]fn positive_receive_returns_actual_sender_task_and_all_four_payload_outputs(){
    for selector in [0,1,17]{let initial=inputs(selector,28);let returned=model(ENTRY_ASM,&initial,receive_outputs);for(name,value)in OUTPUTS.into_iter().zip([901,3,0x100_0000_0001,u64::MAX,0xfeed_abb0]){assert_eq!(returned[name],value,"output {name} retained old selector/input");}assert_eq!(returned["rax"],0x201);assert_ne!(returned["rdi"],selector);}
}
#[test]fn actual_invalid_listen_branch_clears_sender_task_and_payload_through_real_copy_section(){
    let initial=inputs(u64::MAX,28);let returned=model(ENTRY_ASM,&initial,actual_invalid_listen);for name in OUTPUTS{assert_eq!(returned[name],0,"invalid listen leaked {name}");}assert_eq!(returned["rax"],0);
}
#[test]fn unrelated_dispatch_preserves_original_rdi_selector_when_handler_leaves_it_unchanged(){
    for syscall in [0,1,29,63,64,65,68,69,71]{let initial=inputs(0x1234,syscall);let returned=model(ENTRY_ASM,&initial,|regs|{regs.rsi=0x7788;0x66});assert_eq!(returned["rdi"],0x1234);assert_eq!(returned["rsi"],0x7788);assert_eq!(returned["rax"],0x66);}
}
#[test]fn return_pop_layout_preserves_nonoutput_state_and_does_not_add_r9_ipc(){
    let initial=inputs(0,28);let returned=model(ENTRY_ASM,&initial,|regs|{let ret=receive_outputs(regs);regs.r9=0xffff;regs.rcx=0;regs.r11=0;ret});
    for name in ["r9","rcx","r11","rbp","rbx","r12","r13","r14","r15"]{assert_eq!(returned[name],initial[name],"saved {name}");}assert_eq!(returned["rsp"],initial["user_rsp"]);
}
#[test]fn missing_rdi_copy_reproduces_old_selector_even_while_all_other_outputs_survive(){
    let mut old=ENTRY_ASM.to_vec();let at=old.iter().position(|&s|s=="mov rcx, [rsp + 8]").unwrap();assert_eq!(old[at+1],"mov [rbp + 96], rcx");old.drain(at..at+2);let returned=model(&old,&inputs(0,28),receive_outputs);
    assert_eq!(returned["rdi"],0);assert_ne!(returned["rdi"],901);assert_eq!(returned["rsi"],3);assert_eq!(returned["rdx"],0x100_0000_0001);assert_eq!(returned["r10"],u64::MAX);assert_eq!(returned["r8"],0xfeed_abb0);
}
