// SPDX-License-Identifier: GPL-3.0-only
//! The compositor: evaluates a `CompositorNodeTree` (Blender's compositing
//! nodes) on float RGBA buffers.
//!
//! Data passed between nodes is either a single value or an image
//! ([`Val`]). Floats are stored replicated in R, G and B with alpha 1,
//! vectors in R, G, B with alpha 0, so converting between socket types is a
//! cheap per-pixel step ([`convert`]). A node whose inputs are all single
//! values computes a single value; otherwise it works on the *domain* of its
//! first image input (Blender's operation domain), and other images are
//! realized onto that domain centred, like Blender's realtime compositor.
//! Scale and Crop (with "Crop Image Size") change the canvas size; Rotate,
//! Translate, Transform, Flip and the distortions keep it.
//!
//! Pixel loops run row-parallel on std threads ([`par_rows`]). Muted nodes
//! pass their first input of each output's type through; group nodes
//! evaluate their node group with the group node's inputs.
//!
//! Buffers are stored row-major from the top-left like [`Image`]; node
//! settings that use Blender's bottom-left pixel space (mask positions,
//! translations, crop rectangles) are converted where they are read.
use crate::nodes::{Node, NodeTree, Prop, SocketType, Value};
use crate::render::{Pass, RenderResult};
use crate::scene::{Image, Scene};
use std::collections::HashMap;
use std::sync::Arc;

mod filter;
mod glare;
mod ops;
mod transform;

pub type Px = [f32; 4];

/// An image buffer: `w * h` RGBA pixels, row-major from the top-left.
#[derive(Clone, Debug, PartialEq)]
pub struct Buf {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Px>,
}

impl Buf {
    pub fn new(w: usize, h: usize, fill: Px) -> Buf {
        Buf { w, h, px: vec![fill; w * h] }
    }

    pub fn from_image(img: &Image) -> Buf {
        Buf { w: img.width as usize, h: img.height as usize, px: img.pixels.clone() }
    }

    pub fn to_image(&self, name: &str) -> Image {
        Image { name: name.into(), width: self.w as u32, height: self.h as u32, pixels: self.px.clone() }
    }

