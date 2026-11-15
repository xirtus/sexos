// SPDX-License-Identifier: GPL-3.0-only
//! Bounded native whole-helper paths. QuickJS owns all allocation/references.
//! No guest address is executable: checked offsets become host pointers only
//! for this synchronous call, with no guest callbacks or memory growth.
use super::{Executable, HOT};

#[derive(Default)]
pub(super) struct Heap {
    calls: [u16; 3],
    code: [Option<Executable>; 3],
}
impl Heap {
    pub(super) fn run(&mut self, data: &mut [u8], kind: u32, descriptor: u32, output: u32) -> (u32, bool) {
        let k = kind as usize;
        if k >= 3 { return (0, false); }
        let Some(end) = (descriptor as usize).checked_add(24) else { return (0,false); };
        let Some(bytes) = data.get(descriptor as usize..end) else { return (0, false); };
        let d: [usize; 6] = core::array::from_fn(|i| u32::from_le_bytes(bytes[i*4..i*4+4].try_into().unwrap()) as usize);
        let valid = |at: usize, len: usize| at.checked_add(len).is_some_and(|end| end <= data.len());
        let mut frame = [0usize; 8];
        let out_len;
        if k == 0 {
            if !valid(d[0],4) || !valid(d[1],4) || !valid(d[2],8) { return (0,false); }
            out_len = 8;
            for i in 0..3 { frame[i] = data.as_ptr().wrapping_add(d[i]) as usize; }
            frame[3..6].copy_from_slice(&d[3..6]);
        } else {
            if d[1] > 256 || d[4] > 256 || d[2] > 1 || d[5] > 1 { return (0,false); }
            let (a,b) = (d[1] * (d[2]+1), d[4] * (d[5]+1));
            if !valid(d[0], a) || !valid(d[3], b) { return (0,false); }
            out_len = if k == 1 { 4 } else {
                if d[1]+d[4] > 256 { return (0,false); }
                (d[1]+d[4])*2
            };
            // Concatenation cannot overwrite an input still being read.
            if k == 2 {
                let o = output as usize;
                for (at,len) in [(d[0],a),(d[3],b)] {
                    if o < at+len && at < o.saturating_add(out_len) { return (0,false); }
                }
            }
            frame[..6].copy_from_slice(&[data.as_ptr().wrapping_add(d[0]) as usize,
                data.as_ptr().wrapping_add(d[3]) as usize,d[1],d[4],d[2]+1,d[5]+1]);
        }
        if !valid(output as usize,out_len) { return (0,false); }
        self.calls[k] = self.calls[k].saturating_add(1);
        let mut compiled = false;
        if self.calls[k] == HOT {
            let bytes: &[u8] = match k {
                0 => include_bytes!("../guest/jit/own.bin"),
                1 => include_bytes!("../guest/jit/compare.bin"),
                _ => include_bytes!("../guest/jit/concat.bin"),
            };
            if !super::blocked(bytes) { self.code[k] = Executable::new(bytes); }
            compiled = self.code[k].is_some();
        }
        let Some(code) = &self.code[k] else { return (0,compiled); };
        // All frame pointees and output ranges checked above. Templates only
        // read these ranges; string loops consume at most 256 code units.
        let result = unsafe { code.call_heap(&frame, data.as_mut_ptr().add(output as usize)) };
        (result,compiled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor(data: &mut [u8], d: [u32;6]) {
        for (i,v) in d.iter().enumerate() { data[i*4..i*4+4].copy_from_slice(&v.to_le_bytes()); }
    }
    #[test]
    fn invalid_ranges_widths_and_overlap_never_write_or_compile() {
        let mut h=Heap::default(); let mut data=[0x55;128];
        for (kind,d,out) in [
            (0,[126,40,48,1,2,0],80), (0,[32,127,48,1,2,0],80),
            (0,[32,40,124,1,2,0],80), (0,[32,40,48,1,2,0],124),
            (1,[32,1,2,40,1,0],80), (1,[32,257,0,40,1,0],80),
            (1,[127,1,1,40,1,0],80), (2,[32,3,0,40,3,0],33),
            (2,[32,3,0,40,3,0],39), (2,[32,3,0,40,3,0],120),
        ] {
            descriptor(&mut data,d);let before=data;
            for _ in 0..40 { assert_eq!(h.run(&mut data,kind,0,out),(0,false)); }
            assert_eq!(data,before);
        }
        assert_eq!(h.calls,[0;3]);
        assert_eq!(h.run(&mut data,3,0,80),(0,false));
        assert_eq!(h.run(&mut data,0,u32::MAX,80),(0,false));
    }
    #[cfg(all(target_arch="x86_64",any(target_os="linux",target_os="none")))]
    #[test]
    fn native_property_guards_before_output_and_reads_fresh_value() {
        let mut h=Heap::default();let mut data=[0x55;128];
        descriptor(&mut data,[32,40,48,123,456,0]);
        data[32..36].copy_from_slice(&123u32.to_le_bytes());
        data[40..44].copy_from_slice(&456u32.to_le_bytes());
        for _ in 0..HOT { h.run(&mut data,0,0,80); }
        for value in [0u64,1,0x1234_5678_9abc_def0,u64::MAX] {
            data[48..56].copy_from_slice(&value.to_le_bytes());
            assert_eq!(h.run(&mut data,0,0,80).0,1);
            assert_eq!(&data[80..88],&value.to_le_bytes());
        }
        for (at,value) in [(32,124u32),(40,457),(20,16)] {
            let before=data;data[at..at+4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(h.run(&mut data,0,0,80).0,0);
            assert_eq!(&data[80..88],&before[80..88]);data=before;
        }
    }
}
