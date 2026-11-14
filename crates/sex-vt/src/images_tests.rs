// SPDX-License-Identifier: GPL-3.0-only
//! Inline image tests: kitty graphics, sixel, placements that scroll, and drawing.
extern crate std;
use crate::images::*;
use crate::{Parser, Screen};
use alloc::format;
use alloc::vec::Vec;

// Made with Python's zlib, independent of this crate's encoder. 3x2 RGB, rows filtered
// Sub then Up: red, green, blue / white, black, (10,20,30).
const PNG_RGB_3X2: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x03\x00\x00\x00\x02\x08\x02\x00\x00\x00\x12\x16\xf1\x4d\x00\x00\x00\x19\x49\x44\x41\x54\x78\xda\x63\xfc\xcf\xc0\xc0\x08\xc6\x4c\x0c\xff\xff\x33\x30\x32\x70\x89\xc8\x03\x00\x3c\x86\x05\x3f\xef\x68\xa2\xfd\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";
// 2x2, 1-bit palette with tRNS: transparent, yellow / yellow, transparent.
const PNG_PAL_2X2: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x02\x00\x00\x00\x02\x01\x03\x00\x00\x00\x48\x78\x9f\x67\x00\x00\x00\x06\x50\x4c\x54\x45\xff\x00\xff\xff\xff\x00\xb2\x1a\xdd\x6d\x00\x00\x00\x01\x74\x52\x4e\x53\x00\x40\xe6\xd8\x66\x00\x00\x00\x0c\x49\x44\x41\x54\x78\x9c\x63\x70\x60\x68\x00\x00\x01\x44\x00\xc1\x3a\x7a\x1c\x56\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";

const RGBA_2X2: [u8; 16] = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128];

fn screen(cols: usize, rows: usize) -> (Screen, Parser) {
    let s = Screen::new(cols, rows);
    s.set_cell_pixels(10, 20);
    (s, Parser::new())
}
fn apc(control: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("\x1b_G{};", control).into_bytes();
    v.extend_from_slice(&base64_encode(data));
    v.extend_from_slice(b"\x1b\\");
    v
}

#[test]
fn kitty_transmits_and_places_raw_rgba() {
    let (mut s, mut p) = screen(20, 5);
    s.feed(&mut p, b"ab");
    s.feed(&mut p, &apc("a=T,f=32,s=2,v=2,i=7", &RGBA_2X2));
    s.feed(&mut p, b"c");
    assert_eq!(s.take_replies(), b"\x1b_Gi=7;OK\x1b\\".to_vec());
    let img = s.image(7).expect("stored");
    assert_eq!((img.width, img.height), (2, 2));
    assert_eq!(img.pixels, [0xFFFF0000, 0xFF00FF00, 0xFF0000FF, 0x80FFFFFF]);
    let pl = s.placements();
    assert_eq!(pl.len(), 1);
    assert_eq!((pl[0].row, pl[0].col, pl[0].cols, pl[0].rows), (0, 2, 1, 1));
    // The APC string never reaches the text; the cursor moved right of the image.
    assert_eq!(s.row_text(0), "ab c");
}

#[test]
fn kitty_png_rgb_and_palette() {
    let (mut s, mut p) = screen(20, 5);
    s.feed(&mut p, &apc("a=t,f=100,i=1", PNG_RGB_3X2));
    s.feed(&mut p, &apc("a=t,f=100,i=2,q=1", PNG_PAL_2X2));
    assert_eq!(s.take_replies(), b"\x1b_Gi=1;OK\x1b\\".to_vec());
    assert_eq!(s.image(1).unwrap().pixels, [0xFFFF0000, 0xFF00FF00, 0xFF0000FF, 0xFFFFFFFF, 0xFF000000, 0xFF0A141E]);
    assert_eq!(s.image(2).unwrap().pixels, [0x00FF00FF, 0xFFFFFF00, 0xFFFFFF00, 0x00FF00FF]);
    assert!(s.placements().is_empty());
    // Shown later by id, with cells and no cursor movement.
    s.feed(&mut p, b"\x1b_Ga=p,i=1,c=4,r=2,C=1,p=3\x1b\\");
    assert_eq!(s.take_replies(), b"\x1b_Gi=1,p=3;OK\x1b\\".to_vec());
    let pl = s.placements();
    assert_eq!((pl[0].cols, pl[0].rows, pl[0].placement_id), (4, 2, 3));
    assert_eq!(s.cursor(), (0, 0));
    // Our own encoder round-trips through the decoder.
    let (w, h, px) = fixture_image();
    assert_eq!(crate::png::decode(&crate::png::encode_rgba(w, h, &px)).unwrap(), (w, h, px));
}

