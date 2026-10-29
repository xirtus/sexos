//! Immutable BOOT display metadata. No mappings, frames, or capability creation.
#[derive(Clone, Copy)]
pub struct Info {
    pub display: u32,
    pub shell: u32,
    pub input: u32,
    pub address: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}
impl Info {
    pub fn query(self, caller: u32, kind: u64, reserved: [u64; 2]) -> Option<u64> {
        if reserved != [0; 2]
            || caller == 0
            || ![self.display, self.shell, self.input].contains(&caller)
        {
            return None;
        }
        match kind {
            0 => Some(self.width as u64 | ((self.height as u64) << 32)),
            1 if caller == self.display => Some(self.address),
            2 => Some(self.pitch as u64),
            _ => None,
        }
    }
}
