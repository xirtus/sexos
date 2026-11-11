// SPDX-License-Identifier: GPL-3.0-only
//! Experimental QuickJS bytecode baseline JIT. Only pure, bounded
//! int32/boolean functions with forward-only control flow are compiled. The caller retains QuickJS's interrupt polling;
//! unknown operations, non-int arguments, overflow and multiplication by zero
//! bail out before any observable effect. Guest pointers never enter generated code.
use alloc::vec::Vec;
#[path = "jit_opcodes.rs"]
#[allow(dead_code)]
mod op;
#[path = "jit_flow.rs"]
mod flow;
use flow::{Kind, Op};

const HOT: u16 = 32;
const CACHE: usize = 64;
const MAX_BYTECODE: usize = 128;
const STACK: usize = 16;

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub compiled: u64,
    pub executed: u64,
    pub bailouts: u64,
    pub refused: u64,
}
/// PRIVATE diagnostic copy of existing cache state; not a function identity.
pub struct DiagnosticCandidate {
    pub bytecode: Vec<u8>,
    pub argc: usize,
    pub calls: u16,
    pub compiled: bool,
}

pub(crate) struct Jit {
    pub enabled: bool,
    pub stats: Stats,
    entries: Vec<Entry>,
}
struct Entry {
    bytecode: Vec<u8>,
    argc: usize,
    calls: u16,
    code: Option<Executable>,
}
impl Default for Jit {
    fn default() -> Self {
        Self {
            enabled: true,
            stats: Stats::default(),
            entries: Vec::new(),
        }
    }
}

impl Jit {
    pub fn diagnostic_candidates(&self) -> Vec<DiagnosticCandidate> {
        self.entries.iter().take(CACHE).map(|e| DiagnosticCandidate {
            bytecode: e.bytecode.clone(), argc: e.argc, calls: e.calls, compiled: e.code.is_some(),
        }).collect()
    }
    /// All offsets and JSValue tags are validated before compiling or calling code.
    /// The copied arguments are the only memory generated code may read.
    pub fn run(
        &mut self,
        data: &mut [u8],
        pc: u32,
        len: u32,
        argv: u32,
        argc: u32,
        out: u32,
    ) -> u32 {
        if !self.enabled || argc > 8 || len == 0 || len as usize > MAX_BYTECODE {
            return 0;
        }
        let argc = argc as usize;
        let Some(bc) = data.get(pc as usize..pc as usize + len as usize) else {
            return 0;
        };
        let Some(args) = data.get(argv as usize..argv as usize + argc * 8) else {
            return 0;
        };
        let mut copied = [0i32; 8];
        for (i, a) in args.chunks_exact(8).enumerate() {
            // QuickJS wasm32 uses NAN boxing: int32 has tag 0 in the upper word.
            if a[4..] != [0, 0, 0, 0] {
                return 0;
            }
            copied[i] = i32::from_le_bytes(a[..4].try_into().unwrap());
        }
        if data.get(out as usize..out as usize + 4).is_none() {
            return 0;
        }
        let index = match self
            .entries
            .iter()
            .position(|e| e.argc == argc && e.bytecode == bc)
        {
            Some(i) => i,
            None => {
                if self.entries.len() == CACHE {
                    return 0;
                }
                self.entries.push(Entry {
                    bytecode: bc.to_vec(),
                    argc,
                    calls: 0,
                    code: None,
                });
                self.entries.len() - 1
            }
        };
        let e = &mut self.entries[index];
        e.calls = e.calls.saturating_add(1);
        if e.calls == HOT {
            if let Some(bytes) = compile(&e.bytecode, argc) {
                e.code = Executable::new(&bytes);
                if e.code.is_some() {
                    self.stats.compiled += 1;
                } else {
                    self.stats.refused += 1;
                }
            }
        }
        let Some(code) = &e.code else { return 0 };
        let mut result = 0i32;
        let kind = code.call(&copied, &mut result);
        if !matches!(kind, 1 | 2) {
            self.stats.bailouts += 1;
            return 0;
        }
        data[out as usize..out as usize + 4].copy_from_slice(&result.to_le_bytes());
        self.stats.executed += 1;
        kind
    }
}

