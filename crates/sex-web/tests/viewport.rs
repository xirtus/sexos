use sex_web::{Fonts, BAR};

#[test]
fn tiny_window_toolbar_never_writes_beyond_the_actual_surface() {
    let mut fonts = Fonts::new().unwrap();
    for (w, h) in [(1, 1), (8, 5), (23, 29), (31, 30), (64, 32), (0, 0)] {
        let page = sex_web::sex_html::render("<p>small</p>", w, (h - BAR).max(0), &fonts);
        let count = (w * h) as usize;
        let mut guarded = vec![0x11223344; count + 16];
        sex_web::frame(
            &page,
            &mut fonts,
            &mut guarded[..count],
            w,
            h,
            0,
            None,
            "a real address",
            true,
        );
        assert_eq!(&guarded[count..], &[0x11223344; 16], "{w}x{h}");
        if count > 0 {
            assert_ne!(guarded[0], 0x11223344);
        }
    }
}