#[test]
fn kitty_chunked_and_compressed_across_feeds() {
    let data = crate::png::zlib_stored(&RGBA_2X2);
    let b64 = base64_encode(&data);
    let (a, b) = b64.split_at(10); // not a multiple of 4: the decoder carries over
    let mut bytes = format!("x\x1b_Ga=T,f=32,s=2,v=2,o=z,i=9,m=1;").into_bytes();
    bytes.extend_from_slice(a);
    bytes.extend_from_slice(b"\x1b\\\x1b_Gm=0;");
    bytes.extend_from_slice(b);
    bytes.extend_from_slice(b"\x1b\\y");
    // One byte at a time: every split point of ESC _ and ESC \ is crossed.
    let (mut s, mut p) = screen(20, 5);
    for byte in &bytes { s.feed(&mut p, core::slice::from_ref(byte)); }
    assert_eq!(s.take_replies(), b"\x1b_Gi=9;OK\x1b\\".to_vec());
    assert_eq!(s.image(9).unwrap().pixels[3], 0x80FFFFFF);
    assert_eq!(s.row_text(0), "x y");
    // Other escapes next to APC strings still work; a cancelled APC leaves text alone.
    let (mut s, mut p) = screen(20, 5);
    s.feed(&mut p, b"\x1b[31mA\x1b_Gjunk\x1b[32mB");
    assert_eq!(s.row_text(0), "AB");
    assert_eq!(s.visible(1, 0).fg, crate::Color::Indexed(2));
}

#[test]
fn kitty_query_errors_and_quiet() {
    let (mut s, mut p) = screen(20, 5);
    s.feed(&mut p, &apc("a=q,i=31,s=1,v=1,f=24", &[0, 0, 0]));
    assert_eq!(s.take_replies(), b"\x1b_Gi=31;OK\x1b\\".to_vec());
    assert!(s.image(31).is_none());
    s.feed(&mut p, &apc("a=q,i=32,t=f", b"/tmp/x.png"));
    assert!(s.take_replies().starts_with(b"\x1b_Gi=32;EINVAL"));
    s.feed(&mut p, &apc("a=t,i=33,s=4,v=4", &[0; 8]));
    assert!(s.take_replies().starts_with(b"\x1b_Gi=33;ENODATA"));
    s.feed(&mut p, &apc("a=t,i=34,f=100", b"not a png"));
    assert!(s.take_replies().starts_with(b"\x1b_Gi=34;EBADPNG"));
    s.feed(&mut p, &apc("a=t,i=35,s=4,v=4,q=2", &[0; 8]));
    s.feed(&mut p, b"\x1b_Ga=p,i=99\x1b\\");
    assert_eq!(s.take_replies(), b"\x1b_Gi=99;ENOENT:no such image\x1b\\".to_vec());
    // No id, no reply; an image number gets a fresh id back.
    s.feed(&mut p, &apc("a=t,s=1,v=1,f=24", &[1, 2, 3]));
    assert!(s.take_replies().is_empty());
    s.feed(&mut p, &apc("a=t,I=5,s=1,v=1,f=24", &[1, 2, 3]));
    let r = std::string::String::from_utf8(s.take_replies()).unwrap();
    assert!(r.starts_with("\x1b_Gi=") && r.contains(",I=5;OK"), "{r:?}");
}