/// Compile the entire verified acyclic function; every reachable path returns.
/// Branches target verified forward instruction boundaries. No general pointers or indirect calls.
fn compile(bytecode: &[u8], argc: usize) -> Option<Vec<u8>> {
    if argc > 8 || bytecode.is_empty() || bytecode.len() > MAX_BYTECODE {
        return None;
    }
    let plan = flow::validate(bytecode, argc)?;
    for padding in 0..32 {
        let bytes = emit(&plan, padding)?;
        if !blocked(&bytes) {
            return Some(bytes);
        }
    }
    None
}

fn emit(plan: &flow::Plan, padding: usize) -> Option<Vec<u8>> {
    let mut code = Vec::from([0x48, 0x83, 0xec, 0x40]); // 16 copied int32/boolean slots
    let mut bailouts = Vec::new();
    let mut branches = Vec::new();
    let mut addresses = [usize::MAX; MAX_BYTECODE + 1];
    for (instruction, state) in plan.instructions.iter().zip(&plan.states) {
        addresses[instruction.at] = code.len();
        let depth = state.depth;
        match instruction.op {
            Op::Constant(n, _) => {
                // Re-encode immediate bytes that would contain a forbidden sequence.
                let mut literal = [0xb8, 0, 0, 0, 0];
                literal[1..].copy_from_slice(&n.to_le_bytes());
                if !blocked(&literal) { code.extend_from_slice(&literal); }
                else {
                    code.extend_from_slice(&[0x31, 0xc0]);
                    for b in n.to_be_bytes() { code.extend_from_slice(&[0xc1, 0xe0, 0x08, 0xb0, b]); }
                }
                store(&mut code, depth);
            }
            Op::Argument(arg) => {
                code.extend_from_slice(&[0x8b, 0x47, (arg * 4) as u8]);
                store(&mut code, depth);
            }
            Op::Binary(opcode) => {
                load(&mut code, depth - 2);
                code.extend_from_slice(&[0x8b, 0x4c, 0x24, ((depth - 1) * 4) as u8]);
                match opcode {
                    op::ADD => code.extend_from_slice(&[0x01, 0xc8]),
                    op::SUB => code.extend_from_slice(&[0x29, 0xc8]),
                    op::MUL => code.extend_from_slice(&[0x0f, 0xaf, 0xc1]),
                    op::AND => code.extend_from_slice(&[0x21, 0xc8]),
                    op::XOR => code.extend_from_slice(&[0x31, 0xc8]),
                    op::OR => code.extend_from_slice(&[0x09, 0xc8]),
                    _ => {
                        if matches!(opcode, op::STRICT_EQ | op::STRICT_NEQ)
                            && state.kinds[depth - 2] != state.kinds[depth - 1] {
                            code.extend_from_slice(&[0xb8, (opcode == op::STRICT_NEQ) as u8, 0, 0, 0]);
                        } else {
                            code.extend_from_slice(&[0x39, 0xc8]); // signed cmp eax,ecx
                            let condition = match opcode {
                                op::LT => 0x9c, op::LTE => 0x9e, op::GT => 0x9f, op::GTE => 0x9d,
                                op::EQ | op::STRICT_EQ => 0x94, op::NEQ | op::STRICT_NEQ => 0x95,
                                _ => return None,
                            };
                            code.extend_from_slice(&[0x0f, condition, 0xc0, 0x0f, 0xb6, 0xc0]);
                        }
                    }
                }
                if matches!(opcode, op::ADD | op::SUB | op::MUL) {
                    jump(&mut code, &mut bailouts, 0x80); // overflow falls back before output
                }
                if opcode == op::MUL {
                    code.extend_from_slice(&[0x85, 0xc0]);
                    jump(&mut code, &mut bailouts, 0x84); // preserve negative-zero semantics
                }
                store(&mut code, depth - 2);
            }
            Op::Not => {
                load(&mut code, depth - 1);
                code.extend_from_slice(&[0x85, 0xc0, 0x0f, 0x94, 0xc0, 0x0f, 0xb6, 0xc0]);
                store(&mut code, depth - 1);
            }
            Op::Dup => { load(&mut code, depth - 1); store(&mut code, depth); }
            Op::Drop => {}
            Op::If { truthy, target } => {
                load(&mut code, depth - 1); code.extend_from_slice(&[0x85, 0xc0]);
                code.extend_from_slice(&[0x0f, if truthy {0x85} else {0x84}]);
                branches.push((code.len(), target)); code.extend_from_slice(&[0; 4]);
            }
            Op::Jump(target) => {
                code.push(0xe9); branches.push((code.len(), target)); code.extend_from_slice(&[0; 4]);
            }
            Op::Return => {
                let kind = if state.kinds[0] == Kind::Bool {2} else {1};
                code.extend_from_slice(&[0x8b, 0x04, 0x24, 0x89, 0x06, 0xb8, kind, 0, 0, 0]);
                finish(&mut code);
            }
        }
    }
    code.extend(core::iter::repeat_n(0x90, padding));
    let target = code.len(); code.extend_from_slice(&[0x31, 0xc0]); finish(&mut code);
    for at in bailouts { branches.push((at, MAX_BYTECODE)); }
    // MAX_BYTECODE cannot be an admitted target (bytecode indices end at 127).
    addresses[MAX_BYTECODE] = target;
    for (at, byte_target) in branches {
        let native_target = *addresses.get(byte_target)?;
        let displacement = i32::try_from(native_target.checked_sub(at.checked_add(4)?)?).ok()?;
        code[at..at + 4].copy_from_slice(&displacement.to_le_bytes());
    }
    Some(code)
}
fn load(code: &mut Vec<u8>, slot: usize) {
    code.extend_from_slice(&[0x8b, 0x44, 0x24, (slot * 4) as u8]);
}

