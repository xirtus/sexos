use sex_web::{sex_html, Chrome, ChromePalette, Fonts, BAR};
use sex_theme::{tokens, Mode, DEFAULT_ACCENT};

fn shot(mode: Mode) -> Vec<u32> {
    let mut fonts = Fonts::new().unwrap();
    let (w, h) = (400, 250);
    let page = sex_html::render("<body style='margin:0;background:#fff'><p>Article</p>", w, h - BAR, &fonts);
    let mut pixels = vec![0; (w * h) as usize];
    let chrome = Chrome {
        text: "about:welcome",
        palette: ChromePalette::from_tokens(tokens(mode, DEFAULT_ACCENT)),
        ..Chrome::default()
    };
    sex_web::frame_with(&page, &mut fonts, &mut pixels, w, h, 0, None, &chrome);
    pixels
}

#[test]
fn chrome_changes_with_theme_while_site_pixels_stay_the_same() {
    let (light, dark, contrast) = (shot(Mode::Light), shot(Mode::Dark), shot(Mode::HighContrast));
    let at = |pixels: &[u32], x: usize, y: usize| pixels[y * 400 + x];
    assert_eq!(at(&light, 0, 0), tokens(Mode::Light, DEFAULT_ACCENT).surface & 0x00FF_FFFF);
    assert_eq!(at(&dark, 0, 0), tokens(Mode::Dark, DEFAULT_ACCENT).surface & 0x00FF_FFFF);
    assert_eq!(at(&contrast, 0, 0), tokens(Mode::HighContrast, DEFAULT_ACCENT).surface & 0x00FF_FFFF);
    assert_ne!(at(&light, 0, 0), at(&dark, 0, 0));
    for y in 50..250 {
        for x in [0, 50, 200, 399] {
            assert_eq!(at(&light, x, y), at(&dark, x, y), "site pixel {x},{y}");
            assert_eq!(at(&light, x, y), at(&contrast, x, y), "site pixel {x},{y}");
        }
    }
}
