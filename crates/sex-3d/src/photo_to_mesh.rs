// SPDX-License-Identifier: GPL-3.0-only
//! Offline, pure-Rust single-photo-to-textured-3D-mesh generator.
//!
//! Takes an RGBA8 image and produces a closed, watertight, 2-manifold,
//! UV-mapped two-sided relief [`Mesh`] with vertex colors and a bleed-padded
//! albedo [`Image`], upright in Z-up space and optionally scaled to millimetres
//! for 3D printing or direct `.glb` / `.obj` / `.stl` export.
use crate::io::srgb_to_linear;
use crate::math::{Vec2, Vec3};
use crate::mesh::{edge_key, AttrDomain, Face, Mesh};
use crate::print_prep;
use crate::scene::{Image, Material, Scene};
use std::collections::{HashMap, VecDeque};

/// Mesh resolution preset for [`photo_to_mesh`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhotoMeshQuality {
    Draft,
    #[default]
    Standard,
    High,
}

impl PhotoMeshQuality {
    fn max_grid_dim(self) -> usize {
        match self {
            PhotoMeshQuality::Draft => 40,
            PhotoMeshQuality::Standard => 80,
            PhotoMeshQuality::High => 136,
        }
    }
}

/// Options controlling [`photo_to_mesh`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhotoMeshOptions {
    pub quality: PhotoMeshQuality,
    /// Deterministic seed controlling subtle relief depth/surface variation.
    pub take_seed: u32,
    /// Peak front+back thickness relative to the subject's major dimension.
    pub thickness_ratio: f32,
    /// Optional real-world target Z height in millimetres.
    pub real_height_mm: Option<f32>,
}

impl Default for PhotoMeshOptions {
    fn default() -> Self {
        Self {
            quality: PhotoMeshQuality::Standard,
            take_seed: 0,
            thickness_ratio: 0.25,
            real_height_mm: None,
        }
    }
}

/// Output of [`photo_to_mesh`]: a watertight UV-mapped mesh, vertex colors, and
/// a texture ready for scene insertion or export.
#[derive(Clone, Debug, PartialEq)]
pub struct PhotoMeshResult {
    pub mesh: Mesh,
    pub texture: Image,
    /// Fraction of image pixels classified as foreground in `0.0..=1.0`.
    pub foreground_coverage: f32,
}

impl PhotoMeshResult {
    /// Build a ready-to-export [`Scene`] containing the textured mesh object.
    pub fn into_scene(&self, name: &str) -> Scene {
        let mut scene = Scene::new();
        scene.images.push(self.texture.clone());
        let img_idx = scene.images.len() - 1;
        let mat = Material {
            name: format!("{name}_Mat"),
            base_color: [1.0, 1.0, 1.0, 1.0],
            base_color_texture: Some(img_idx),
            metallic: 0.0,
            roughness: 0.6,
            ..Default::default()
        };
        let mat_idx = scene.add_material(mat);
        let obj_idx = scene.add_mesh_object(name, self.mesh.clone());
        scene.objects[obj_idx].materials.push(mat_idx);
        scene.active_object = Some(obj_idx);
        scene.selected = vec![obj_idx];
        scene
    }
}