    /// The pixel at (x, y) from the top-left, zero outside.
    #[inline]
    pub fn get(&self, x: isize, y: isize) -> Px {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            [0.0; 4]
        } else {
            self.px[y as usize * self.w + x as usize]
        }
    }

    /// The pixel at (x, y) clamped to the edges.
    #[inline]
    pub fn clamped(&self, x: isize, y: isize) -> Px {
        if self.w == 0 || self.h == 0 {
            return [0.0; 4];
        }
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.px[y * self.w + x]
    }

    /// Bilinear sample at continuous pixel coordinates (pixel centres at
    /// `i + 0.5`), zero outside the image.
    pub fn bilinear(&self, x: f32, y: f32) -> Px {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let a = self.get(x0, y0);
        let b = self.get(x0 + 1, y0);
        let c = self.get(x0, y0 + 1);
        let d = self.get(x0 + 1, y0 + 1);
        let mut o = [0.0; 4];
        for k in 0..4 {
            o[k] = (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty;
        }
        o
    }

    /// Bilinear sample clamped to the edges.
    pub fn bilinear_clamped(&self, x: f32, y: f32) -> Px {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let a = self.clamped(x0, y0);
        let b = self.clamped(x0 + 1, y0);
        let c = self.clamped(x0, y0 + 1);
        let d = self.clamped(x0 + 1, y0 + 1);
        let mut o = [0.0; 4];
        for k in 0..4 {
            o[k] = (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty;
        }
        o
    }

    /// Nearest-pixel sample at continuous coordinates, zero outside.
    pub fn nearest(&self, x: f32, y: f32) -> Px {
        self.get(x.floor() as isize, y.floor() as isize)
    }

    /// Bicubic (Catmull-Rom) sample, zero outside.
    pub fn bicubic(&self, x: f32, y: f32) -> Px {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let w = |t: f32| {
            let t2 = t * t;
            let t3 = t2 * t;
            [-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2]
        };
        let (wx, wy) = (w(tx), w(ty));
        let mut o = [0.0; 4];
        for j in 0..4 {
            for i in 0..4 {
                let p = self.get(x0 as isize - 1 + i as isize, y0 as isize - 1 + j as isize);
                let k = wx[i] * wy[j];
                for c in 0..4 {
                    o[c] += p[c] * k;
                }
            }
        }
        o
    }

    /// Sample with a named interpolation ("NEAREST", "BILINEAR", "BICUBIC").
    pub fn sample(&self, x: f32, y: f32, interp: Interp) -> Px {
        match interp {
            Interp::Nearest => self.nearest(x, y),
            Interp::Bilinear => self.bilinear(x, y),
            Interp::Bicubic => self.bicubic(x, y),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Nearest,
    Bilinear,
    Bicubic,
}

impl Interp {
    pub fn from_name(s: &str) -> Interp {
        match s.to_ascii_uppercase().as_str() {
            "NEAREST" => Interp::Nearest,
            "BICUBIC" => Interp::Bicubic,
            _ => Interp::Bilinear,
        }
    }
}

/// A value passed along a link: one value for the whole image, or an image.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Single(Px),
    Image(Arc<Buf>),
}

impl Val {
    pub fn single(&self) -> Option<Px> {
        match self {
            Val::Single(p) => Some(*p),
            Val::Image(_) => None,
        }
    }
    pub fn image(&self) -> Option<&Buf> {
        match self {
            Val::Image(b) => Some(b),
            Val::Single(_) => None,
        }
    }
    pub fn size(&self) -> Option<(usize, usize)> {
        self.image().map(|b| (b.w, b.h))
    }
    pub fn float(f: f32) -> Val {
        Val::Single([f, f, f, 1.0])
    }
    pub fn img(b: Buf) -> Val {
        Val::Image(Arc::new(b))
    }
    /// The value at pixel (x, y) of `dom` (images must already be realized).
    #[inline]
    pub fn at(&self, i: usize) -> Px {
        match self {
            Val::Single(p) => *p,
            Val::Image(b) => b.px[i],
        }
    }
    /// The single value, or the average of the image (used where Blender
    /// expects a single value but an image is linked).
    pub fn as_single(&self) -> Px {
        match self {
            Val::Single(p) => *p,
            Val::Image(b) => {
                let n = b.px.len().max(1) as f32;
                let mut s = [0.0; 4];
                for p in &b.px {
                    for k in 0..4 {
                        s[k] += p[k];
                    }
                }
                s.map(|v| v / n)
            }
        }
    }
    /// The image realized onto a `w` x `h` domain (singles fill it).
    pub fn realize(&self, w: usize, h: usize) -> Arc<Buf> {
        match self {
            Val::Single(p) => Arc::new(Buf::new(w, h, *p)),
            Val::Image(b) if b.w == w && b.h == h => b.clone(),
            Val::Image(b) => Arc::new(realize(b, w, h)),
        }
    }
}

/// Place `b` centred on a `w` x `h` canvas (nearest pixels, transparent
/// outside), as Blender realizes an input on another operation domain.
pub fn realize(b: &Buf, w: usize, h: usize) -> Buf {
    let ox = (w as isize - b.w as isize) / 2;
    let oy = (h as isize - b.h as isize) / 2;
    let mut out = Buf::new(w, h, [0.0; 4]);
    for y in 0..h {
        for x in 0..w {
            out.px[y * w + x] = b.get(x as isize - ox, y as isize - oy);
        }
    }
    out
}

/// Convert a pixel between socket types (see the module docs).
#[inline]
pub fn convert_px(p: Px, from: SocketType, to: SocketType) -> Px {
    use SocketType as T;
    let kind = |t: T| match t {
        T::Float | T::Int | T::Bool => 0,
        T::Vector | T::Rotation => 1,
        _ => 2,
    };
    match (kind(from), kind(to)) {
        (a, b) if a == b => p,
        (0, 1) => [p[0], p[0], p[0], 0.0],
        (0, _) => [p[0], p[0], p[0], 1.0],
        (_, 0) => {
            let f = (p[0] + p[1] + p[2]) / 3.0;
            [f, f, f, 1.0]
        }
        (2, 1) => [p[0], p[1], p[2], 0.0],
        _ => [p[0], p[1], p[2], 1.0],
    }
}

pub fn convert(v: Val, from: SocketType, to: SocketType) -> Val {
    use SocketType as T;
    let same = |t: T| match t {
        T::Float | T::Int | T::Bool => 0,
        T::Vector | T::Rotation => 1,
        _ => 2,
    };
    if same(from) == same(to) {
        return v;
    }
    match v {
        Val::Single(p) => Val::Single(convert_px(p, from, to)),
        Val::Image(b) => Val::img(Buf { w: b.w, h: b.h, px: b.px.iter().map(|p| convert_px(*p, from, to)).collect() }),
    }
}

/// A socket's own value as a pixel of its type.
pub fn socket_px(v: &Value, ty: SocketType) -> Px {
    match ty {
        SocketType::Vector | SocketType::Rotation => {
            let a = v.as_vector();
            [a[0], a[1], a[2], 0.0]
        }
        SocketType::Color => v.as_color(),
        _ => {
            let f = v.as_float();
            [f, f, f, 1.0]
        }
    }
}

/// Camera data the Defocus node needs (from the scene's active camera).
#[derive(Clone, Copy, Debug)]
pub struct CameraInfo {
    /// Focal length in millimetres.
    pub focal_mm: f32,
    /// Sensor width in millimetres.
    pub sensor_mm: f32,
    /// Focus distance in metres.
    pub focus_m: f32,
}

/// What the compositor reads from the outside world.
pub struct Context<'a> {
    pub scene: &'a Scene,
    /// The render whose layers feed Render Layers nodes.
    pub render: Option<&'a RenderResult>,
    /// Scene render resolution (the Composite output size).
    pub render_size: (usize, usize),
    /// Render resolution percentage / 100 (Scale node "Scene Size").
    pub render_scale: f32,
    pub frame: i32,
    pub fps: f32,
    /// Worker threads (0 = all cores).
    pub threads: usize,
    pub camera: Option<CameraInfo>,
}

impl<'a> Context<'a> {
    /// A context for `scene` at its current frame with `render` (whose size
    /// becomes the render size when given).
    pub fn new(scene: &'a Scene, render: Option<&'a RenderResult>, size: (usize, usize)) -> Context<'a> {
        let size = render.map(|r| (r.color.width as usize, r.color.height as usize)).unwrap_or(size);
        let camera = scene.active_camera.and_then(|i| match &scene.objects.get(i)?.data {
            crate::scene::ObjectData::Camera(c) => {
                let aspect = size.0 as f32 / size.1.max(1) as f32;
                let focus = match c.dof.focus_object {
                    Some(o) if o < scene.objects.len() => {
                        let a = scene.world_matrix(i).transform_point(crate::math::Vec3::ZERO);
                        let b = scene.world_matrix(o).transform_point(crate::math::Vec3::ZERO);
                        (a - b).length()
                    }
                    _ => c.dof.focus_distance,
                };
                Some(CameraInfo { focal_mm: c.focal_length(aspect), sensor_mm: c.sensor_width, focus_m: focus })
            }
            _ => None,
        });
        Context { scene, render, render_size: size, render_scale: 1.0, frame: scene.frame_current, fps: scene.fps, threads: 0, camera }
    }
}

