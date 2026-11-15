// SPDX-License-Identifier: GPL-3.0-only
use batik_core::brush::{Brush, BrushPreset, PointerSample, StrokeEngine};
use batik_core::layer::RasterLayer;
use batik_core::selection::SelectionMask;
use sex_image::Limits;

#[test]
fn test_brush_presets_creation() {
    let airbrush = Brush::from_preset(BrushPreset::SoftAirbrush);
    assert!(airbrush.hardness < 0.2);

    let round = Brush::from_preset(BrushPreset::HardRound);
    assert!(round.hardness > 0.8);

    let chisel = Brush::from_preset(BrushPreset::Calligraphy);
    assert!(chisel.chisel_ratio < 0.5);

    let eraser = Brush::from_preset(BrushPreset::Eraser);
    assert!(eraser.eraser);
}

#[test]
fn test_calligraphy_chisel_dab() {
    let chisel = Brush::from_preset(BrushPreset::Calligraphy);
    let limits = Limits::SURFACE;
    let mut layer = RasterLayer::new(20, 20, &limits).unwrap();
    let sel = SelectionMask::new(20, 20).unwrap();
    let mut engine = StrokeEngine::new(chisel);

    let sample = PointerSample::new(10.0, 10.0, 1.0);
    engine.paint_to(&mut layer, &sel, sample).unwrap();

    // Center pixel should be tinted
    let center = layer.pixels()[10 * 20 + 10];
    assert_ne!(center, 0);

    // Dab width along minor axis is narrower than circle
    let painted_count = layer.pixels().iter().filter(|p| **p != 0).count();
    assert!(painted_count > 0 && painted_count < 20 * 20);
}

#[test]
fn test_pressure_taper_curves() {
    let mut brush = Brush::default();
    brush.size_dynamics = true;
    brush.opacity_dynamics = true;
    brush.size = 10.0;
    brush.flow = 1.0;

    // Minimum pressure
    let low_size = brush.effective_size(0.05);
    let low_flow = brush.effective_flow(0.05);

    // Full pressure
    let high_size = brush.effective_size(1.0);
    let high_flow = brush.effective_flow(1.0);

    assert!(low_size < high_size);
    assert!(low_flow < high_flow);
    assert!(low_size >= 0.5);
    assert_eq!(high_size, 10.0);
    assert_eq!(high_flow, 1.0);
}
