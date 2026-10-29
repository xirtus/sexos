//! Read immutable BOOT display metadata. Dimensions grant no framebuffer access.
pub const OP: u64 = 0xE6;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}
pub fn geometry() -> Result<Geometry, u64> {
    let v = crate::pdx_call_checked(0, OP, 0, 0, 0)?;
    let pitch = crate::pdx_call_checked(0, OP, 2, 0, 0)?;
    let g = Geometry {
        width: v as u32,
        height: (v >> 32) as u32,
        pitch: pitch as u32,
    };
    if g.width == 0 || g.height == 0 || g.width > 8192 || g.height > 8192 || pitch > u32::MAX as u64
    {
        return Err(crate::ERR_CAP_INVALID);
    }
    Ok(g)
}
/// Only the exact BOOT display subject can resolve its existing framebuffer.
pub fn framebuffer() -> Result<u64, u64> {
    crate::pdx_call_checked(0, OP, 1, 0, 0)
}