/// What a compositor run produced.
#[derive(Clone, Debug, Default)]
pub struct CompositeOutput {
    /// The active Composite node's image (render size).
    pub composite: Option<Image>,
    /// The active Viewer node's image ("Viewer Node").
    pub viewer: Option<Image>,
    /// File Output images: (path without extension, image).
    pub files: Vec<(String, Image)>,
}

/// Evaluate `tree` (a compositor tree) in `ctx`.
pub fn composite(tree: &NodeTree, ctx: &Context) -> CompositeOutput {
    let mut out = CompositeOutput::default();
    let mut ev = Eval { ctx, out: &mut out, depth: 0 };
    ev.run(tree, None);
    out
}

struct Eval<'a, 'b> {
    ctx: &'a Context<'a>,
    out: &'b mut CompositeOutput,
    depth: usize,
}

/// The node an output-type idname is active for: the tree's active node of
/// that type, else the first unmuted one.
fn active_output(tree: &NodeTree, idname: &str) -> Option<usize> {
    if let Some(a) = tree.active {
        if tree.nodes.get(a).is_some_and(|n| n.idname == idname && !n.mute) {
            return Some(a);
        }
    }
    tree.nodes.iter().position(|n| n.idname == idname && !n.mute)
}

impl<'a, 'b> Eval<'a, 'b> {
    /// Evaluate a tree. For a group, `group_in` holds the group node's input
    /// values and the Group Output's input values are returned.
    fn run(&mut self, tree: &NodeTree, group_in: Option<&[Val]>) -> Vec<Val> {
        let mut memo: HashMap<usize, Vec<Val>> = HashMap::new();
        let top = group_in.is_none();
        let mut result = Vec::new();
        if top {
            if let Some(c) = active_output(tree, "CompositorNodeComposite") {
                let ins = self.inputs(tree, c, &mut memo, group_in);
                let (rw, rh) = self.ctx.render_size;
                let img = ops::output_image(&tree.nodes[c], &ins, Some((rw.max(1), rh.max(1))));
                self.out.composite = Some(img.to_image("Render Result"));
            }
            if let Some(v) = active_output(tree, "CompositorNodeViewer") {
                let ins = self.inputs(tree, v, &mut memo, group_in);
                let (rw, rh) = self.ctx.render_size;
                let dom = if ins[0].image().is_some() { None } else { Some((rw.max(1), rh.max(1))) };
                let img = ops::output_image(&tree.nodes[v], &ins, dom);
                self.out.viewer = Some(img.to_image("Viewer Node"));
            }
            for (i, n) in tree.nodes.iter().enumerate() {
                if n.idname == "CompositorNodeOutputFile" && !n.mute {
                    let ins = self.inputs(tree, i, &mut memo, group_in);
                    let base = match n.prop("base_path") {
                        Some(Prop::String(s)) => s.clone(),
                        _ => String::new(),
                    };
                    let slots: Vec<String> = match n.prop("file_slots") {
                        Some(Prop::String(s)) => s.lines().map(str::to_string).collect(),
                        _ => Vec::new(),
                    };
                    for (k, s) in n.inputs.iter().enumerate() {
                        if tree.input_link(i, k).is_none() {
                            continue;
                        }
                        let slot = slots.get(k).cloned().unwrap_or_else(|| s.name.clone());
                        let (rw, rh) = self.ctx.render_size;
                        let b = ins[k].realize_or(rw.max(1), rh.max(1));
                        out_file(self.out, &base, &slot, self.ctx.frame, &b);
                    }
                }
            }
        } else if let Some(go) = tree.nodes.iter().position(|n| n.idname == "NodeGroupOutput") {
            result = self.inputs(tree, go, &mut memo, group_in);
        }
        result
    }

