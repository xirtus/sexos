//! JPEG XL through jxl-oxide (MIT OR Apache-2.0). jxl-oxide needs std (it
//! reads through `std::io` and uses std collections), so this module is
//! behind the `jxl` feature, which turns `std` on.

use alloc::vec::Vec;

use jxl_oxide::{JxlImage, PixelFormat};

use crate::{admit, Error};
use sex_image::{Image, Limits};

/// The size of a JPEG XL picture as shown (its orientation already applied).
pub fn jxl_dimensions(bytes: &[u8]) -> Result<(usize, usize), Error> {
    let image = open(bytes)?;
    Ok((image.width() as usize, image.height() as usize))
}

fn open(bytes: &[u8]) -> Result<JxlImage, Error> {
    JxlImage::builder().read(std::io::Cursor::new(bytes)).map_err(|_| Error::Jxl)
}

/// Decodes the first frame of a JPEG XL file (bare codestream or container)
/// into sex-image's pixel layout. The orientation in the JPEG XL header is
/// applied by the decoder; do not apply EXIF orientation again.
pub fn decode_jxl(bytes: &[u8], limits: &Limits) -> Result<Image, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let image = open(bytes)?;
    let (w, h) = (image.width() as usize, image.height() as usize);
    let pixels = admit(limits, w, h)?;
    let format = image.pixel_format();
    if matches!(format, PixelFormat::Cmyk | PixelFormat::Cmyka) {
        return Err(Error::Unsupported);
    }
    let render = image.render_frame(0).map_err(|_| Error::Jxl)?;
    let mut stream = render.stream();
    let channels = stream.channels() as usize;
    if stream.width() as usize != w || stream.height() as usize != h || !(1..=4).contains(&channels) {
        return Err(Error::Jxl);
    }
    let mut samples = Vec::new();
    samples.try_reserve_exact(pixels * channels).map_err(|_| Error::OutOfMemory)?;
    samples.resize(pixels * channels, 0u8);
    stream.write_to_buffer(&mut samples);
    let mut out = Vec::new();
    out.try_reserve_exact(pixels).map_err(|_| Error::OutOfMemory)?;
    out.extend(samples.chunks_exact(channels).map(|s| {
        let (r, g, b, a) = match channels {
            1 => (s[0], s[0], s[0], 255),
            2 => (s[0], s[0], s[0], s[1]),
            3 => (s[0], s[1], s[2], 255),
            _ => (s[0], s[1], s[2], s[3]),
        };
        u32::from_be_bytes([a, r, g, b])
    }));
    Ok(Image { width: w, height: h, pixels: out })
}
