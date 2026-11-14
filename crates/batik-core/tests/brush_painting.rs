// SPDX-License-Identifier: GPL-3.0-only
use batik_core::blend::split_argb;
use batik_core::brush::{bucket_fill, Brush, PointerSample, StrokeEngine};
use batik_core::geom::Rect;
use batik_core::layer::RasterLayer;
use batik_core::selection::{SelectionCombine, SelectionMask};
use sex_image::Limits;

#[test]
fn test_brush_pressure_dynamics() {
    let limits = Limits::SURFACE;
    let mut raster = RasterLayer::new(50, 50, &limits).unwrap();
    let selection = SelectionMask::new(50, 50).unwrap();

    let brush = Brush {
        size: 10.0,
        color: 0xFF00FF00, // green
        opacity: 1.0,
        flow: 1.0,
        hardness: 1.0,
        spacing: 0.2,
        eraser: false,
        size_dynamics: true,
        opacity_dynamics: false,
    };

    let mut engine = StrokeEngine::new(brush);

    // Light pressure sample at (10, 10) with pressure 0.2 -> radius ~1 px
    engine.paint_to(&mut raster, &selection, PointerSample::new(10.0, 10.0, 0.2)).unwrap();
    assert_eq!(raster.pixel_at(10, 10), Some(0xFF00FF00));
    // (14, 10) is 4 pixels away; with radius 1 it should NOT be painted
    assert_eq!(raster.pixel_at(14, 10), Some(0));

    // High pressure sample at (30, 30) with pressure 1.0 -> radius 5 px
    engine.finish_stroke();
    engine.paint_to(&mut raster, &selection, PointerSample::new(30.0, 30.0, 1.0)).unwrap();
    assert_eq!(raster.pixel_at(30, 30), Some(0xFF00FF00));
    // (34, 30) is 4 pixels away; with radius 5 it MUST be painted
    assert_eq!(raster.pixel_at(34, 30), Some(0xFF00FF00));
}

#[test]
fn test_brush_selection_clipping() {
    let limits = Limits::SURFACE;
    let mut raster = RasterLayer::new(20, 20, &limits).unwrap();
    let mut selection = SelectionMask::new(20, 20).unwrap();

    // Select only left half (x from 0 to 9)
    selection.select_rect(Rect::new(0, 0, 10, 20), SelectionCombine::Replace);

    let brush = Brush {
        size: 8.0,
        color: 0xFFFF0000,
        opacity: 1.0,
        flow: 1.0,
        hardness: 1.0,
        spacing: 0.2,
        eraser: false,
        size_dynamics: false,
        opacity_dynamics: false,
    };
    let mut engine = StrokeEngine::new(brush);

    // Paint dab right on border at (10, 10)
    engine.paint_to(&mut raster, &selection, PointerSample::new(10.0, 10.0, 1.0)).unwrap();

    // Left side inside selection MUST be painted
    assert_eq!(raster.pixel_at(8, 10), Some(0xFFFF0000));
    // Right side outside selection MUST NOT be painted!
    assert_eq!(raster.pixel_at(11, 10), Some(0));
}

#[test]
fn test_eraser_mode() {
    let limits = Limits::SURFACE;
    let mut raster = RasterLayer::new(10, 10, &limits).unwrap();
    raster.pixels.fill(0xFF0000FF); // opaque blue

    let selection = SelectionMask::new(10, 10).unwrap();

    let brush = Brush {
        size: 4.0,
        color: 0,
        opacity: 1.0,
        flow: 1.0,
        hardness: 1.0,
        spacing: 0.2,
        eraser: true,
        size_dynamics: false,
        opacity_dynamics: false,
    };
    let mut engine = StrokeEngine::new(brush);
    engine.paint_to(&mut raster, &selection, PointerSample::new(5.0, 5.0, 1.0)).unwrap();

    let (a, _, _, _) = split_argb(raster.pixel_at(5, 5).unwrap());
    assert_eq!(a, 0, "eraser should set alpha to 0");
}

#[test]
fn test_bucket_fill_scanline() {
    let limits = Limits::SURFACE;
    let mut raster = RasterLayer::new(10, 10, &limits).unwrap();
    let selection = SelectionMask::new(10, 10).unwrap();

    // Fill entire layer from (0, 0) with purple
    bucket_fill(&mut raster, 0, 0, 0xFF800080, 0, &selection).unwrap();

    for p in raster.pixels {
        assert_eq!(p, 0xFF800080);
    }
}
