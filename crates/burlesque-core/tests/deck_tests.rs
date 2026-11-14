// SPDX-License-Identifier: GPL-3.0-only
//! Comprehensive host integration tests for Burlesque presentations engine.

use burlesque_core::*;
use sex_archive::{ArchiveBuilder, OutputFormat};

/// A minimal valid 2x2 red PNG image.
fn make_test_png() -> Vec<u8> {
    // 8-byte signature
    let mut png = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
    ];

    // IHDR chunk (2x2, 8-bit RGB)
    let ihdr_data = [
        0x00, 0x00, 0x00, 0x02, // width = 2
        0x00, 0x00, 0x00, 0x02, // height = 2
        0x08,                   // bit depth = 8
        0x02,                   // color type = RGB
        0x00,                   // compression = 0
        0x00,                   // filter = 0
        0x00,                   // interlace = 0
    ];
    let mut ihdr_chunk = Vec::new();
    ihdr_chunk.extend_from_slice(b"IHDR");
    ihdr_chunk.extend_from_slice(&ihdr_data);
    let ihdr_crc = crc32fast::hash(&ihdr_chunk);

    png.extend_from_slice(&(ihdr_data.len() as u32).to_be_bytes());
    png.extend_from_slice(&ihdr_chunk);
    png.extend_from_slice(&ihdr_crc.to_be_bytes());

    // IDAT chunk (2 scanlines: [filter=0, R, G, B, R, G, B], [filter=0, R, G, B, R, G, B])
    let raw_scanlines = [
        0x00, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00, // row 0: red, red
        0x00, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00, // row 1: red, red
    ];
    let compressed_idat = miniz_oxide::deflate::compress_to_vec_zlib(&raw_scanlines, 6);
    let mut idat_chunk = Vec::new();
    idat_chunk.extend_from_slice(b"IDAT");
    idat_chunk.extend_from_slice(&compressed_idat);
    let idat_crc = crc32fast::hash(&idat_chunk);

    png.extend_from_slice(&(compressed_idat.len() as u32).to_be_bytes());
    png.extend_from_slice(&idat_chunk);
    png.extend_from_slice(&idat_crc.to_be_bytes());

    // IEND chunk
    let mut iend_chunk = Vec::new();
    iend_chunk.extend_from_slice(b"IEND");
    let iend_crc = crc32fast::hash(&iend_chunk);
    png.extend_from_slice(&0u32.to_be_bytes());
    png.extend_from_slice(&iend_chunk);
    png.extend_from_slice(&iend_crc.to_be_bytes());

    png
}