    /// The values of `node`'s inputs, converted to the input socket types.
    fn inputs(&mut self, tree: &NodeTree, node: usize, memo: &mut HashMap<usize, Vec<Val>>, group_in: Option<&[Val]>) -> Vec<Val> {
        let n = &tree.nodes[node];
        let mut v = Vec::with_capacity(n.inputs.len());
        for (k, s) in n.inputs.iter().enumerate() {
            let link = tree.input_link(node, k).filter(|l| !l.muted);
            let val = match link {
                Some(l) if l.from_node < tree.nodes.len() => {
                    let from_ty = tree.nodes[l.from_node].outputs.get(l.from_socket).map(|o| o.ty).unwrap_or(SocketType::Color);
                    let outs = self.node_outputs(tree, l.from_node, memo, group_in);
                    let src = outs.get(l.from_socket).cloned().unwrap_or(Val::Single([0.0; 4]));
                    convert(src, from_ty, s.ty)
                }
                _ => Val::Single(socket_px(&s.value, s.ty)),
            };
            v.push(val);
        }
        v
    }

    fn node_outputs(&mut self, tree: &NodeTree, node: usize, memo: &mut HashMap<usize, Vec<Val>>, group_in: Option<&[Val]>) -> Vec<Val> {
        if let Some(v) = memo.get(&node) {
            return v.clone();
        }
        // Guard against cycles: a node being evaluated reads zeros.
        memo.insert(node, tree.nodes[node].outputs.iter().map(|_| Val::Single([0.0; 4])).collect());
        let ins = self.inputs(tree, node, memo, group_in);
        let n = &tree.nodes[node];
        let outs = if n.mute {
            muted(n, &ins)
        } else if n.idname == "NodeReroute" {
            vec![ins.first().cloned().unwrap_or(Val::Single([0.0; 4]))]
        } else if n.idname == "NodeGroupInput" {
            let g = group_in.unwrap_or(&[]);
            n.outputs.iter().enumerate().map(|(k, s)| g.get(k).cloned().unwrap_or(Val::Single(socket_px(&s.value, s.ty)))).collect()
        } else if crate::nodes::edit::is_group_node(n) {
            self.group(n, &ins)
        } else {
            let mut o = ops::eval_node(self.ctx, n, &ins);
            o.resize(n.outputs.len(), Val::Single([0.0; 4]));
            // Outputs carry their socket's type.
            o
        };
        memo.insert(node, outs.clone());
        outs
    }