fn store(code: &mut Vec<u8>, slot: usize) {
    code.extend_from_slice(&[0x89, 0x44, 0x24, (slot * 4) as u8]);
}
fn finish(code: &mut Vec<u8>) {
    code.extend_from_slice(&[0x48, 0x83, 0xc4, 0x40, 0xc3]);
}
fn jump(code: &mut Vec<u8>, list: &mut Vec<usize>, condition: u8) {
    code.extend_from_slice(&[0x0f, condition]);
    list.push(code.len());
    code.extend_from_slice(&[0; 4]);
}
fn blocked(code: &[u8]) -> bool {
    code.windows(3).any(|b| {
        b[0] == core::hint::black_box(0x0f)
            && b[1] == core::hint::black_box(0x01)
            && b[2] == core::hint::black_box(0xef)
            || b[0] == 0x0f
                && ((b[1] == 0xae && b[2] & 0x38 == 0x28) || (b[1] == 0xc7 && b[2] & 0x38 == 0x18))
    })
}

// Code belongs to this instance's owning program/key; no global executable cache.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
struct Executable(sex_app::code::CodeBuf);
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
impl Executable {
    fn new(bytes: &[u8]) -> Option<Self> {
        let mut code = sex_app::code::CodeBuf::new(bytes.len()).ok()?;
        code.load(bytes).ok()?;
        Some(Self(code))
    }
    fn call(&self, args: &[i32; 8], result: &mut i32) -> u32 {
        // SAFETY: the complete emitter is restricted to bounded arithmetic/boolean forward control flow and
        // checked argument slots; the only store targets this live result.
        unsafe {
            self.0
                .entry::<unsafe extern "C" fn(*const i32, *mut i32) -> i32>(0)(
                args.as_ptr(),
                result,
            ) as u32
        }
    }
}