fn create_pptx_fixture() -> Vec<u8> {
    let test_png = make_test_png();
    let mut zip = ArchiveBuilder::new(OutputFormat::Zip);

    let pres_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
    <p:sldSz cx="12192000" cy="6858000"/>
    <p:sldIdLst>
        <p:sldId id="rId1"/>
        <p:sldId id="rId2"/>
    </p:sldIdLst>
</p:presentation>"#;

    let pres_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
    <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
    <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/>
</Relationships>"#;

    let slide1_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
    <p:cSld>
        <p:spTree>
            <!-- Title shape -->
            <p:sp>
                <p:spPr>
                    <a:xfrm>
                        <a:off x="1000000" y="800000"/>
                        <a:ext cx="10000000" cy="1500000"/>
                    </a:xfrm>
                    <a:prstGeom prst="rect"/>
                    <a:solidFill>
                        <a:srgbClr val="180828"/>
                    </a:solidFill>
                </p:spPr>
                <p:txBody>
                    <a:p>
                        <a:pPr algn="ctr"/>
                        <a:r>
                            <a:rPr sz="4400" b="1">
                                <a:solidFill>
                                    <a:srgbClr val="F43F5E"/>
                                </a:solidFill>
                            </a:rPr>
                            <a:t>SexOS Burlesque Presentations</a:t>
                        </a:r>
                    </a:p>
                </p:txBody>
            </p:sp>
            <!-- Subtitle and bullets shape -->
            <p:sp>
                <p:spPr>
                    <a:xfrm>
                        <a:off x="1000000" y="2600000"/>
                        <a:ext cx="10000000" cy="3500000"/>
                    </a:xfrm>
                    <a:prstGeom prst="roundRect"/>
                </p:spPr>
                <p:txBody>
                    <a:p>
                        <a:pPr lvl="0"/>
                        <a:r>
                            <a:rPr sz="2800">
                                <a:solidFill>
                                    <a:srgbClr val="F59E0B"/>
                                </a:solidFill>
                            </a:rPr>
                            <a:t>Slide 1 Subtitle Heading</a:t>
                        </a:r>
                    </a:p>
                    <a:p>
                        <a:pPr lvl="1"/>
                        <a:r>
                            <a:rPr sz="2200"/>
                            <a:t>High fidelity vector drawing with tiny-skia</a:t>
                        </a:r>
                    </a:p>
                    <a:p>
                        <a:pPr lvl="1"/>
                        <a:r>
                            <a:rPr sz="2200"/>
                            <a:t>True proportional typography via sex-font</a:t>
                        </a:r>
                    </a:p>
                </p:txBody>
            </p:sp>
            <!-- Embedded Picture -->
            <p:pic>
                <p:spPr>
                    <a:xfrm>
                        <a:off x="8000000" y="3000000"/>
                        <a:ext cx="2500000" cy="2500000"/>
                    </a:xfrm>
                </p:spPr>
                <p:blipFill>
                    <a:blip r:embed="rIdImg1"/>
                </p:blipFill>
            </p:pic>
        </p:spTree>
    </p:cSld>
</p:sld>"#;

    let slide1_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
    <Relationship Id="rIdNotes1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide1.xml"/>
    <Relationship Id="rIdImg1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"#;

    let slide2_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
    <p:cSld>
        <p:spTree>
            <p:sp>
                <p:spPr>
                    <a:xfrm>
                        <a:off x="1000000" y="800000"/>
                        <a:ext cx="10000000" cy="1500000"/>
                    </a:xfrm>
                </p:spPr>
                <p:txBody>
                    <a:p>
                        <a:r>
                            <a:rPr sz="4000" b="1"/>
                            <a:t>Slide 2: Architecture &amp; Performance</a:t>
                        </a:r>
                    </a:p>
                </p:txBody>
            </p:sp>
        </p:spTree>
    </p:cSld>
</p:sld>"#;

    let notes1_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
         xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
    <p:cSld>
        <p:spTree>
            <p:sp>
                <p:txBody>
                    <a:p>
                        <a:r>
                            <a:t>Welcome to the Burlesque presentation engine on SexOS.</a:t>
                        </a:r>
                    </a:p>
                    <a:p>
                        <a:r>
                            <a:t>Emphasize zero-copy IPC and microkernel isolation.</a:t>
                        </a:r>
                    </a:p>
                </p:txBody>
            </p:sp>
        </p:spTree>
    </p:cSld>
</p:notes>"#;

    zip.add_file("ppt/presentation.xml", pres_xml.len() as u64, 0, pres_xml.as_bytes()).unwrap();
    zip.add_file("ppt/_rels/presentation.xml.rels", pres_rels.len() as u64, 0, pres_rels.as_bytes()).unwrap();
    zip.add_file("ppt/slides/slide1.xml", slide1_xml.len() as u64, 0, slide1_xml.as_bytes()).unwrap();
    zip.add_file("ppt/slides/_rels/slide1.xml.rels", slide1_rels.len() as u64, 0, slide1_rels.as_bytes()).unwrap();
    zip.add_file("ppt/slides/slide2.xml", slide2_xml.len() as u64, 0, slide2_xml.as_bytes()).unwrap();
    zip.add_file("ppt/notesSlides/notesSlide1.xml", notes1_xml.len() as u64, 0, notes1_xml.as_bytes()).unwrap();
    zip.add_file("ppt/media/image1.png", test_png.len() as u64, 0, &test_png[..]).unwrap();

    let mut out = Vec::new();
    zip.write(&mut out, &mut sex_archive::no_progress).unwrap();
    out
}

