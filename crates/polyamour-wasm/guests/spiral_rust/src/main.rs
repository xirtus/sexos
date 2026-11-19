// SPDX-License-Identifier: GPL-3.0-only
//! Real `#![no_std]` Rust `.wasm` Polyamour add-on compiled for `wasm32-unknown-unknown`
//! against `polyamour-sdk` (`SexWasmTransport` + `Client`).
//!
//! Rebuild:
//!   cargo build --release --target wasm32-unknown-unknown --manifest-path crates/polyamour-wasm/guests/spiral_rust/Cargo.toml
//!   cp crates/polyamour-wasm/guests/spiral_rust/target/wasm32-unknown-unknown/release/spiral_rust.wasm crates/polyamour-wasm/tests/spiral_rust.wasm
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, vec::Vec};
use polyamour_sdk::transport::wasm_guest::{exit, guest_args, SexWasmTransport, WasmBumpAlloc};
use polyamour_sdk::{
    Client, Color, Euler, MaterialParams, MeshData, ModifierSpec, ObjectRef, Placement, Primitive,
    SdkError, UiPropKind, UiPropSpec, Vec3,
};

#[global_allocator]
static ALLOC: WasmBumpAlloc = WasmBumpAlloc::new();

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { exit(1) }
}

fn run() -> Result<(), SdkError> {
    let args = guest_args();
    let mut steps: u32 = 12;
    let mut radius: f32 = 1.5;
    let mut rise: f32 = 0.2;

    for arg in &args {
        if let Some((k, v)) = arg.split_once('=') {
            match k {
                "steps" => {
                    if let Ok(n) = v.parse::<u32>() {
                        steps = n.clamp(3, 96);
                    }
                }
                "radius" => {
                    if let Ok(r) = v.parse::<f32>() {
                        if r.is_finite() {
                            radius = r.clamp(0.4, 20.0);
                        }
                    }
                }
                "rise" => {
                    if let Ok(h) = v.parse::<f32>() {
                        if h.is_finite() {
                            rise = h.clamp(0.05, 2.0);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut pa = Client::open(SexWasmTransport::new())?;

    // Register interactive sidebar panel in Polyamour (3D View -> Sidebar -> WASM tab)
    pa.ui_panel("spiral_rust", "Rust WASM Staircase", "WASM")?;
    pa.ui_prop(
        "spiral_rust",
        &UiPropSpec {
            key: "steps".into(),
            kind: UiPropKind::Int,
            label: "Steps".into(),
            default: Some(format!("{steps}")),
            min: Some(3.0),
            max: Some(96.0),
        },
    )?;
    pa.ui_prop(
        "spiral_rust",
        &UiPropSpec {
            key: "radius".into(),
            kind: UiPropKind::Float,
            label: "Radius".into(),
            default: Some(format!("{radius}")),
            min: Some(0.4),
            max: Some(20.0),
        },
    )?;
    pa.ui_prop(
        "spiral_rust",
        &UiPropSpec {
            key: "rise".into(),
            kind: UiPropKind::Float,
            label: "Step Rise".into(),
            default: Some(format!("{rise}")),
            min: Some(0.05),
            max: Some(2.0),
        },
    )?;
    pa.ui_prop(
        "spiral_rust",
        &UiPropSpec {
            key: "rebuild".into(),
            kind: UiPropKind::Button,
            label: "Rebuild Staircase".into(),
            default: None,
            min: None,
            max: None,
        },
    )?;

    // Replace previous staircase pole if re-invoked from the sidebar button
    let old_pole = ObjectRef::named("WasmPole");
    if pa.exists(&old_pole)? {
        pa.delete(&old_pole)?;
    }

    let coll = pa.collection_new("WasmStairs", None)?;
    let brass = pa.material_new(
        "WasmBrass",
        &MaterialParams {
            base_color: Color::rgba(0.9, 0.72, 0.32, 1.0),
            metallic: 1.0,
            roughness: 0.22,
            ..MaterialParams::default()
        },
    )?;

    let height = steps as f32 * rise + 0.4;
    let pole = pa.add_primitive(
        Primitive::Cylinder { segments: 24, radius: 0.14, depth: height },
        &Placement::at(Vec3::new(0.0, 0.0, height * 0.5))
            .named("WasmPole")
            .in_collection(&coll),
    )?;
    pa.material_assign(&pole, &brass, None)?;

    let turn = core::f32::consts::TAU / 12.0;
    let mut first_eval_faces = 0usize;
    for i in 0..steps {
        let angle = i as f32 * turn;
        let step = pa.add_primitive(
            Primitive::Cube { size: 1.0 },
            &Placement::at(Vec3::new(
                radius * libm::cosf(angle),
                radius * libm::sinf(angle),
                i as f32 * rise,
            ))
            .named(format!("WasmStep{i:02}"))
            .with_rotation(Euler::new(0.0, 0.0, angle))
            .with_scale(Vec3::new(1.1, 0.38, 0.08))
            .in_collection(&coll),
        )?;
        pa.set_parent(&step, Some(&pole), true)?;
        pa.modifier_add(&step, ModifierSpec::Subdivision { levels: 1, simple: false })?;
        pa.material_assign(&step, &brass, None)?;

        if i == 0 {
            let eval = pa.mesh_get(&step, true)?;
            first_eval_faces = eval.face_count();
        }
    }

    // Helical handrail (>64 verts exercises binary base64 mesh upload from wasm32)
    let samples = steps * 6 + 1;
    let mut verts = Vec::with_capacity(samples as usize);
    for k in 0..samples {
        let a = k as f32 * (turn / 6.0);
        verts.push(Vec3::new(
            (radius + 0.5) * libm::cosf(a),
            (radius + 0.5) * libm::sinf(a),
            k as f32 * (rise / 6.0) + 0.85,
        ));
    }
    let edges: Vec<[u32; 2]> = (0..samples - 1).map(|k| [k, k + 1]).collect();
    let rail_mesh = MeshData::from_pydata(&verts, &edges, &[]).map_err(SdkError::Wire)?;
    let rail = pa.add_mesh_object("WasmHandrail", &rail_mesh, &Placement::default().in_collection(&coll))?;
    pa.set_parent(&rail, Some(&pole), true)?;
    pa.material_assign(&rail, &brass, None)?;

    pa.log(&format!(
        "spiral_rust: steps={steps} radius={radius} step0_eval_faces={first_eval_faces} rail_verts={samples}"
    ));
    let _ = pa.finish()?;
    Ok(())
}

#[no_mangle]
pub extern "C" fn _start() {
    if let Err(err) = run() {
        let mut t = SexWasmTransport::new();
        polyamour_sdk::Transport::log(&mut t, &format!("spiral_rust failed: {err}"));
        unsafe { exit(1) }
    }
}
