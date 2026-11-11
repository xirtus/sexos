// SPDX-License-Identifier: GPL-3.0-only
//! Typed, acyclic bytecode admission. No code is emitted until every instruction,
//! edge and stack join is checked. Unsupported/dead instructions fail closed.
use alloc::vec::Vec;
use super::{op, MAX_BYTECODE, STACK};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind { Int, Bool }
#[derive(Clone, Copy, Debug)]
pub(super) enum Op {
    Constant(i32, Kind), Argument(usize), Binary(u8), Not, Dup, Drop,
    If { truthy: bool, target: usize }, Jump(usize), Return,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Instruction { pub at: usize, pub op: Op }
#[derive(Clone, Copy, Debug)]
pub(super) struct State { pub depth: usize, pub kinds: [Kind; STACK] }
impl State {
    fn empty() -> Self { Self { depth: 0, kinds: [Kind::Int; STACK] } }
    fn push(&mut self, kind: Kind) -> Option<()> {
        if self.depth == STACK { return None; }
        self.kinds[self.depth] = kind; self.depth += 1; Some(())
    }
    fn pop(&mut self) -> Option<Kind> {
        self.depth = self.depth.checked_sub(1)?; Some(self.kinds[self.depth])
    }
    fn agrees(self, other: Self) -> bool {
        self.depth == other.depth && self.kinds[..self.depth] == other.kinds[..other.depth]
    }
}
pub(super) struct Plan { pub instructions: Vec<Instruction>, pub states: Vec<State> }
fn edge(states: &mut [Option<State>], index: usize, state: State) -> Option<()> {
    let slot = states.get_mut(index)?;
    if let Some(previous) = slot { if !previous.agrees(state) { return None; } }
    else { *slot = Some(state); }
    Some(())
}
fn branch(at: usize, next: usize, delta: i32, len: usize) -> Option<usize> {
    // QuickJS offsets begin at the first operand byte, not at the next opcode.
    let target = at.checked_add(1)?.checked_add_signed(delta as isize)?;
    (target >= next && target < len).then_some(target)
}
pub(super) fn validate(bc: &[u8], argc: usize) -> Option<Plan> {
    if argc > 8 || bc.is_empty() || bc.len() > MAX_BYTECODE { return None; }
    let mut instructions = Vec::new();
    let mut boundary = [usize::MAX; MAX_BYTECODE + 1];
    let mut pc = 0;
    while pc < bc.len() {
        let at = pc; let code = bc[pc]; pc += 1;
        let op = match code {
            op::PUSH_MINUS1..=op::PUSH_7 => Op::Constant(code as i32 - op::PUSH_0 as i32, Kind::Int),
            op::PUSH_FALSE | op::PUSH_TRUE => Op::Constant((code == op::PUSH_TRUE) as i32, Kind::Bool),
            op::PUSH_I8 => { let n = *bc.get(pc)? as i8 as i32; pc += 1; Op::Constant(n, Kind::Int) },
            op::PUSH_I16 => { let n = i16::from_le_bytes(bc.get(pc..pc+2)?.try_into().ok()?) as i32; pc += 2; Op::Constant(n, Kind::Int) },
            op::PUSH_I32 => { let n = i32::from_le_bytes(bc.get(pc..pc+4)?.try_into().ok()?); pc += 4; Op::Constant(n, Kind::Int) },
            op::GET_ARG => { let n = u16::from_le_bytes(bc.get(pc..pc+2)?.try_into().ok()?) as usize; pc += 2; Op::Argument(n) },
            op::GET_ARG0..=op::GET_ARG3 => Op::Argument((code-op::GET_ARG0) as usize),
            op::ADD | op::SUB | op::MUL | op::AND | op::XOR | op::OR |
            op::LT | op::LTE | op::GT | op::GTE | op::EQ | op::NEQ | op::STRICT_EQ | op::STRICT_NEQ => Op::Binary(code),
            op::LNOT => Op::Not, op::DUP => Op::Dup, op::DROP => Op::Drop,
            op::IF_FALSE8 | op::IF_TRUE8 | op::GOTO8 => {
                let delta = *bc.get(pc)? as i8 as i32; pc += 1;
                let target = branch(at, pc, delta, bc.len())?;
                if code == op::GOTO8 { Op::Jump(target) } else { Op::If { truthy: code == op::IF_TRUE8, target } }
            },
            op::GOTO16 => { let delta = i16::from_le_bytes(bc.get(pc..pc+2)?.try_into().ok()?) as i32; pc += 2; Op::Jump(branch(at,pc,delta,bc.len())?) },
            op::IF_FALSE | op::IF_TRUE | op::GOTO => {
                let delta = i32::from_le_bytes(bc.get(pc..pc+4)?.try_into().ok()?); pc += 4;
                let target = branch(at, pc, delta, bc.len())?;
                if code == op::GOTO { Op::Jump(target) } else { Op::If { truthy: code == op::IF_TRUE, target } }
            },
            op::RETURN => Op::Return,
            _ => return None,
        };
        boundary[at] = instructions.len(); instructions.push(Instruction { at, op });
    }
    let mut incoming = alloc::vec![None; instructions.len()]; incoming[0] = Some(State::empty());
    let mut states = Vec::with_capacity(instructions.len());
    for (i, instruction) in instructions.iter().enumerate() {
        let mut state = incoming[i]?; states.push(state);
        match instruction.op {
            Op::Constant(_,kind) => state.push(kind)?,
            Op::Argument(arg) => { if arg >= argc { return None; } state.push(Kind::Int)?; },
            Op::Binary(code) => {
                state.pop()?; state.pop()?;
                let comparison = matches!(code,op::LT|op::LTE|op::GT|op::GTE|op::EQ|op::NEQ|op::STRICT_EQ|op::STRICT_NEQ);
                state.push(if comparison {Kind::Bool} else {Kind::Int})?;
            },
            Op::Not => { state.pop()?; state.push(Kind::Bool)?; },
            Op::Dup => { let kind = state.pop()?; state.push(kind)?; state.push(kind)?; },
            Op::Drop => { state.pop()?; },
            Op::If {target,..} => { state.pop()?; edge(&mut incoming,*boundary.get(target)?,state)?; },
            Op::Jump(target) => { edge(&mut incoming,*boundary.get(target)?,state)?; continue; },
            Op::Return => { if state.depth != 1 { return None; } continue; },
        }
        // A reachable path may not fall out of the function without returning.
        edge(&mut incoming,i+1,state)?;
    }
    Some(Plan { instructions, states })
}