#[test]
fn kitty_deletes() {
    let (mut s, mut p) = screen(20, 5);
    s.feed(&mut p, &apc("a=T,f=32,s=2,v=2,i=1,q=2", &RGBA_2X2));
    s.feed(&mut p, &apc("a=T,f=32,s=2,v=2,i=2,q=2,z=5", &RGBA_2X2));
    s.feed(&mut p, b"\x1b_Ga=p,i=1,q=2\x1b\\");
    assert_eq!(s.placements().len(), 3);
    // Lower case: placements go, the data stays.
    s.feed(&mut p, b"\x1b_Ga=d,d=i,i=1\x1b\\");
    assert_eq!(s.placements().len(), 1);
    assert!(s.image(1).is_some());
    // By z-index, then upper case frees what nothing shows.
    s.feed(&mut p, b"\x1b_Ga=d,d=Z,z=5\x1b\\");
    assert!(s.placements().is_empty());
    assert!(s.image(2).is_none());
    s.feed(&mut p, b"\x1b_Ga=d,d=I,i=1\x1b\\");
    assert!(s.image(1).is_none());
    assert_eq!(s.images().bytes(), 0);
    // By cell, column, and everything on screen.
    s.feed(&mut p, b"\x1b[1;1H");
    s.feed(&mut p, &apc("a=T,f=32,s=2,v=2,i=3,q=2,c=2,r=2", &RGBA_2X2));
    s.feed(&mut p, b"\x1b_Ga=d,d=p,x=5,y=1\x1b\\");
    assert_eq!(s.placements().len(), 1);
    s.feed(&mut p, b"\x1b_Ga=d,d=p,x=2,y=2\x1b\\");
    assert!(s.placements().is_empty());
    s.feed(&mut p, b"\x1b_Ga=p,i=3,q=2,c=2,r=2\x1b\\\x1b_Ga=d,d=x,x=1\x1b\\");
    assert_eq!(s.placements().len(), 1);
    s.feed(&mut p, b"\x1b_Ga=d\x1b\\");
    assert!(s.placements().is_empty());
    assert!(s.image(3).is_some());
}

#[test]
fn sixel_fixture_decodes_with_known_colours() {
    let (mut s, mut p) = screen(20, 5);
    s.feed(&mut p, b"\x1b[2;3H");
    s.feed(&mut p, &fixture_sixel());
    let pl = s.placements();
    assert_eq!(pl.len(), 1);
    let (w, h) = FIXTURE_SIXEL_SIZE;
    assert_eq!((pl[0].width, pl[0].height, pl[0].row, pl[0].col, pl[0].cols, pl[0].rows), (w, h, 1, 2, 3, 1));
    // Below the image, in its column.
    assert_eq!(s.cursor(), (2, 2));
    let img = s.images();
    assert_eq!(img.len(), 1);
    let pixels = &s.placements_iter().next().map(|p| s.images().get(p.key).unwrap().pixels.clone()).unwrap();
    for y in 0..h { for x in 0..w {
        let want = FIXTURE_SIXEL_COLORS[(x >= w / 2) as usize] | 0xFF00_0000;
        assert_eq!(pixels[y * w + x], want, "pixel {x},{y}");
    } }
}

#[test]
fn sixel_hls_palette_repeat_and_transparency() {
    let (mut s, mut p) = screen(20, 5);
    // HLS hue 120 is red in DEC's circle; P2=1 keeps undrawn pixels clear. `@` sets bit 0 only.
    s.feed(&mut p, b"\x1bP0;1q#3;1;120;50;100#3!3@$#4;2;0;100;0~-#3?\x1b\\");
    let key = s.placements_iter().next().unwrap().key;
    let img = s.images().get(key).unwrap();
    assert_eq!((img.width, img.height), (3, 6));
    // Column 0: the green `~` over red `@` (drawn after); row 1 (bit 1) only green.
    assert_eq!(img.pixels[0], 0xFF00FF00);
    assert_eq!(img.pixels[1], 0xFFFF0000);
    assert_eq!(img.pixels[3], 0xFF00FF00);
    assert_eq!(img.pixels[4] >> 24, 0, "undrawn stays transparent");
    // An opaque background (P2 0) fills undrawn pixels with register 0.
    let (mut s, mut p) = screen(20, 5);
    s.feed(&mut p, b"\x1bPq\"1;1;2;6#1;2;100;100;100@\x1b\\");
    let key = s.placements_iter().next().unwrap().key;
    assert_eq!(s.images().get(key).unwrap().pixels[..4], [0xFFFFFFFF, 0xFF000000, 0xFF000000, 0xFF000000]);
}