fn create_odp_fixture() -> Vec<u8> {
    let mut zip = ArchiveBuilder::new(OutputFormat::Zip);

    let content_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
                         xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
                         xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
                         xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
                         xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0">
    <office:body>
        <office:presentation>
            <draw:page draw:name="First ODP Slide">
                <draw:frame svg:x="2cm" svg:y="2cm" svg:width="20cm" svg:height="4cm">
                    <draw:text-box>
                        <text:p>Welcome to OpenDocument Presentations</text:p>
                    </draw:text-box>
                </draw:frame>
                <draw:frame svg:x="2cm" svg:y="7cm" svg:width="20cm" svg:height="6cm">
                    <draw:text-box>
                        <text:list>
                            <text:list-item>
                                <text:p>First bullet point in ODP</text:p>
                            </text:list-item>
                            <text:list-item>
                                <text:p>Second bullet point in ODP</text:p>
                            </text:list-item>
                        </text:list>
                    </draw:text-box>
                </draw:frame>
                <presentation:notes>
                    <text:p>Presenter notes from ODP test document.</text:p>
                </presentation:notes>
            </draw:page>
        </office:presentation>
    </office:body>
</office:document-content>"#;

    zip.add_file("content.xml", content_xml.len() as u64, 0, content_xml.as_bytes()).unwrap();

    let mut out = Vec::new();
    zip.write(&mut out, &mut sex_archive::no_progress).unwrap();
    out
}

#[test]
fn test_pptx_import_and_render() {
    let pptx_bytes = create_pptx_fixture();
    let deck = Deck::open_pptx(&pptx_bytes).expect("open pptx fixture succeeds");

    assert_eq!(deck.slide_count(), 2);
    let slide1 = deck.get_slide(0).expect("slide 1 exists");
    assert!(slide1.title.contains("SexOS Burlesque"));
    assert!(slide1.speaker_notes.contains("Welcome to the Burlesque"));

    // Check that elements were parsed
    assert!(!slide1.elements.is_empty());

    // Rasterize slide 1 to pixels
    let w = 960u32;
    let h = 540u32;
    let mut pixels = vec![0u32; (w * h) as usize];
    render_slide(slide1, &deck.theme, deck.aspect_ratio, &mut pixels, w, h);

    // Verify non-blank drawing
    let bg_word = deck.theme.palette.background.premultiplied();
    let drawn_pixels = pixels.iter().filter(|&&p| p != bg_word).count();
    assert!(drawn_pixels > 100, "Slide 1 must render text and shapes (got {} drawn pixels)", drawn_pixels);
}

#[test]
fn test_presenter_session_and_view_rendering() {
    let pptx_bytes = create_pptx_fixture();
    let deck = Deck::open_pptx(&pptx_bytes).expect("open pptx fixture succeeds");

    let mut session = PresenterSession::new(deck);
    assert_eq!(session.current_slide, 0);
    assert_eq!(session.slide_counter_text(), "1 / 2");

    // Advance
    assert!(session.next_slide());
    assert_eq!(session.current_slide, 1);
    assert_eq!(session.slide_counter_text(), "2 / 2");

    // Timer
    session.tick(125_000); // 2 mins 5 secs
    assert_eq!(session.timer_text(), "02:05");

    // Blank screen toggle
    session.toggle_black_screen();
    assert_eq!(session.blank_mode, BlankMode::Black);
    session.clear_blank();
    assert_eq!(session.blank_mode, BlankMode::None);

    // Render presenter view
    let pw = 1024u32;
    let ph = 640u32;
    let mut buf = vec![0u32; (pw * ph) as usize];
    render_presenter_view(&session, &mut buf, pw, ph);

    // Verify presenter view has rendered content
    let non_zero = buf.iter().filter(|&&p| p != 0).count();
    assert!(non_zero > 1000, "Presenter view must render headers, panels, previews");
}