// Host x86 Linux backend for differential correctness/performance gates. Mappings
// transition RW -> RX, never RWX, and are unmapped with the instance.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
struct Executable(*mut core::ffi::c_void);
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
impl Executable {
    fn new(bytes: &[u8]) -> Option<Self> {
        unsafe extern "C" {
            fn mmap(
                p: *mut core::ffi::c_void,
                n: usize,
                prot: i32,
                flags: i32,
                fd: i32,
                o: isize,
            ) -> *mut core::ffi::c_void;
            fn mprotect(p: *mut core::ffi::c_void, n: usize, prot: i32) -> i32;
            fn munmap(p: *mut core::ffi::c_void, n: usize) -> i32;
        }
        if bytes.len() > 4096 || blocked(bytes) {
            return None;
        }
        unsafe {
            let p = mmap(core::ptr::null_mut(), 4096, 3, 0x22, -1, 0);
            if p as isize == -1 {
                return None;
            }
            core::ptr::write_bytes(p as *mut u8, 0xcc, 4096);
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
            if mprotect(p, 4096, 5) != 0 {
                munmap(p, 4096);
                return None;
            }
            Some(Self(p))
        }
    }
    fn call(&self, args: &[i32; 8], result: &mut i32) -> u32 {
        unsafe {
            let f: unsafe extern "C" fn(*const i32, *mut i32) -> i32 = core::mem::transmute(self.0);
            f(args.as_ptr(), result) as u32
        }
    }
}
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
impl Drop for Executable {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn munmap(p: *mut core::ffi::c_void, n: usize) -> i32;
        }
        unsafe {
            munmap(self.0, 4096);
        }
    }
}
#[cfg(not(all(target_arch = "x86_64", any(target_os = "none", target_os = "linux"))))]
struct Executable;
#[cfg(not(all(target_arch = "x86_64", any(target_os = "none", target_os = "linux"))))]
impl Executable {
    fn new(_: &[u8]) -> Option<Self> {
        None
    }
    fn call(&self, _: &[i32; 8], _: &mut i32) -> u32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_incomplete_functions_and_memory_operations() {
        for bc in [
            &[op::RETURN][..],
            &[op::GET_ARG0, op::RETURN],
            &[op::PUSH_0, op::RETURN, op::PUSH_0],
            &[op::PUSH_I32, 1],
            &[op::PUSH_0, 0, op::RETURN],
        ] {
            assert!(compile(bc, 0).is_none());
        }
        assert!(compile(&[op::GET_ARG0, op::RETURN], 1).is_some());
    }
    fn boolean_flags() -> Vec<u8> {
        // Independently authored: !!(x & 1) && !!(x & 2) || !!(x & 4).
        alloc::vec![op::GET_ARG0,op::PUSH_1,op::AND,op::LNOT,op::LNOT,op::DUP,
            op::IF_FALSE8,7,op::DROP,op::GET_ARG0,op::PUSH_2,op::AND,op::LNOT,op::LNOT,
            op::DUP,op::IF_TRUE8,7,op::DROP,op::GET_ARG0,op::PUSH_4,op::AND,op::LNOT,op::LNOT,op::RETURN]
    }
    #[test]
    fn forward_control_flow_requires_valid_typed_joins() {
        assert!(compile(&boolean_flags(),1).is_some());
        for bc in [
            alloc::vec![op::GET_ARG0,op::IF_TRUE8,255,op::PUSH_1,op::RETURN], // back edge
            alloc::vec![op::PUSH_1,op::IF_TRUE8,2,op::PUSH_I32,0,0,0,0,op::RETURN], // operand target
            alloc::vec![op::PUSH_0,op::IF_TRUE8,2,op::PUSH_0,op::RETURN], // unequal heights
            alloc::vec![op::PUSH_0,op::IF_TRUE8,4,op::PUSH_1,op::GOTO8,2,op::PUSH_FALSE,op::RETURN], // int/bool join
            alloc::vec![op::PUSH_0,op::IF_TRUE,255,255,255,127,op::PUSH_0,op::RETURN], // overflowing/outside target
            alloc::vec![op::PUSH_0,op::IF_TRUE8,1,op::PUSH_0], // path falls out
            alloc::vec![op::DUP,op::RETURN],
        ] { assert!(compile(&bc,1).is_none(),"accepted {bc:?}"); }
        let mut too_deep=alloc::vec![op::PUSH_0;STACK+1];too_deep.push(op::RETURN);
        assert!(compile(&too_deep,0).is_none());
    }
    #[test]
    fn noninteger_arguments_fall_back_before_any_output() {
        let mut jit=Jit::default();let mut data=[0x55u8;64];
        data[0..2].copy_from_slice(&[op::GET_ARG0,op::RETURN]);
        // QuickJS bool, null, object, double NaN and negative-zero encodings are
        // not int32 arguments; the interpreter retains their exact semantics.
        for upper in [1u32,2,0xffff_ffff,0x7ff8_0000,0x8000_0000] {
            data[16..20].copy_from_slice(&0u32.to_le_bytes());data[20..24].copy_from_slice(&upper.to_le_bytes());
            let before=data;assert_eq!(jit.run(&mut data,0,2,16,1,40),0);assert_eq!(data,before);
        }
        assert_eq!(jit.stats.compiled,0);assert_eq!(jit.stats.executed,0);
    }
    #[cfg(all(target_arch="x86_64",target_os="linux"))]
    #[test]
    fn actual_forward_branch_code_preserves_boolean_types_and_short_circuit_values() {
        let code=Executable::new(&compile(&boolean_flags(),1).unwrap()).unwrap();
        for x in (-512..512).chain([i32::MIN,i32::MAX]) {
            let mut out=-1;let kind=code.call(&[x,0,0,0,0,0,0,0],&mut out);
            assert_eq!((kind,out),(2,(((x&1)!=0&&(x&2)!=0)||(x&4)!=0)as i32));
        }
        // x && 7 returns an int32 zero or seven, not a coerced boolean.
        let bc=[op::GET_ARG0,op::DUP,op::IF_FALSE8,3,op::DROP,op::PUSH_7,op::RETURN];
        let code=Executable::new(&compile(&bc,1).unwrap()).unwrap();
        for x in [0,1,-1,i32::MIN,i32::MAX] {
            let mut out=-1;assert_eq!(code.call(&[x,0,0,0,0,0,0,0],&mut out),1);
            assert_eq!(out,if x==0 {0}else{7});
        }
        for (op,want) in [(op::STRICT_EQ,0),(op::STRICT_NEQ,1),(op::EQ,1),(op::NEQ,0)] {
            let code=Executable::new(&compile(&[op::PUSH_TRUE,op::PUSH_1,op,op::RETURN],0).unwrap()).unwrap();
            let mut out=-1;assert_eq!(code.call(&[0;8],&mut out),2);assert_eq!(out,want);
        }
        // Separate returns can carry different kinds without a mixed stack join.
        let code=Executable::new(&compile(&[op::GET_ARG0,op::IF_FALSE8,3,op::PUSH_TRUE,op::RETURN,op::PUSH_7,op::RETURN],1).unwrap()).unwrap();
        for (x,want) in [(0,(1,7)),(1,(2,1))] {
            let mut out=-1;assert_eq!((code.call(&[x,0,0,0,0,0,0,0],&mut out),out),want);
        }
    }
    #[test]
    fn constants_never_embed_key_changing_bytes() {
        for n in [0x00ef010f, 0x0028ae0f, 0x0018c70f, i32::MIN, i32::MAX] {
            let mut bc = alloc::vec![op::PUSH_I32];
            bc.extend(n.to_le_bytes());
            bc.push(op::RETURN);
            assert!(!blocked(&compile(&bc, 0).unwrap()));
        }
    }
    #[test]
    fn malformed_linear_memory_falls_back_without_writing() {
        let mut jit = Jit::default();
        let mut data = [0u8; 32];
        assert_eq!(jit.run(&mut data, u32::MAX, 1, 0, 0, 0), 0);
        assert_eq!(jit.run(&mut data, 0, 129, 0, 0, 0), 0);
        assert_eq!(jit.run(&mut data, 0, 1, u32::MAX, 8, 0), 0);
        assert_eq!(jit.run(&mut data, 0, 1, 0, 0, u32::MAX), 0);
        assert_eq!(data, [0; 32]);
    }
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    #[test]
    fn emitted_code_obeys_int32_and_overflow_contract() {
        for opcode in [op::ADD, op::SUB, op::MUL, op::AND, op::XOR, op::OR] {
            let c = Executable::new(
                &compile(&[op::GET_ARG0, op::GET_ARG1, opcode, op::RETURN], 2).unwrap(),
            )
            .unwrap();
            for a in [i32::MIN, -65536, -1, 0, 1, 65536, i32::MAX] {
                for b in [i32::MIN, -65536, -1, 0, 1, 65536, i32::MAX] {
                    let expected = match opcode {
                        op::ADD => a.checked_add(b),
                        op::SUB => a.checked_sub(b),
                        op::MUL => a.checked_mul(b).filter(|x| *x != 0),
                        op::AND => Some(a & b),
                        op::XOR => Some(a ^ b),
                        op::OR => Some(a | b),
                        _ => unreachable!(),
                    };
                    let mut out = 123;
                    let ok = c.call(&[a, b, 0, 0, 0, 0, 0, 0], &mut out);
                    assert_eq!(ok, u32::from(expected.is_some()));
                    if let Some(want) = expected {
                        assert_eq!(out, want);
                    } else {
                        assert_eq!(out, 123);
                    }
                }
            }
        }
    }
}