#[test]
fn placements_scroll_into_history_and_go_after_the_trim() {
    let (mut s, mut p) = screen(10, 3);
    s.set_history_max(2);
    s.feed(&mut p, b"top\r\n");
    s.feed(&mut p, &apc("a=T,f=32,s=2,v=2,i=1,q=2,c=2,r=1", &RGBA_2X2));
    assert_eq!(s.placements()[0].row, 1);
    s.feed(&mut p, b"\r\n\r\n\r\n");
    // Three lines scrolled; one was trimmed: the anchor is on history row 1 of 2.
    assert_eq!(s.history_len(), 2);
    assert_eq!(s.placements()[0].row, 1);
    s.feed(&mut p, b"\r\n");
    assert_eq!(s.placements()[0].row, 0);
    s.feed(&mut p, b"\r\n");
    assert!(s.placements().is_empty());
    assert!(s.image(1).is_some(), "an image with an id outlives its placements");
    // Anonymous images go with their last placement.
    s.feed(&mut p, FIXTURE_SIXEL);
    assert_eq!(s.images().len(), 2);
    for _ in 0..6 { s.feed(&mut p, b"\r\n"); }
    assert_eq!(s.images().len(), 1);
}

#[test]
fn erase_and_alternate_screen_clear_placements() {
    let (mut s, mut p) = screen(10, 3);
    s.feed(&mut p, &apc("a=T,f=32,s=2,v=2,i=1,q=2", &RGBA_2X2));
    s.feed(&mut p, b"\x1b[?1049h");
    assert!(s.placements().is_empty(), "the alternate screen starts with none");
    s.feed(&mut p, b"\x1b_Ga=p,i=1,q=2\x1b\\");
    assert_eq!(s.placements().len(), 1);
    s.feed(&mut p, b"\x1b[?1049l");
    assert_eq!(s.placements().len(), 1, "the primary's is back");
    s.feed(&mut p, b"\x1b[?1049h\x1b[?1049l");
    assert_eq!(s.placements().len(), 1);
    // Scrolled into the history, ED 2 keeps it; ED 3 clears the rest.
    s.feed(&mut p, b"\r\n\r\n\r\n\r\n\x1b[2J");
    assert_eq!(s.placements().len(), 1);
    s.feed(&mut p, b"\x1b[H\x1b_Ga=p,i=1,q=2\x1b\\\x1b[2J");
    assert_eq!(s.placements().len(), 1);
    s.feed(&mut p, b"\x1b[3J");
    assert!(s.placements().is_empty());
}

#[test]
fn memory_cap_evicts_the_oldest() {
    let (mut s, mut p) = screen(10, 3);
    s.set_image_memory_cap(10 * 1024 * 1024);
    // 1024x1024 RGB, 4 MiB decoded each, sent in 4096-byte chunks as kitty clients do.
    let big = crate::png::zlib_stored(&alloc::vec![7u8; 1024 * 1024 * 3]);
    for i in 1..=3 {
        let b64 = base64_encode(&big);
        let mut chunks = b64.chunks(4096).peekable();
        let mut first = true;
        while let Some(c) = chunks.next() {
            let m = chunks.peek().is_some() as u8;
            let head = if first { format!("a=t,f=24,s=1024,v=1024,i={i},q=2,o=z,m={m}") } else { format!("m={m}") };
            first = false;
            let mut v = format!("\x1b_G{head};").into_bytes();
            v.extend_from_slice(c);
            v.extend_from_slice(b"\x1b\\");
            s.feed(&mut p, &v);
        }
    }
    assert!(s.image(1).is_none());
    assert!(s.image(2).is_some() && s.image(3).is_some());
    assert_eq!(s.images().bytes(), 8 * 1024 * 1024);
    assert_eq!(s.image(3).unwrap().pixels[5], 0xFF070707);
}

#[test]
fn replies_advertise_graphics() {
    let (mut s, mut p) = screen(10, 3);
    s.feed(&mut p, b"\x1b[c\x1b[?2;1;0S\x1b[?1;1;0S\x1b[16t\x1b[14t");
    assert_eq!(s.take_replies(), b"\x1b[?62;4;22c\x1b[?2;0;100;60S\x1b[?1;0;256S\x1b[6;20;10t\x1b[4;60;100t".to_vec());
}