#[test]
fn test_pdf_export_and_vellum_verification() {
    let pptx_bytes = create_pptx_fixture();
    let deck = Deck::open_pptx(&pptx_bytes).expect("open pptx fixture succeeds");

    let pdf_bytes = deck.export_pdf().expect("export_pdf succeeds");

    // Basic PDF structure checks
    assert!(pdf_bytes.starts_with(b"%PDF-1.4\n"));
    assert!(pdf_bytes.ends_with(b"%%EOF\n"));

    // Verify with vellum-edit Editor (which loads Document, parses xref table & catalog, and collects pages)
    let editor = vellum_edit::Editor::open(pdf_bytes.clone()).expect("vellum-edit Editor::open succeeds");
    assert_eq!(editor.page_count(), deck.slide_count(), "Exported PDF page count must match slide count!");
    assert_eq!(editor.original(), &pdf_bytes[..]);
    let page0 = editor.page_info(0).expect("page 0 info succeeds");
    assert_eq!(page0.media_box, [0.0, 0.0, deck.width() as f64, deck.height() as f64]);
}

#[test]
fn test_odp_import_and_render() {
    let odp_bytes = create_odp_fixture();
    let deck = Deck::open_odp(&odp_bytes).expect("open odp fixture succeeds");

    assert_eq!(deck.slide_count(), 1);
    let slide = deck.get_slide(0).expect("first slide exists");
    assert!(slide.title.contains("First ODP Slide") || slide.title.contains("Welcome to OpenDocument"));
    assert!(slide.speaker_notes.contains("Presenter notes from ODP"));

    let w = 800u32;
    let h = 450u32;
    let mut pixels = vec![0u32; (w * h) as usize];
    render_slide(slide, &deck.theme, deck.aspect_ratio, &mut pixels, w, h);

    let bg_word = deck.theme.palette.background.premultiplied();
    let drawn = pixels.iter().filter(|&&p| p != bg_word).count();
    assert!(drawn > 50, "ODP slide must render content");
}

#[test]
fn test_markdown_deck_roundtrip() {
    let md_input = r#"# SexOS Burlesque
## A New Era of Presentations

- Built on pure Rust SASOS
- Microkernel capabilities and PDX IPC
- High performance presenter view

<!-- notes:
Introduce Burlesque to the audience.
Highlight memory safety and isolation.
-->

---

# Architecture Overview

```rust
fn present_deck(deck: &Deck) -> Result<()> {
    let session = PresenterSession::new(deck.clone());
    Ok(())
}
```

- Hardware isolated domains
- PKU memory protection

<!-- notes:
Explain the Rust code block.
-->
"#;

    let deck = Deck::from_markdown(md_input).expect("markdown import succeeds");
    assert_eq!(deck.slide_count(), 2);

    let slide1 = deck.get_slide(0).unwrap();
    assert_eq!(slide1.title, "SexOS Burlesque");
    assert!(slide1.speaker_notes.contains("Introduce Burlesque"));

    let slide2 = deck.get_slide(1).unwrap();
    assert_eq!(slide2.title, "Architecture Overview");
    assert!(slide2.speaker_notes.contains("Explain the Rust code block"));

    // Check code element
    let has_code = slide2.elements.iter().any(|e| matches!(e, SlideElement::Code(_)));
    assert!(has_code, "Slide 2 must have a code element");

    // Export to markdown and check roundtrip
    let exported_md = deck.to_markdown();
    assert!(exported_md.contains("# SexOS Burlesque"));
    assert!(exported_md.contains("Introduce Burlesque"));
    assert!(exported_md.contains("# Architecture Overview"));
    assert!(exported_md.contains("fn present_deck"));

    // Re-import exported markdown
    let reimported = Deck::from_markdown(&exported_md).expect("re-import exported markdown succeeds");
    assert_eq!(reimported.slide_count(), 2);
}