/// Generate a watertight, UV-mapped, upright 3D relief mesh from a single RGBA8 photo.
pub fn photo_to_mesh(
    width: u32,
    height: u32,
    rgba: &[u8],
    options: PhotoMeshOptions,
) -> Result<PhotoMeshResult, String> {
    if width == 0 || height == 0 {
        return Err("image dimensions must be non-zero".into());
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| "image dimensions overflow".to_string())?;
    if rgba.len() != expected {
        return Err(format!(
            "RGBA buffer length {} does not match {}x{}x4 ({expected})",
            rgba.len(),
            width,
            height
        ));
    }

    let max_dim = options.quality.max_grid_dim();
    let scale = max_dim as f32 / width.max(height) as f32;
    let gw = ((width as f32 * scale).round() as usize).clamp(8, max_dim);
    let gh = ((height as f32 * scale).round() as usize).clamp(8, max_dim);

    // 1. Bilinearly resample input RGBA8 onto working grid (gw x gh).
    let grid = resample_rgba(width as usize, height as usize, rgba, gw, gh);

    // 2. Segment foreground subject.
    let mut fg = segment_foreground(&grid, gw, gh)?;

    // 3. Morphological cleanup + manifold mask conditioning (no pinches, no 1px spurs).
    morphological_cleanup(&mut fg, gw, gh);
    let fg_count = fg.iter().filter(|&&b| b).count();
    if fg_count == 0 {
        return Err("image has no foreground subject".into());
    }
    let foreground_coverage = fg_count as f32 / (gw * gh) as f32;

    // 4. Exact 2D Euclidean distance transform inside foreground mask.
    let dist = distance_transform(&fg, gw, gh);
    let max_dist = dist.iter().copied().fold(0.0f32, f32::max).max(1.0);
    let mut norm_dist: Vec<f32> = dist
        .iter()
        .zip(&fg)
        .map(|(&d, &in_fg)| if in_fg { (d / max_dist).clamp(0.0, 1.0) } else { 0.0 })
        .collect();
    smooth_masked(&mut norm_dist, &fg, gw, gh, 2);

    // 5. Luminance and gradient shape-from-shading relief detail.
    let lum: Vec<f32> = grid
        .iter()
        .map(|p| (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0)
        .collect();
    let mut low_pass = lum.clone();
    smooth_masked(&mut low_pass, &fg, gw, gh, 4);

    // 6. Construct vertex grid & relief field.
    let aspect = width as f32 / height as f32;
    let base_h = 2.0f32;
    let base_w = base_h * aspect;
    let thick_ratio = if options.thickness_ratio.is_finite() {
        options.thickness_ratio.clamp(0.02, 2.0)
    } else {
        0.25
    };
    let max_depth = 0.5 * thick_ratio * base_h.max(base_w);

    let vw = gw + 1;
    let vh = gh + 1;
    let mut is_boundary = vec![false; vw * vh];
    let mut is_used = vec![false; vw * vh];

    for vy in 0..vh {
        for vx in 0..vw {
            let mut occ = 0u8;
            for dy in [0isize, -1] {
                for dx in [0isize, -1] {
                    let cx = vx as isize + dx;
                    let cy = vy as isize + dy;
                    if cx >= 0 && cy >= 0 && (cx as usize) < gw && (cy as usize) < gh {
                        if fg[cy as usize * gw + cx as usize] {
                            occ += 1;
                        }
                    }
                }
            }
            let vi = vy * vw + vx;
            if occ > 0 {
                is_used[vi] = true;
                if occ < 4 {
                    is_boundary[vi] = true;
                }
            }
        }
    }

    // Smooth boundary XZ coordinates to remove pixel staircase artifacts.
    let mut vx_pos = vec![0.0f32; vw * vh];
    let mut vz_pos = vec![0.0f32; vw * vh];
    for vy in 0..vh {
        for vx in 0..vw {
            let vi = vy * vw + vx;
            vx_pos[vi] = (vx as f32 / gw as f32 - 0.5) * base_w;
            // Image row 0 is top (+Z); row gh is bottom (-Z).
            vz_pos[vi] = (0.5 - vy as f32 / gh as f32) * base_h;
        }
    }
    smooth_boundary_contour(&fg, gw, gh, &is_boundary, &mut vx_pos, &mut vz_pos);

    let mut front_depth = vec![0.0f32; vw * vh];
    let mut back_depth = vec![0.0f32; vw * vh];
    let mut vert_color = vec![[1.0f32; 4]; vw * vh];

    for vy in 0..vh {
        for vx in 0..vw {
            let vi = vy * vw + vx;
            if !is_used[vi] {
                continue;
            }
            let mut d_sum = 0.0f32;
            let mut l_sum = 0.0f32;
            let mut lp_sum = 0.0f32;
            let mut rgb_sum = [0.0f32; 3];
            let mut cnt = 0.0f32;
            for dy in [0isize, -1] {
                for dx in [0isize, -1] {
                    let cx = vx as isize + dx;
                    let cy = vy as isize + dy;
                    if cx >= 0 && cy >= 0 && (cx as usize) < gw && (cy as usize) < gh {
                        let ci = cy as usize * gw + cx as usize;
                        if fg[ci] {
                            d_sum += norm_dist[ci];
                            l_sum += lum[ci];
                            lp_sum += low_pass[ci];
                            let p = grid[ci];
                            rgb_sum[0] += p[0] as f32 / 255.0;
                            rgb_sum[1] += p[1] as f32 / 255.0;
                            rgb_sum[2] += p[2] as f32 / 255.0;
                            cnt += 1.0;
                        }
                    }
                }
            }
            if cnt > 0.0 {
                for c in 0..3 {
                    vert_color[vi][c] = srgb_to_linear((rgb_sum[c] / cnt).clamp(0.0, 1.0));
                }
            }
            let rim_depth = (max_depth * 0.035).max(1e-3);
            if is_boundary[vi] {
                front_depth[vi] = rim_depth;
                back_depth[vi] = rim_depth * 0.75;
                continue;
            }
            let dn = (d_sum / cnt.max(1.0)).clamp(0.0, 1.0);
            let l = l_sum / cnt.max(1.0);
            let lp = lp_sum / cnt.max(1.0);
            let high_pass = (l - lp).clamp(-0.5, 0.5);
            // Smooth pillow inflation curve with beveled silhouette rim.
            let inflate = (2.0 * dn - dn * dn).max(0.0).sqrt();
            let u = vx as f32 / gw as f32;
            let v = 1.0 - vy as f32 / gh as f32;
            let seed_mod = seeded_relief_modulation(u, v, options.take_seed);
            let sfs_detail = 0.14 * (l - 0.5) + 0.22 * high_pass;
            let front_factor = (1.0 + sfs_detail + seed_mod).clamp(0.45, 1.55);
            let back_factor = (0.70 + 0.35 * sfs_detail - 0.5 * seed_mod).clamp(0.35, 1.25);
            front_depth[vi] = (inflate * max_depth * front_factor).max(rim_depth * 1.1);
            back_depth[vi] = (inflate * max_depth * back_factor).max(rim_depth * 0.85);
        }
    }

    // 7. Build watertight, strictly manifold two-sided closed mesh.
    let mut mesh = Mesh::new();
    let mut front_id = vec![u32::MAX; vw * vh];
    let mut back_id = vec![u32::MAX; vw * vh];
    let mut point_colors: Vec<[f32; 4]> = Vec::new();
    let mut vert_uv = vec![Vec2::ZERO; vw * vh];

    for vy in 0..vh {
        for vx in 0..vw {
            let vi = vy * vw + vx;
            if !is_used[vi] {
                continue;
            }
            let x = vx_pos[vi];
            let z = vz_pos[vi];
            let u = (x / base_w + 0.5).clamp(0.0, 1.0);
            let v = (z / base_h + 0.5).clamp(0.0, 1.0);
            vert_uv[vi] = Vec2::new(u, v);

            let fid = mesh.add_vertex(Vec3::new(x, -front_depth[vi], z));
            point_colors.push(vert_color[vi]);
            let bid = mesh.add_vertex(Vec3::new(x, back_depth[vi], z));
            point_colors.push(vert_color[vi]);
            front_id[vi] = fid;
            back_id[vi] = bid;
        }
    }

    let cidx = |vx: usize, vy: usize| vy * vw + vx;
    for cy in 0..gh {
        for cx in 0..gw {
            if !fg[cy * gw + cx] {
                continue;
            }
            // Corners in image coords: tl=(cx,cy), tr=(cx+1,cy), bl=(cx,cy+1), br=(cx+1,cy+1)
            let tl = cidx(cx, cy);
            let tr = cidx(cx + 1, cy);
            let bl = cidx(cx, cy + 1);
            let br = cidx(cx + 1, cy + 1);

            // Front side faces -Y: CCW around +Y-to-eye is [bl, br, tr, tl].
            let f_tris = if (cx + cy) % 2 == 0 {
                [[bl, br, tr], [bl, tr, tl]]
            } else {
                [[bl, br, tl], [br, tr, tl]]
            };
            for tri in f_tris {
                mesh.faces.push(Face {
                    verts: tri.map(|i| front_id[i]).to_vec(),
                    uvs: tri.map(|i| vert_uv[i]).to_vec(),
                    material: 0,
                    smooth: true,
                });
            }

            // Back side faces +Y: opposite winding [bl, tl, tr, br].
            let b_tris = if (cx + cy) % 2 == 0 {
                [[bl, tr, br], [bl, tl, tr]]
            } else {
                [[bl, tl, br], [tl, tr, br]]
            };
            for tri in b_tris {
                mesh.faces.push(Face {
                    verts: tri.map(|i| back_id[i]).to_vec(),
                    uvs: tri.map(|i| vert_uv[i]).to_vec(),
                    material: 0,
                    smooth: true,
                });
            }

            // Stitch CCW boundary edges (a -> b on the front sheet) to the back
            // sheet with two side-wall triangles and mark silhouette seams.
            let rim_edges = [
                (cy + 1 == gh || !fg[(cy + 1) * gw + cx], bl, br),
                (cx + 1 == gw || !fg[cy * gw + cx + 1], br, tr),
                (cy == 0 || !fg[(cy - 1) * gw + cx], tr, tl),
                (cx == 0 || !fg[cy * gw + cx - 1], tl, bl),
            ];
            for (is_rim, a, b) in rim_edges {
                if !is_rim {
                    continue;
                }
                let fa = front_id[a];
                let fb = front_id[b];
                let ba = back_id[a];
                let bb = back_id[b];
                mesh.faces.push(Face {
                    verts: vec![fa, ba, bb],
                    uvs: vec![vert_uv[a], vert_uv[a], vert_uv[b]],
                    material: 0,
                    smooth: false,
                });
                mesh.faces.push(Face {
                    verts: vec![fa, bb, fb],
                    uvs: vec![vert_uv[a], vert_uv[b], vert_uv[b]],
                    material: 0,
                    smooth: false,
                });
                mesh.seams.insert(edge_key(fa, fb));
                mesh.seams.insert(edge_key(ba, bb));
            }
        }
    }

    let col_idx = mesh.add_color_attribute("Col", AttrDomain::Point, [1.0; 4]);
    mesh.color_attributes[col_idx].data = point_colors;

    // Ensure upright placement: center XY at origin and place min Z at 0.0.
    let b = mesh.bounds();
    if !b.is_empty() {
        let cen = b.center();
        mesh.translate(Vec3::new(-cen.x, -cen.y, -b.min.z));
    }
    if let Some(target_mm) = options.real_height_mm {
        print_prep::scale_to_height_mm(&mut mesh, target_mm);
    }

    // 8. Build bleed-padded albedo texture Image at full input resolution.
    let texture = build_bleed_texture(width as usize, height as usize, rgba, &fg, gw, gh);

    Ok(PhotoMeshResult {
        mesh,
        texture,
        foreground_coverage,
    })
}

/// Convenience helper creating a textured [`Scene`] directly from an RGBA8 image.
pub fn photo_to_scene(
    name: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
    options: PhotoMeshOptions,
) -> Result<Scene, String> {
    let res = photo_to_mesh(width, height, rgba, options)?;
    Ok(res.into_scene(name))
}

fn resample_rgba(w: usize, h: usize, rgba: &[u8], gw: usize, gh: usize) -> Vec<[u8; 4]> {
    let mut out = vec![[0u8; 4]; gw * gh];
    for gy in 0..gh {
        let fy = (gy as f32 + 0.5) * h as f32 / gh as f32 - 0.5;
        let y0 = fy.floor().clamp(0.0, (h - 1) as f32) as usize;
        let y1 = (y0 + 1).min(h - 1);
        let ty = (fy - y0 as f32).clamp(0.0, 1.0);
        for gx in 0..gw {
            let fx = (gx as f32 + 0.5) * w as f32 / gw as f32 - 0.5;
            let x0 = fx.floor().clamp(0.0, (w - 1) as f32) as usize;
            let x1 = (x0 + 1).min(w - 1);
            let tx = (fx - x0 as f32).clamp(0.0, 1.0);
            let px = |x: usize, y: usize| -> [f32; 4] {
                let i = (y * w + x) * 4;
                [rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32, rgba[i + 3] as f32]
            };
            let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
            let mut dst = [0u8; 4];
            for k in 0..4 {
                let v = (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty)
                    + (c[k] * (1.0 - tx) + d[k] * tx) * ty;
                dst[k] = v.round().clamp(0.0, 255.0) as u8;
            }
            out[gy * gw + gx] = dst;
        }
    }
    out
}

fn color_dist(a: [u8; 4], b: [u8; 4]) -> f32 {
    let dr = a[0] as f32 - b[0] as f32;
    let dg = a[1] as f32 - b[1] as f32;
    let db = a[2] as f32 - b[2] as f32;
    (0.299 * dr * dr + 0.587 * dg * dg + 0.114 * db * db).sqrt()
}

fn segment_foreground(grid: &[[u8; 4]], gw: usize, gh: usize) -> Result<Vec<bool>, String> {
    let has_opaque = grid.iter().any(|p| p[3] >= 128);
    if !has_opaque {
        return Err("image has no opaque foreground pixels".into());
    }
    let has_transparent = grid.iter().any(|p| p[3] < 128);
    if has_transparent {
        return Ok(grid.iter().map(|p| p[3] >= 128).collect());
    }

    // Opaque photo: border-seeded background color/edge matting.
    let mut border_coords = Vec::new();
    for x in 0..gw {
        border_coords.push((x, 0));
        if gh > 1 {
            border_coords.push((x, gh - 1));
        }
    }
    for y in 1..gh.saturating_sub(1) {
        border_coords.push((0, y));
        if gw > 1 {
            border_coords.push((gw - 1, y));
        }
    }

    // Collect up to 8 representative border background colors via farthest-first sampling.
    let mut palette = vec![grid[0]];
    for _ in 0..7 {
        let mut best_c = None;
        let mut best_d = 12.0f32;
        for &(x, y) in &border_coords {
            let p = grid[y * gw + x];
            let d = palette.iter().map(|&c| color_dist(p, c)).fold(f32::INFINITY, f32::min);
            if d > best_d {
                best_d = d;
                best_c = Some(p);
            }
        }
        match best_c {
            Some(c) => palette.push(c),
            None => break,
        }
    }

    let palette_dist = |p: [u8; 4]| -> f32 {
        palette.iter().map(|&c| color_dist(p, c)).fold(f32::INFINITY, f32::min)
    };
    let mut border_dists: Vec<f32> = border_coords
        .iter()
        .map(|&(x, y)| palette_dist(grid[y * gw + x]))
        .collect();
    border_dists.sort_by(|a, b| a.total_cmp(b));
    let spread = border_dists[(border_dists.len() * 85 / 100).min(border_dists.len() - 1)];
    let thresh = (spread * 2.4 + 22.0).clamp(18.0, 68.0);

    // Sobel gradient magnitude on luminance.
    let lum = |x: usize, y: usize| -> f32 {
        let p = grid[y * gw + x];
        0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
    };
    let mut grad = vec![0.0f32; gw * gh];
    for y in 1..gh.saturating_sub(1) {
        for x in 1..gw.saturating_sub(1) {
            let gx = -lum(x - 1, y - 1) + lum(x + 1, y - 1)
                - 2.0 * lum(x - 1, y)
                + 2.0 * lum(x + 1, y)
                - lum(x - 1, y + 1)
                + lum(x + 1, y + 1);
            let gy = -lum(x - 1, y - 1) - 2.0 * lum(x, y - 1) - lum(x + 1, y - 1)
                + lum(x - 1, y + 1)
                + 2.0 * lum(x, y + 1)
                + lum(x + 1, y + 1);
            grad[y * gw + x] = (gx * gx + gy * gy).sqrt() * 0.25;
        }
    }

    let mut visited_bg = vec![false; gw * gh];
    let mut q = VecDeque::new();
    for &(x, y) in &border_coords {
        let idx = y * gw + x;
        if palette_dist(grid[idx]) <= thresh * 1.15 && !visited_bg[idx] {
            visited_bg[idx] = true;
            q.push_back((x, y));
        }
    }

    while let Some((x, y)) = q.pop_front() {
        let cur_px = grid[y * gw + x];
        let nbrs = [
            (x > 0).then(|| (x - 1, y)),
            (x + 1 < gw).then(|| (x + 1, y)),
            (y > 0).then(|| (x, y - 1)),
            (y + 1 < gh).then(|| (x, y + 1)),
        ];
        for n in nbrs.into_iter().flatten() {
            let ni = n.1 * gw + n.0;
            if visited_bg[ni] {
                continue;
            }
            let np = grid[ni];
            let pd = palette_dist(np);
            let step = color_dist(cur_px, np);
            if pd <= thresh || (step <= 14.0 && grad[ni] <= 34.0 && pd <= thresh * 1.45) {
                visited_bg[ni] = true;
                q.push_back(n);
            }
        }
    }

    let mut fg: Vec<bool> = visited_bg.iter().map(|&b| !b).collect();
    let count = fg.iter().filter(|&&b| b).count();
    let total = gw * gh;

    // Guard if border flood consumed almost the entire image (e.g. uniform photo).
    if count < total / 40 {
        let mut max_d = 0.0f32;
        let dists: Vec<f32> = grid.iter().map(|&p| {
            let d = palette_dist(p);
            max_d = max_d.max(d);
            d
        }).collect();
        if max_d >= 12.0 {
            let cut = max_d * 0.35;
            fg = dists.iter().map(|&d| d > cut).collect();
        } else {
            // Nearly uniform photo: centered elliptic subject inset from border.
            for y in 0..gh {
                let ny = (y as f32 + 0.5) / gh as f32 * 2.0 - 1.0;
                for x in 0..gw {
                    let nx = (x as f32 + 0.5) / gw as f32 * 2.0 - 1.0;
                    fg[y * gw + x] = nx * nx + ny * ny <= 0.64;
                }
            }
        }
    }

    Ok(fg)
}

fn morphological_cleanup(fg: &mut Vec<bool>, gw: usize, gh: usize) {
    let dilate = |src: &[bool]| -> Vec<bool> {
        let mut dst = src.to_vec();
        for y in 0..gh {
            for x in 0..gw {
                if src[y * gw + x] {
                    continue;
                }
                let hit = (x > 0 && src[y * gw + x - 1])
                    || (x + 1 < gw && src[y * gw + x + 1])
                    || (y > 0 && src[(y - 1) * gw + x])
                    || (y + 1 < gh && src[(y + 1) * gw + x]);
                if hit {
                    dst[y * gw + x] = true;
                }
            }
        }
        dst
    };
    let erode = |src: &[bool]| -> Vec<bool> {
        let mut dst = vec![false; gw * gh];
        for y in 0..gh {
            for x in 0..gw {
                if !src[y * gw + x] {
                    continue;
                }
                let ok = (x == 0 || src[y * gw + x - 1])
                    && (x + 1 == gw || src[y * gw + x + 1])
                    && (y == 0 || src[(y - 1) * gw + x])
                    && (y + 1 == gh || src[(y + 1) * gw + x]);
                dst[y * gw + x] = ok;
            }
        }
        dst
    };

    // Closing (fill 1px holes), then opening (drop 1px isolated specks) if mask is sizable.
    let initial = fg.iter().filter(|&&b| b).count();
    let closed = erode(&dilate(fg));
    let opened = dilate(&erode(&closed));
    if opened.iter().filter(|&&b| b).count() >= (initial / 4).max(4) {
        *fg = opened;
    } else {
        *fg = closed;
    }

    // Keep significant connected components (>= 1/16 of largest component).
    let mut comp_id = vec![0usize; gw * gh];
    let mut sizes = vec![0usize];
    let mut q = VecDeque::new();
    for start in 0..(gw * gh) {
        if !fg[start] || comp_id[start] != 0 {
            continue;
        }
        let cid = sizes.len();
        comp_id[start] = cid;
        let mut sz = 0usize;
        q.push_back((start % gw, start / gw));
        while let Some((x, y)) = q.pop_front() {
            sz += 1;
            for (nx, ny) in [
                (x > 0).then(|| (x - 1, y)),
                (x + 1 < gw).then(|| (x + 1, y)),
                (y > 0).then(|| (x, y - 1)),
                (y + 1 < gh).then(|| (x, y + 1)),
            ]
            .into_iter()
            .flatten()
            {
                let ni = ny * gw + nx;
                if fg[ni] && comp_id[ni] == 0 {
                    comp_id[ni] = cid;
                    q.push_back((nx, ny));
                }
            }
        }
        sizes.push(sz);
    }
    let max_sz = sizes.iter().copied().max().unwrap_or(0);
    let min_keep = (max_sz / 16).max(4);
    for (i, &cid) in comp_id.iter().enumerate() {
        if cid > 0 && sizes[cid] < min_keep {
            fg[i] = false;
        }
    }

    // Condition mask for strict 2-manifoldness:
    // Resolve 2x2 diagonal checkerboard pinches so every vertex neighborhood is
    // 4-connected and every foreground boundary is a set of simple closed loops.
    for _ in 0..8 {
        let mut changed = false;
        let mut in_2x2 = vec![false; gw * gh];
        for y in 0..gh.saturating_sub(1) {
            for x in 0..gw.saturating_sub(1) {
                if fg[y * gw + x]
                    && fg[y * gw + x + 1]
                    && fg[(y + 1) * gw + x]
                    && fg[(y + 1) * gw + x + 1]
                {
                    in_2x2[y * gw + x] = true;
                    in_2x2[y * gw + x + 1] = true;
                    in_2x2[(y + 1) * gw + x] = true;
                    in_2x2[(y + 1) * gw + x + 1] = true;
                }
            }
        }
        for i in 0..(gw * gh) {
            if fg[i] && !in_2x2[i] {
                fg[i] = false;
                changed = true;
            }
        }
        for y in 0..gh.saturating_sub(1) {
            for x in 0..gw.saturating_sub(1) {
                let a = fg[y * gw + x];
                let b = fg[y * gw + x + 1];
                let c = fg[(y + 1) * gw + x];
                let d = fg[(y + 1) * gw + x + 1];
                if a && d && !b && !c {
                    fg[y * gw + x + 1] = true;
                    fg[(y + 1) * gw + x] = true;
                    changed = true;
                } else if !a && !d && b && c {
                    fg[y * gw + x] = true;
                    fg[(y + 1) * gw + x + 1] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Exact 1D squared Euclidean distance transform (Felzenszwalb-Huttenlocher).
fn edt_1d(f: &[f32], d: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f32; n + 1];
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;

    for q in 1..n {
        let mut s;
        loop {
            let r = v[k];
            let num = (f[q] + (q * q) as f32) - (f[r] + (r * r) as f32);
            let den = 2.0 * (q as f32 - r as f32);
            s = num / den;
            if s <= z[k] {
                if k == 0 {
                    break;
                }
                k -= 1;
            } else {
                break;
            }
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::INFINITY;
    }

    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let dq = q as f32 - v[k] as f32;
        d[q] = dq * dq + f[v[k]];
    }
}

/// Exact 2D Euclidean distance from each foreground cell to the nearest background
/// (including 1-cell margin outside the image bounds).
fn distance_transform(fg: &[bool], gw: usize, gh: usize) -> Vec<f32> {
    let pw = gw + 2;
    let ph = gh + 2;
    const INF: f32 = 1e12;
    let mut buf = vec![0.0f32; pw * ph];
    for y in 0..gh {
        for x in 0..gw {
            if fg[y * gw + x] {
                buf[(y + 1) * pw + (x + 1)] = INF;
            }
        }
    }

    // Columns
    let mut col_in = vec![0.0f32; ph];
    let mut col_out = vec![0.0f32; ph];
    for x in 0..pw {
        for y in 0..ph {
            col_in[y] = buf[y * pw + x];
        }
        edt_1d(&col_in, &mut col_out);
        for y in 0..ph {
            buf[y * pw + x] = col_out[y];
        }
    }

    // Rows
    let mut row_out = vec![0.0f32; pw];
    for y in 0..ph {
        edt_1d(&buf[y * pw..(y + 1) * pw], &mut row_out);
        buf[y * pw..(y + 1) * pw].copy_from_slice(&row_out);
    }

    let mut out = vec![0.0f32; gw * gh];
    for y in 0..gh {
        for x in 0..gw {
            out[y * gw + x] = buf[(y + 1) * pw + (x + 1)].max(0.0).sqrt();
        }
    }
    out
}

fn smooth_masked(field: &mut [f32], fg: &[bool], gw: usize, gh: usize, passes: usize) {
    let mut tmp = field.to_vec();
    for _ in 0..passes {
        for y in 0..gh {
            for x in 0..gw {
                let i = y * gw + x;
                if !fg[i] {
                    tmp[i] = 0.0;
                    continue;
                }
                let mut sum = field[i] * 2.0;
                let mut w = 2.0;
                for (nx, ny) in [
                    (x > 0).then(|| (x - 1, y)),
                    (x + 1 < gw).then(|| (x + 1, y)),
                    (y > 0).then(|| (x, y - 1)),
                    (y + 1 < gh).then(|| (x, y + 1)),
                ]
                .into_iter()
                .flatten()
                {
                    let ni = ny * gw + nx;
                    if fg[ni] {
                        sum += field[ni];
                        w += 1.0;
                    } else {
                        w += 1.0; // zero outside boundary
                    }
                }
                tmp[i] = sum / w;
            }
        }
        field.copy_from_slice(&tmp);
    }
}

/// Smooth the boundary loop of `(vx, vz)` so pixelated staircases become smooth curves.
fn smooth_boundary_contour(
    fg: &[bool],
    gw: usize,
    gh: usize,
    is_boundary: &[bool],
    vx_pos: &mut [f32],
    vz_pos: &mut [f32],
) {
    let vw = gw + 1;
    let vh = gh + 1;
    let cidx = |x: usize, y: usize| y * vw + x;
    let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
    for cy in 0..gh {
        for cx in 0..gw {
            if !fg[cy * gw + cx] {
                continue;
            }
            let edges = [
                (cx == 0 || !fg[cy * gw + cx - 1], cidx(cx, cy), cidx(cx, cy + 1)),
                (cx + 1 == gw || !fg[cy * gw + cx + 1], cidx(cx + 1, cy), cidx(cx + 1, cy + 1)),
                (cy == 0 || !fg[(cy - 1) * gw + cx], cidx(cx, cy), cidx(cx + 1, cy)),
                (cy + 1 == gh || !fg[(cy + 1) * gw + cx], cidx(cx, cy + 1), cidx(cx + 1, cy + 1)),
            ];
            for (rim, a, b) in edges {
                if rim {
                    adj.entry(a).or_default().push(b);
                    adj.entry(b).or_default().push(a);
                }
            }
        }
    }

    for lambda in [0.45f32, -0.32f32, 0.45f32, -0.32f32] {
        let old_x = vx_pos.to_vec();
        let old_z = vz_pos.to_vec();
        for (&vi, nbrs) in &adj {
            if !is_boundary[vi] || nbrs.len() != 2 {
                continue;
            }
            let avg_x = 0.5 * (old_x[nbrs[0]] + old_x[nbrs[1]]);
            let avg_z = 0.5 * (old_z[nbrs[0]] + old_z[nbrs[1]]);
            vx_pos[vi] = old_x[vi] + lambda * (avg_x - old_x[vi]);
            vz_pos[vi] = old_z[vi] + lambda * (avg_z - old_z[vi]);
        }
    }
    let _ = vh;
}

/// Deterministic seeded 2D value noise for subtle relief variation across takes.
fn seeded_relief_modulation(u: f32, v: f32, seed: u32) -> f32 {
    let hash2 = |ix: i32, iy: i32, s: u32| -> f32 {
        let mut h = (ix as u32).wrapping_mul(0x27d4_eb2d)
            ^ (iy as u32).wrapping_mul(0x1656_67b1)
            ^ s.wrapping_mul(0x9e37_79b9);
        h ^= h >> 15;
        h = h.wrapping_mul(0x85eb_ca6b);
        h ^= h >> 13;
        ((h & 0xffff) as f32 / 32767.5) - 1.0
    };
    let noise = |x: f32, y: f32, s: u32| -> f32 {
        let ix = x.floor() as i32;
        let iy = y.floor() as i32;
        let fx = x - ix as f32;
        let fy = y - iy as f32;
        let sx = fx * fx * (3.0 - 2.0 * fx);
        let sy = fy * fy * (3.0 - 2.0 * fy);
        let a = hash2(ix, iy, s);
        let b = hash2(ix + 1, iy, s);
        let c = hash2(ix, iy + 1, s);
        let d = hash2(ix + 1, iy + 1, s);
        (a * (1.0 - sx) + b * sx) * (1.0 - sy) + (c * (1.0 - sx) + d * sx) * sy
    };
    let n1 = noise(u * 3.5, v * 3.5, seed);
    let n2 = noise(u * 7.0 + 13.0, v * 7.0 + 7.0, seed.wrapping_add(101));
    0.10 * n1 + 0.05 * n2
}

fn build_bleed_texture(
    w: usize,
    h: usize,
    rgba: &[u8],
    fg: &[bool],
    gw: usize,
    gh: usize,
) -> Image {
    let mut pixels = vec![[0.0f32; 4]; w * h];
    let mut valid = vec![false; w * h];
    for y in 0..h {
        let gy = (y * gh / h).min(gh - 1);
        for x in 0..w {
            let gx = (x * gw / w).min(gw - 1);
            let i = (y * w + x) * 4;
            let linear = [
                srgb_to_linear(rgba[i] as f32 / 255.0),
                srgb_to_linear(rgba[i + 1] as f32 / 255.0),
                srgb_to_linear(rgba[i + 2] as f32 / 255.0),
                1.0,
            ];
            let pi = y * w + x;
            pixels[pi] = linear;
            valid[pi] = fg[gy * gw + gx] && rgba[i + 3] >= 64;
        }
    }

    // Dilate foreground colors outward into masked background pixels to avoid edge bleeding.
    for _ in 0..6 {
        let prev_px = pixels.clone();
        let prev_ok = valid.clone();
        let mut any = false;
        for y in 0..h {
            for x in 0..w {
                let pi = y * w + x;
                if prev_ok[pi] {
                    continue;
                }
                let mut sum = [0.0f32; 3];
                let mut cnt = 0.0f32;
                for (nx, ny) in [
                    (x > 0).then(|| (x - 1, y)),
                    (x + 1 < w).then(|| (x + 1, y)),
                    (y > 0).then(|| (x, y - 1)),
                    (y + 1 < h).then(|| (x, y + 1)),
                ]
                .into_iter()
                .flatten()
                {
                    let ni = ny * w + nx;
                    if prev_ok[ni] {
                        for c in 0..3 {
                            sum[c] += prev_px[ni][c];
                        }
                        cnt += 1.0;
                    }
                }
                if cnt > 0.0 {
                    pixels[pi] = [sum[0] / cnt, sum[1] / cnt, sum[2] / cnt, 1.0];
                    valid[pi] = true;
                    any = true;
                }
            }
        }
        if !any {
            break;
        }
    }

    Image {
        name: "PhotoAlbedo".into(),
        width: w as u32,
        height: h as u32,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io;

    fn test_subject_rgba(w: u32, h: u32, transparent_bg: bool) -> Vec<u8> {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.5;
        let r = w.min(h) as f32 * 0.34;
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= r {
                    let shade = (1.0 - dist / r).clamp(0.0, 1.0);
                    rgba[i] = (140.0 + 100.0 * shade) as u8;
                    rgba[i + 1] = (70.0 + 80.0 * shade) as u8;
                    rgba[i + 2] = (50.0 + 40.0 * shade) as u8;
                    rgba[i + 3] = 255;
                } else {
                    // Light gray-blue studio backdrop or transparent.
                    rgba[i] = 225;
                    rgba[i + 1] = 232;
                    rgba[i + 2] = 240;
                    rgba[i + 3] = if transparent_bg { 0 } else { 255 };
                }
            }
        }
        rgba
    }

    #[test]
    fn photo_to_mesh_produces_watertight_textured_upright_glb_and_stl() {
        let (w, h) = (64, 64);
        let rgba = test_subject_rgba(w, h, false);
        let res = photo_to_mesh(
            w,
            h,
            &rgba,
            PhotoMeshOptions {
                quality: PhotoMeshQuality::Standard,
                take_seed: 42,
                thickness_ratio: 0.3,
                real_height_mm: Some(80.0),
            },
        )
        .unwrap();

        res.mesh.validate().unwrap();
        assert!(res.mesh.has_uvs(), "UVs must be populated");
        assert!(!res.mesh.color_attributes.is_empty(), "vertex colors must be populated");
        assert!(res.foreground_coverage > 0.2 && res.foreground_coverage < 0.6);

        let rep = print_prep::inspect_manifold(&res.mesh);
        assert!(rep.is_watertight, "generated mesh must be watertight: {rep:?}");
        assert_eq!(rep.boundary_edges, 0);
        assert_eq!(rep.non_manifold_edges, 0);
        assert_eq!(rep.non_manifold_vertices, 0);

        let b_mm = print_prep::mesh_bounds_mm(&res.mesh);
        assert!((b_mm.height - 80.0).abs() < 0.5, "real height mm: {b_mm:?}");
        assert!(res.mesh.bounds().min.z.abs() < 1e-4, "upright bed min Z");
        assert!(print_prep::volume_cm3(&res.mesh) > 1.0);

        // Exports directly to GLB, OBJ, and binary STL.
        let scene = res.into_scene("PhotoSubject");
        let glb = io::gltf::export(&scene, true).expect("GLB export");
        assert!(glb.starts_with(b"glTF"));
        let stl = io::stl::export_binary(&scene);
        assert!(io::stl::is_binary(&stl));
        let obj = io::obj::export_files(&scene, "photo");
        assert!(obj.len() >= 3); // .obj, .mtl, texture .png
    }

    #[test]
    fn photo_to_mesh_quality_levels_and_seed_determinism() {
        let (w, h) = (64, 64);
        let rgba = test_subject_rgba(w, h, true);
        let draft = photo_to_mesh(
            w,
            h,
            &rgba,
            PhotoMeshOptions { quality: PhotoMeshQuality::Draft, take_seed: 1, ..Default::default() },
        )
        .unwrap();
        let high = photo_to_mesh(
            w,
            h,
            &rgba,
            PhotoMeshOptions { quality: PhotoMeshQuality::High, take_seed: 1, ..Default::default() },
        )
        .unwrap();
        assert!(high.mesh.face_count() > draft.mesh.face_count());

        let seed_a = photo_to_mesh(
            w,
            h,
            &rgba,
            PhotoMeshOptions { quality: PhotoMeshQuality::Draft, take_seed: 7, ..Default::default() },
        )
        .unwrap();
        let seed_a2 = photo_to_mesh(
            w,
            h,
            &rgba,
            PhotoMeshOptions { quality: PhotoMeshQuality::Draft, take_seed: 7, ..Default::default() },
        )
        .unwrap();
        let seed_b = photo_to_mesh(
            w,
            h,
            &rgba,
            PhotoMeshOptions { quality: PhotoMeshQuality::Draft, take_seed: 99, ..Default::default() },
        )
        .unwrap();
        assert_eq!(seed_a.mesh.positions, seed_a2.mesh.positions);
        assert_ne!(seed_a.mesh.positions, seed_b.mesh.positions);
    }

    #[test]
    fn photo_to_mesh_rejects_empty_alpha_and_bad_buffers() {
        assert!(photo_to_mesh(0, 10, &[], Default::default()).is_err());
        assert!(photo_to_mesh(4, 4, &[0u8; 12], Default::default()).is_err());
        assert!(photo_to_mesh(8, 8, &vec![0u8; 8 * 8 * 4], Default::default()).is_err());
    }
}
