// SPDX-License-Identifier: GPL-3.0-only
//! Host integration and Weft window interaction tests for Climax.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use climax::{Climax, ClimaxMode};
use climax::weft::{Button, Event, Id, Pointer, PointerKind, Strings, Theme, Ui};

fn create_app(width: u32, height: u32) -> (Ui, Climax, Vec<u32>) {
    let mut ui = Ui::new(width, height, Theme::light(), Strings::new("en"));
    let climax = Climax::build(&mut ui);
    let mut pixels = vec![0u32; (width * height) as usize];
    ui.paint(&mut pixels);
    (ui, climax, pixels)
}

fn apply(ui: &mut Ui, climax: &mut Climax, event: Event) {
    for r in ui.handle(event) {
        climax.on(ui, &r, || 1000);
    }
}

fn click(ui: &mut Ui, climax: &mut Climax, id: Id) {
    let c = ui.rect(id).center();
    for kind in [
        PointerKind::Move,
        PointerKind::Down(Button::Left),
        PointerKind::Up(Button::Left),
    ] {
        apply(
            ui,
            climax,
            Event::Pointer(Pointer {
                x: c.x,
                y: c.y,
                kind,
            }),
        );
    }
}

#[test]
fn test_climax_initial_layout_and_painting() {
    let (mut ui, climax, mut pixels) = create_app(800, 600);
    assert_eq!(climax.mode, ClimaxMode::Benchmarks);
    assert!(!climax.suite_ran);
    assert_eq!(climax.torture_runner.steps_completed(), 0);

    ui.paint(&mut pixels);
    assert!(pixels.iter().any(|px| *px != 0), "pixels must be drawn to window");
}

#[test]
fn test_climax_mode_switching_and_buttons() {
    let (mut ui, mut climax, _) = create_app(800, 600);
    assert_eq!(climax.mode, ClimaxMode::Benchmarks);

    // Click Thermal Torture tab
    let torture_tab = climax.choices[1];
    click(&mut ui, &mut climax, torture_tab);
    assert_eq!(climax.mode, ClimaxMode::Thermal);

    // Click Benchmarks tab
    let bench_tab = climax.choices[0];
    click(&mut ui, &mut climax, bench_tab);
    assert_eq!(climax.mode, ClimaxMode::Benchmarks);
}

#[test]
fn test_climax_run_suite_and_json_generation() {
    let (mut ui, mut climax, mut pixels) = create_app(800, 600);
    assert!(!climax.suite_ran);
    assert!(climax.suite_result.is_none());

    // Click Run Suite
    let run_btn = climax.run_suite;
    click(&mut ui, &mut climax, run_btn);

    assert!(climax.suite_ran);
    let res = climax.suite_result.as_ref().expect("suite result exists");
    assert!(res.arx_ops > 0);
    assert!(res.sha_ops > 0);
    assert!(res.gemm_ops > 0);
    assert!(res.dsp_ops > 0);
    assert!(res.stream_res.operations > 0);
    assert!(!res.ladder_steps.is_empty());
    assert!(res.scores.composite > 0);

    // Check json structure
    assert!(res.json_payload.contains("\"target\":\"sexos\""));
    assert!(res.json_payload.contains("\"arx\":"));
    assert!(res.json_payload.contains("\"sha256\":"));
    assert!(res.json_payload.contains("\"gemm\":"));
    assert!(res.json_payload.contains("\"dsp\":"));
    assert!(res.json_payload.contains("\"stream\":"));
    assert!(res.json_payload.contains("\"ladder\":"));
    assert!(res.json_payload.contains("\"scores\":"));

    ui.paint(&mut pixels);
}

#[test]
fn test_climax_torture_step_burst_and_reset() {
    let (mut ui, mut climax, _) = create_app(800, 600);

    // Switch to thermal mode
    let torture_tab = climax.choices[1];
    click(&mut ui, &mut climax, torture_tab);
    assert_eq!(climax.mode, ClimaxMode::Thermal);
    assert_eq!(climax.torture_runner.steps_completed(), 0);

    // Click Start Step
    let step_btn = climax.start_step;
    click(&mut ui, &mut climax, step_btn);
    assert_eq!(climax.torture_runner.steps_completed(), 1);
    assert!(!climax.torture_analysis.corruption_detected);

    // Click Run Burst (5x)
    let burst_btn = climax.run_burst;
    click(&mut ui, &mut climax, burst_btn);
    assert_eq!(climax.torture_runner.steps_completed(), 6);
    assert!(climax.torture_analysis.stability_percent > 0.0);

    // Click Reset
    let reset_btn = climax.reset;
    click(&mut ui, &mut climax, reset_btn);
    assert_eq!(climax.torture_runner.steps_completed(), 0);
}

#[test]
fn test_controls_fit_within_window_geometry() {
    let (ui, climax, _) = create_app(800, 600);
    for id in [
        climax.choices[0],
        climax.choices[1],
        climax.run_suite,
        climax.start_step,
        climax.run_burst,
        climax.reset,
    ] {
        let r = ui.rect(id);
        assert!(r.x >= 0.0 && r.y >= 0.0, "control rect {r:?} must have positive coordinates");
        assert!(r.x + r.w <= 800.0, "control rect {r:?} exceeds window width");
    }
}