#[test]
fn fixture_kitty_png_shows_the_quadrants() {
    let (mut s, mut p) = screen(20, 5);
    s.feed(&mut p, &fixture_kitty_png());
    assert!(s.take_replies().is_empty());
    let img = s.image(FIXTURE_KITTY_PNG_ID).unwrap();
    let (w, h) = FIXTURE_SIZE;
    assert_eq!((img.width, img.height), (w, h));
    assert_eq!(img.pixels[0], 0xFF00_0000 | FIXTURE_QUADRANTS[0]);
    assert_eq!(img.pixels[w * h - 1], 0xFF00_0000 | FIXTURE_QUADRANTS[3]);
    let pl = s.placements();
    assert_eq!((pl[0].cols, pl[0].rows), FIXTURE_KITTY_CELLS);
    assert!(fixture_kitty_png().windows(3).filter(|w| *w == b"m=1").count() >= 1, "sent in chunks");
}

#[test]
fn composite_draws_into_the_cell_box_only() {
    let (mut s, mut p) = screen(8, 4);
    s.feed(&mut p, b"\x1b[2;2H");
    s.feed(&mut p, &fixture_kitty_png()); // 8x2 cells from column 1, row 1: clipped at 7 columns
    let (cw, ch, stride) = (10, 20, 100);
    let mut px = alloc::vec![0x123456u32; stride * 100];
    composite(&s, cw, ch, &[], &mut px, stride);
    let at = |x: usize, y: usize| px[y * stride + x];
    assert_eq!(at(10 + 5, 20 + 5), FIXTURE_QUADRANTS[0]);
    assert_eq!(at(79, 20 + 5), FIXTURE_QUADRANTS[1]);
    assert_eq!(at(10 + 5, 20 + 30), FIXTURE_QUADRANTS[2]);
    assert_eq!(at(5, 25), 0x123456, "left of the image");
    assert_eq!(at(85, 25), 0x123456, "right of the grid");
    assert_eq!(at(15, 65), 0x123456, "below the image");
    // Only the rows asked for.
    let mut px2 = alloc::vec![0x123456u32; stride * 100];
    composite(&s, cw, ch, &[false, false, true, false], &mut px2, stride);
    assert_eq!(px2[25 * stride + 15], 0x123456);
    assert_eq!(px2[45 * stride + 15], FIXTURE_QUADRANTS[2]);
}

#[cfg(feature = "render")]
#[test]
fn painter_draws_images_over_text() {
    use crate::render::{Cache, Painter};
    let mut s = Screen::new(10, 5);
    let mut p = Parser::new();
    let mut painter = Painter::new(10, 20);
    let mut cache = alloc::boxed::Box::new(Cache::new());
    let stride = 120; // wider than the grid: the painter draws into part of a window
    let mut px = alloc::vec![0u32; stride * 100];
    let dirty = s.take_dirty();
    painter.draw(&s, &dirty, &mut cache, &mut px, stride, true);
    s.feed(&mut p, b"hello\r\n");
    s.feed(&mut p, &fixture_kitty_png());
    s.feed(&mut p, b"\r\n\r\nend");
    let dirty = s.take_dirty();
    assert!(dirty[1] && dirty[2]);
    painter.draw(&s, &dirty, &mut cache, &mut px, stride, true);
    let at = |x: usize, y: usize| px[y * stride + x];
    assert_eq!(at(5, 25), FIXTURE_QUADRANTS[0]);
    assert_eq!(at(75, 25), FIXTURE_QUADRANTS[1]);
    assert_eq!(at(5, 55), FIXTURE_QUADRANTS[2]);
    assert_eq!(at(75, 55), FIXTURE_QUADRANTS[3]);
    assert_eq!(at(105, 25), crate::DEFAULT_BG, "the sliver right of the grid is not painted over");
    // Deleting it redraws its rows as text.
    s.feed(&mut p, b"\x1b_Ga=d\x1b\\");
    let dirty = s.take_dirty();
    assert!(dirty[1] && dirty[2]);
    painter.draw(&s, &dirty, &mut cache, &mut px, stride, true);
    assert_eq!(px[25 * stride + 5], crate::DEFAULT_BG);
}