#[test]
fn test_command_stack_undo_redo() {
    let mut deck = Deck::new(AspectRatio::Ratio16x9);
    let mut stack = CommandStack::new(20);

    // Initial state
    assert_eq!(deck.slide_count(), 0);
    assert!(!stack.can_undo());
    assert!(!stack.can_redo());

    // 1. AddSlide
    let mut slide1 = Slide::new(1, SlideLayout::TitleSlide);
    slide1.title = String::from("Slide One");
    stack.apply(Command::AddSlide { index: 0, slide: slide1.clone() }, &mut deck);
    assert_eq!(deck.slide_count(), 1);
    assert!(stack.can_undo());

    // 2. AddSlide 2
    let mut slide2 = Slide::new(2, SlideLayout::TitleAndBody);
    slide2.title = String::from("Slide Two");
    stack.apply(Command::AddSlide { index: 1, slide: slide2.clone() }, &mut deck);
    assert_eq!(deck.slide_count(), 2);

    // 3. MoveSlide
    stack.apply(Command::MoveSlide { from: 0, to: 1 }, &mut deck);
    assert_eq!(deck.get_slide(0).unwrap().title, "Slide Two");
    assert_eq!(deck.get_slide(1).unwrap().title, "Slide One");

    // Undo MoveSlide
    assert!(stack.undo(&mut deck));
    assert_eq!(deck.get_slide(0).unwrap().title, "Slide One");
    assert_eq!(deck.get_slide(1).unwrap().title, "Slide Two");

    // Redo MoveSlide
    assert!(stack.redo(&mut deck));
    assert_eq!(deck.get_slide(0).unwrap().title, "Slide Two");
    assert_eq!(deck.get_slide(1).unwrap().title, "Slide One");

    // Undo twice
    assert!(stack.undo(&mut deck)); // undo Move
    assert!(stack.undo(&mut deck)); // undo AddSlide 2
    assert_eq!(deck.slide_count(), 1);
    assert_eq!(deck.get_slide(0).unwrap().title, "Slide One");

    // Undo last AddSlide
    assert!(stack.undo(&mut deck));
    assert_eq!(deck.slide_count(), 0);
    assert!(!stack.can_undo());
    assert!(stack.can_redo());
}

#[test]
fn test_malformed_and_fuzz_safety() {
    // 1. Truncated / garbage inputs
    assert!(Deck::open_pptx(b"").is_err());
    assert!(Deck::open_pptx(b"not a zip file at all").is_err());
    assert!(Deck::open_pptx(b"PK\x03\x04truncated").is_err());

    assert!(Deck::open_odp(b"").is_err());
    assert!(Deck::open_odp(b"random garbage data 1234567890").is_err());

    // 2. Malformed XML string
    let bad_xmls = [
        "<unclosed",
        "<<<<>>>>",
        "<a b=\"c></a>",
        "<![CDATA[ unclosed cdata",
        "<!-- unclosed comment",
        "&unknown_entity; &&&&&&",
        "non-ascii \u{0000} \u{FFFF}",
    ];

    for xml in bad_xmls {
        let _ = burlesque_core::xml_utils::XmlNode::parse(xml);
    }

    // 3. Empty or malformed markdown
    let empty_deck = Deck::from_markdown("").expect("empty markdown handled");
    assert!(empty_deck.slide_count() >= 1);

    let garbage_deck = Deck::from_markdown("----\n---\n---\n```\n```\n<!-- notes:\n").expect("malformed md handled");
    assert!(garbage_deck.slide_count() >= 1);
}