    fn group(&mut self, n: &Node, ins: &[Val]) -> Vec<Val> {
        let Some(g) = crate::nodes::edit::group_of(n) else {
            return n.outputs.iter().map(|_| Val::Single([0.0; 4])).collect();
        };
        let Some(tree) = self.ctx.scene.node_groups.get(g) else {
            return n.outputs.iter().map(|_| Val::Single([0.0; 4])).collect();
        };
        if self.depth > 32 {
            return n.outputs.iter().map(|_| Val::Single([0.0; 4])).collect();
        }
        self.depth += 1;
        let r = self.run(tree, Some(ins));
        self.depth -= 1;
        // Convert from the group output sockets' types to the node's.
        let go = tree.nodes.iter().find(|x| x.idname == "NodeGroupOutput");
        n.outputs
            .iter()
            .enumerate()
            .map(|(k, s)| {
                let v = r.get(k).cloned().unwrap_or(Val::Single(socket_px(&s.value, s.ty)));
                let from = go.and_then(|g| g.inputs.get(k)).map(|x| x.ty).unwrap_or(s.ty);
                convert(v, from, s.ty)
            })
            .collect()
    }
}

impl Val {
    fn realize_or(&self, w: usize, h: usize) -> Buf {
        match self {
            Val::Image(b) => (**b).clone(),
            Val::Single(p) => Buf::new(w, h, *p),
        }
    }
}

fn out_file(out: &mut CompositeOutput, base: &str, slot: &str, frame: i32, b: &Buf) {
    let mut path = String::from(base);
    if !path.is_empty() && !path.ends_with('/') {
        path.push('/');
    }
    path.push_str(slot);
    path.push_str(&format!("{frame:04}"));
    out.files.push((path, b.to_image(slot)));
}

