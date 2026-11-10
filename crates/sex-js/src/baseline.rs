// SPDX-License-Identifier: GPL-3.0-only
//! Experimental QuickJS bytecode baseline JIT. Only pure, bounded straight-line
//! int32 functions are compiled. The caller retains QuickJS's interrupt polling;
//! unknown operations, non-int arguments, overflow and multiplication by zero
//! bail out before any observable effect. Guest pointers never enter generated code.
use alloc::vec::Vec;
#[path = "jit_opcodes.rs"]
#[allow(dead_code)]
mod op;

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
    ) -> bool {
        if !self.enabled || argc > 8 || len == 0 || len as usize > MAX_BYTECODE {
            return false;
        }
        let argc = argc as usize;
        let Some(bc) = data.get(pc as usize..pc as usize + len as usize) else {
            return false;
        };
        let Some(args) = data.get(argv as usize..argv as usize + argc * 8) else {
            return false;
        };
        let mut copied = [0i32; 8];
        for (i, a) in args.chunks_exact(8).enumerate() {
            // QuickJS wasm32 uses NAN boxing: int32 has tag 0 in the upper word.
            if a[4..] != [0, 0, 0, 0] {
                return false;
            }
            copied[i] = i32::from_le_bytes(a[..4].try_into().unwrap());
        }
        if data.get(out as usize..out as usize + 4).is_none() {
            return false;
        }
        let index = match self
            .entries
            .iter()
            .position(|e| e.argc == argc && e.bytecode == bc)
        {
            Some(i) => i,
            None => {
                if self.entries.len() == CACHE {
                    return false;
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
        let Some(code) = &e.code else { return false };
        let mut result = 0i32;
        if !code.call(&copied, &mut result) {
            self.stats.bailouts += 1;
            return false;
        }
        data[out as usize..out as usize + 4].copy_from_slice(&result.to_le_bytes());
        self.stats.executed += 1;
        true
    }
}

/// Compile the *entire* function, including a final return with no trailing bytes.
/// The emitter has no general pointers, indirect calls or guest-controlled branches.
fn compile(bytecode: &[u8], argc: usize) -> Option<Vec<u8>> {
    if argc > 8 || bytecode.is_empty() || bytecode.len() > MAX_BYTECODE {
        return None;
    }
    for padding in 0..32 {
        let bytes = emit(bytecode, argc, padding)?;
        if !blocked(&bytes) {
            return Some(bytes);
        }
    }
    None
}

fn emit(bc: &[u8], argc: usize, padding: usize) -> Option<Vec<u8>> {
    let mut code = Vec::from([0x48, 0x83, 0xec, 0x40]); // sub rsp,64: 16 int32 slots
    let mut bailouts = Vec::new();
    let (mut pc, mut depth) = (0, 0usize);
    let mut returned = false;
    while pc < bc.len() {
        let opcode = bc[pc];
        pc += 1;
        let constant = match opcode {
            op::PUSH_MINUS1..=op::PUSH_7 => Some(opcode as i32 - op::PUSH_0 as i32),
            op::PUSH_I8 => {
                let n = *bc.get(pc)? as i8 as i32;
                pc += 1;
                Some(n)
            }
            op::PUSH_I16 => {
                let n = i16::from_le_bytes(bc.get(pc..pc + 2)?.try_into().ok()?) as i32;
                pc += 2;
                Some(n)
            }
            op::PUSH_I32 => {
                let n = i32::from_le_bytes(bc.get(pc..pc + 4)?.try_into().ok()?);
                pc += 4;
                Some(n)
            }
            _ => None,
        };
        if let Some(n) = constant {
            if depth == STACK {
                return None;
            }
            // Use a short immediate where safe; re-encode dangerous payloads byte
            // by byte. The complete function is scanned across boundaries too.
            let mut literal = [0xb8, 0, 0, 0, 0]; // mov eax,imm32
            literal[1..].copy_from_slice(&n.to_le_bytes());
            if !blocked(&literal) {
                code.extend_from_slice(&literal);
            } else {
                code.extend_from_slice(&[0x31, 0xc0]);
                for b in n.to_be_bytes() {
                    code.extend_from_slice(&[0xc1, 0xe0, 0x08, 0xb0, b]);
                }
            }
            store(&mut code, depth);
            depth += 1;
        } else {
            match opcode {
                op::GET_ARG | op::GET_ARG0..=op::GET_ARG3 => {
                    let arg = if opcode == op::GET_ARG {
                        let n = u16::from_le_bytes(bc.get(pc..pc + 2)?.try_into().ok()?) as usize;
                        pc += 2;
                        n
                    } else {
                        (opcode - op::GET_ARG0) as usize
                    };
                    if arg >= argc || depth == STACK {
                        return None;
                    }
                    code.extend_from_slice(&[0x8b, 0x47, (arg * 4) as u8]); // mov eax,[rdi+offset]
                    store(&mut code, depth);
                    depth += 1;
                }
                op::ADD | op::SUB | op::MUL | op::AND | op::XOR | op::OR => {
                    if depth < 2 {
                        return None;
                    }
                    code.extend_from_slice(&[0x8b, 0x44, 0x24, ((depth - 2) * 4) as u8]);
                    code.extend_from_slice(&[0x8b, 0x4c, 0x24, ((depth - 1) * 4) as u8]);
                    match opcode {
                        op::ADD => code.extend_from_slice(&[0x01, 0xc8]),
                        op::SUB => code.extend_from_slice(&[0x29, 0xc8]),
                        op::MUL => code.extend_from_slice(&[0x0f, 0xaf, 0xc1]),
                        op::AND => code.extend_from_slice(&[0x21, 0xc8]),
                        op::XOR => code.extend_from_slice(&[0x31, 0xc8]),
                        op::OR => code.extend_from_slice(&[0x09, 0xc8]),
                        _ => unreachable!(),
                    }
                    if matches!(opcode, op::ADD | op::SUB | op::MUL) {
                        jump(&mut code, &mut bailouts, 0x80); // jo fallback (preserve double semantics)
                    }
                    if opcode == op::MUL {
                        // Conservative: every zero product falls back, preserving -0.
                        code.extend_from_slice(&[0x85, 0xc0]);
                        jump(&mut code, &mut bailouts, 0x84);
                    }
                    depth -= 1;
                    store(&mut code, depth - 1);
                }
                op::RETURN => {
                    if depth != 1 || pc != bc.len() {
                        return None;
                    }
                    code.extend_from_slice(&[0x8b, 0x04, 0x24, 0x89, 0x06, 0xb8, 1, 0, 0, 0]);
                    finish(&mut code);
                    returned = true;
                }
                _ => return None,
            }
        }
    }
    if !returned {
        return None;
    }
    code.extend(core::iter::repeat_n(0x90, padding));
    let target = code.len();
    code.extend_from_slice(&[0x31, 0xc0]);
    finish(&mut code);
    for at in bailouts {
        let displacement = i32::try_from(target - (at + 4)).ok()?;
        code[at..at + 4].copy_from_slice(&displacement.to_le_bytes());
    }
    Some(code)
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
    fn call(&self, args: &[i32; 8], result: &mut i32) -> bool {
        // SAFETY: the complete emitter is restricted to bounded arithmetic and
        // checked argument slots; the only store targets this live result.
        unsafe {
            self.0
                .entry::<unsafe extern "C" fn(*const i32, *mut i32) -> i32>(0)(
                args.as_ptr(),
                result,
            ) != 0
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
    fn call(&self, args: &[i32; 8], result: &mut i32) -> bool {
        unsafe {
            let f: unsafe extern "C" fn(*const i32, *mut i32) -> i32 = core::mem::transmute(self.0);
            f(args.as_ptr(), result) != 0
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
    fn call(&self, _: &[i32; 8], _: &mut i32) -> bool {
        false
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
        assert!(!jit.run(&mut data, u32::MAX, 1, 0, 0, 0));
        assert!(!jit.run(&mut data, 0, 129, 0, 0, 0));
        assert!(!jit.run(&mut data, 0, 1, u32::MAX, 8, 0));
        assert!(!jit.run(&mut data, 0, 1, 0, 0, u32::MAX));
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
                    assert_eq!(ok, expected.is_some());
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