/// A muted node's outputs: each output takes the first input of the same
/// type (any data input otherwise), as Blender's internal links.
fn muted(n: &Node, ins: &[Val]) -> Vec<Val> {
    n.outputs
        .iter()
        .map(|o| {
            let same = n.inputs.iter().position(|i| i.ty == o.ty);
            let any = n.inputs.iter().position(|i| matches!(i.ty, SocketType::Color | SocketType::Float | SocketType::Vector));
            match same.or(any) {
                Some(k) => convert(ins[k].clone(), n.inputs[k].ty, o.ty),
                None => Val::Single(socket_px(&o.value, o.ty)),
            }
        })
        .collect()
}

/// Run `f(y, row)` over the rows of a `w`-wide buffer in parallel.
pub fn par_rows<T: Send>(data: &mut [T], w: usize, threads: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    if w == 0 || data.is_empty() {
        return;
    }
    let rows = data.len() / w;
    let threads = crate::render::resolve_threads(threads).min(rows.max(1));
    if threads <= 1 || data.len() < 4096 {
        for (y, row) in data.chunks_mut(w).enumerate() {
            f(y, row);
        }
        return;
    }
    let per = rows.div_ceil(threads * 4).max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let chunks: Vec<std::sync::Mutex<Option<(usize, &mut [T])>>> = data.chunks_mut(w * per).enumerate().map(|(i, c)| std::sync::Mutex::new(Some((i * per, c)))).collect();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(slot) = chunks.get(i) else { break };
                let Some((y0, chunk)) = slot.lock().unwrap().take() else { continue };
                for (k, row) in chunk.chunks_mut(w).enumerate() {
                    f(y0 + k, row);
                }
            });
        }
    });
}

/// Build a `w` x `h` buffer from `f(x, y)` (top-left coordinates) in parallel.
pub fn gen(w: usize, h: usize, threads: usize, f: impl Fn(usize, usize) -> Px + Sync) -> Buf {
    let mut b = Buf::new(w, h, [0.0; 4]);
    par_rows(&mut b.px, w, threads, |y, row| {
        for (x, p) in row.iter_mut().enumerate() {
            *p = f(x, y);
        }
    });
    b
}

/// The domain of an operation: the size of the first image among `ins`
/// in `priority` order (all inputs when `priority` is empty).
pub fn domain(ins: &[Val], priority: &[usize]) -> Option<(usize, usize)> {
    if priority.is_empty() {
        ins.iter().find_map(|v| v.size())
    } else {
        priority.iter().find_map(|&i| ins.get(i).and_then(|v| v.size()))
    }
}

/// Per-pixel operation with `nout` outputs: `f(inputs, outputs)`. Single
/// values in, single values out; otherwise on the first image's domain.
pub fn pixelwise(threads: usize, ins: &[Val], nout: usize, f: impl Fn(&[Px], &mut [Px]) + Sync) -> Vec<Val> {
    let Some((w, h)) = domain(ins, &[]) else {
        let a: Vec<Px> = ins.iter().map(|v| v.single().unwrap()).collect();
        let mut o = vec![[0.0; 4]; nout];
        f(&a, &mut o);
        return o.into_iter().map(Val::Single).collect();
    };
    let real: Vec<Val> = ins.iter().map(|v| if v.image().is_some() { Val::Image(v.realize(w, h)) } else { v.clone() }).collect();
    let mut data = vec![[0.0f32; 4]; w * h * nout];
    par_rows(&mut data, w * nout, threads, |y, row| {
        let mut a = vec![[0.0; 4]; real.len()];
        for x in 0..w {
            let i = y * w + x;
            for (k, v) in real.iter().enumerate() {
                a[k] = v.at(i);
            }
            f(&a, &mut row[x * nout..x * nout + nout]);
        }
    });
    (0..nout).map(|k| Val::img(Buf { w, h, px: data.iter().skip(k).step_by(nout).copied().collect() })).collect()
}

/// Render pass output of a render result as a buffer.
pub(crate) fn pass_buf(r: &RenderResult, p: Pass) -> Option<Buf> {
    r.pass(p).map(Buf::from_image)
}

#[cfg(test)]
mod tests;
