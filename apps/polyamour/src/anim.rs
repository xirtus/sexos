// SPDX-License-Identifier: GPL-3.0-only
//! Keyframe animation, Blender's way: every object can carry F-curves on its
//! `location`, `rotation_euler` and `scale` channels (one curve per axis), each
//! a list of keyframes with Constant, Linear or Bezier interpolation and
//! auto-clamped handles. Evaluating the curves at the current frame poses the
//! objects; that happens whenever the frame changes ([`tick`]).
//!
//! The module also keeps the report history the Info editor lists, the
//! in-program clipboard the text editors share and a small mailbox that routes
//! menu operators back to the editor whose header opened the menu.
use crate::doc::Doc;
use crate::ui::Op;
use sex_3d::script::quat_to_euler_xyz;
use sex_3d::{Quat, Transform, Vec3};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Constant,
    Linear,
    Bezier,
}

impl Interp {
    pub fn from_arg(s: &str) -> Option<Interp> {
        match s {
            "CONSTANT" => Some(Interp::Constant),
            "LINEAR" => Some(Interp::Linear),
            "BEZIER" => Some(Interp::Bezier),
            _ => None,
        }
    }
}

/// The transform property a curve drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prop {
    Location,
    Rotation,
    Scale,
}

impl Prop {
    pub const ALL: [Prop; 3] = [Prop::Location, Prop::Rotation, Prop::Scale];
    /// Blender's RNA path.
    pub fn path(self) -> &'static str {
        match self {
            Prop::Location => "location",
            Prop::Rotation => "rotation_euler",
            Prop::Scale => "scale",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Prop::Location => "Location",
            Prop::Rotation => "Euler Rotation",
            Prop::Scale => "Scale",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    pub frame: f32,
    pub value: f32,
    /// Interpolation towards the next key.
    pub interp: Interp,
    /// Handles as absolute (frame, value) points; auto-clamped, recomputed on change.
    pub left: [f32; 2],
    pub right: [f32; 2],
    pub select: bool,
}

impl Keyframe {
    pub fn new(frame: f32, value: f32, interp: Interp) -> Keyframe {
        Keyframe { frame, value, interp, left: [frame - 1.0, value], right: [frame + 1.0, value], select: false }
    }
}

/// One animated channel: `prop[index]`.
#[derive(Clone, Debug, PartialEq)]
pub struct FCurve {
    pub prop: Prop,
    pub index: usize,
    /// Sorted by frame, at most one key per frame.
    pub keys: Vec<Keyframe>,
    pub select: bool,
}

impl FCurve {
    pub fn new(prop: Prop, index: usize) -> FCurve {
        FCurve { prop, index, keys: Vec::new(), select: false }
    }
    /// Blender's channel name: "X Location", "Y Euler Rotation", "Z Scale".
    pub fn label(&self) -> String {
        format!("{} {}", ["X", "Y", "Z"][self.index.min(2)], self.prop.label())
    }
    /// The graph editor's XYZ-to-RGB curve colours.
    pub fn color(&self) -> u32 {
        [0xe8574d, 0x7ec34a, 0x4c8ef0][self.index.min(2)]
    }
    /// Position of this channel in an object's 9 transform values.
    pub fn slot(&self) -> usize {
        self.prop as usize * 3 + self.index
    }

    /// Add a key, replacing one already on that frame.
    pub fn insert(&mut self, frame: f32, value: f32, interp: Interp) {
        match self.keys.iter_mut().find(|k| (k.frame - frame).abs() < 0.01) {
            Some(k) => k.value = value,
            None => {
                let at = self.keys.partition_point(|k| k.frame < frame);
                self.keys.insert(at, Keyframe::new(frame, value, interp));
            }
        }
        self.recalc_handles();
    }

    /// Sort keys by frame and merge keys that landed on the same frame (a moved,
    /// selected key wins), then recompute handles. Run after moving keys.
    pub fn sort(&mut self) {
        self.keys.sort_by(|a, b| a.frame.total_cmp(&b.frame).then(a.select.cmp(&b.select)));
        let mut out: Vec<Keyframe> = Vec::with_capacity(self.keys.len());
        for k in self.keys.drain(..) {
            match out.last_mut() {
                Some(last) if (last.frame - k.frame).abs() < 0.01 => {
                    if k.select || !last.select {
                        *last = k;
                    }
                }
                _ => out.push(k),
            }
        }
        self.keys = out;
        self.recalc_handles();
    }

    /// Blender's auto-clamped handles: smooth through each key, flat at the
    /// ends and at local extremes, and never overshooting a neighbour.
    pub fn recalc_handles(&mut self) {
        let n = self.keys.len();
        for i in 0..n {
            let k = self.keys[i];
            let prev = (i > 0).then(|| self.keys[i - 1]);
            let next = (i + 1 < n).then(|| self.keys[i + 1]);
            let dl = prev.map_or(0.0, |p| (k.frame - p.frame) / 3.0);
            let dr = next.map_or(0.0, |q| (q.frame - k.frame) / 3.0);
            let mut slope = 0.0;
            if let (Some(p), Some(q)) = (prev, next) {
                let extreme = (k.value >= p.value && k.value >= q.value) || (k.value <= p.value && k.value <= q.value);
                if !extreme && q.frame > p.frame {
                    slope = (q.value - p.value) / (q.frame - p.frame);
                    // Clamp so neither handle passes its neighbour's value.
                    let lim_l = if dl > 0.0 { (k.value - p.value).abs() / dl } else { f32::INFINITY };
                    let lim_r = if dr > 0.0 { (q.value - k.value).abs() / dr } else { f32::INFINITY };
                    let lim = lim_l.min(lim_r);
                    if slope.abs() > lim {
                        slope = lim * slope.signum();
                    }
                }
            }
            let dl = if prev.is_some() { dl } else { dr.max(1.0 / 3.0) };
            let dr = if next.is_some() { dr } else { dl.max(1.0 / 3.0) };
            self.keys[i].left = [k.frame - dl, k.value - slope * dl];
            self.keys[i].right = [k.frame + dr, k.value + slope * dr];
        }
    }

    /// The curve's value at `frame` (constant extrapolation, as Blender's default).
    pub fn evaluate(&self, frame: f32) -> f32 {
        let keys = &self.keys;
        let (Some(first), Some(last)) = (keys.first(), keys.last()) else { return 0.0 };
        if frame <= first.frame {
            return first.value;
        }
        if frame >= last.frame {
            return last.value;
        }
        let i = keys.partition_point(|k| k.frame <= frame).saturating_sub(1);
        let (a, b) = (keys[i], keys[i + 1]);
        match a.interp {
            Interp::Constant => a.value,
            Interp::Linear => {
                let t = (frame - a.frame) / (b.frame - a.frame);
                a.value + (b.value - a.value) * t
            }
            Interp::Bezier => {
                let x1 = a.right[0].clamp(a.frame, b.frame);
                let x2 = b.left[0].clamp(a.frame, b.frame);
                let bez = |p0: f32, p1: f32, p2: f32, p3: f32, t: f32| {
                    let u = 1.0 - t;
                    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
                };
                // x(t) is monotonic with the handles inside the segment: bisect.
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..32 {
                    let mid = 0.5 * (lo + hi);
                    if bez(a.frame, x1, x2, b.frame, mid) < frame {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                bez(a.value, a.right[1], b.left[1], b.value, 0.5 * (lo + hi))
            }
        }
    }
}

/// An object's animation (Blender's action with an "Object Transforms" group).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectAnim {
    /// Objects are matched by name (names are unique in a scene).
    pub object: String,
    pub curves: Vec<FCurve>,
    /// Channel-list disclosure: the object row and its "Object Transforms" group.
    pub expanded: bool,
    pub group_expanded: bool,
}

impl ObjectAnim {
    fn curve_mut(&mut self, prop: Prop, index: usize) -> &mut FCurve {
        let pos = self.curves.iter().position(|c| c.prop == prop && c.index == index);
        let i = match pos {
            Some(i) => i,
            None => {
                let at = self.curves.partition_point(|c| (c.prop, c.index) < (prop, index));
                self.curves.insert(at, FCurve::new(prop, index));
                at
            }
        };
        &mut self.curves[i]
    }
    /// Sorted unique frames that carry a key on any channel.
    pub fn key_frames(&self) -> Vec<f32> {
        let mut f: Vec<f32> = self.curves.iter().flat_map(|c| c.keys.iter().map(|k| k.frame)).collect();
        f.sort_by(f32::total_cmp);
        f.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        f
    }
}

/// All animation in the document.
#[derive(Clone, Debug)]
pub struct Anim {
    pub objects: Vec<ObjectAnim>,
    /// The timeline's record button (auto-keying).
    pub auto_key: bool,
    /// Playing backwards (Shift+Ctrl+Space / the reverse play button).
    pub reverse: bool,
    /// Interpolation of new keys (Preferences > Animation, Bezier by default).
    pub interp: Interp,
    /// Keys or curves changed: evaluate again even if the frame did not.
    pub dirty: bool,
    last_frame: Option<i32>,
    /// Transforms of selected objects after the last evaluation, for auto-keying.
    seen: Vec<(String, Transform)>,
}

impl Default for Anim {
    fn default() -> Self {
        Anim { objects: Vec::new(), auto_key: false, reverse: false, interp: Interp::Bezier, dirty: false, last_frame: None, seen: Vec::new() }
    }
}

impl Anim {
    pub fn get(&self, object: &str) -> Option<&ObjectAnim> {
        self.objects.iter().find(|o| o.object == object)
    }
    pub fn get_mut(&mut self, object: &str) -> Option<&mut ObjectAnim> {
        self.objects.iter_mut().find(|o| o.object == object)
    }
    pub(crate) fn entry(&mut self, object: &str) -> &mut ObjectAnim {
        let i = match self.objects.iter().position(|o| o.object == object) {
            Some(i) => i,
            None => {
                self.objects.push(ObjectAnim { object: object.into(), curves: Vec::new(), expanded: true, group_expanded: true });
                self.objects.len() - 1
            }
        };
        &mut self.objects[i]
    }
    /// Follow an object rename: its curves keep driving it under the new name.
    pub fn rename_object(&mut self, old: &str, new: &str) {
        for o in self.objects.iter_mut().filter(|o| o.object == old) {
            o.object = new.to_string();
        }
        for (n, _) in self.seen.iter_mut().filter(|(n, _)| n == old) {
            *n = new.to_string();
        }
        self.dirty = true;
    }
    /// Every key, for selection passes.
    pub fn keys_mut(&mut self) -> impl Iterator<Item = &mut Keyframe> {
        self.objects.iter_mut().flat_map(|o| o.curves.iter_mut().flat_map(|c| c.keys.iter_mut()))
    }
    pub fn any_selected_key(&self) -> bool {
        self.objects.iter().any(|o| o.curves.iter().any(|c| c.keys.iter().any(|k| k.select)))
    }
}

/// A transform as Blender's nine animatable values.
pub fn channel_values(t: &Transform) -> [f32; 9] {
    let e = quat_to_euler_xyz(t.rotation);
    [t.location.x, t.location.y, t.location.z, e[0], e[1], e[2], t.scale.x, t.scale.y, t.scale.z]
}

/// Insert LocRotScale keys (or one property) on the selected objects at `frame`.
/// Returns how many objects were keyed.
pub fn insert_keys(doc: &mut Doc, frame: i32, only: Option<Prop>) -> usize {
    let sel = doc.scene.selected.clone();
    let interp = doc.anim.interp;
    let mut n = 0;
    for i in sel {
        let Some(o) = doc.scene.objects.get(i) else { continue };
        let (name, vals) = (o.name.clone(), channel_values(&o.transform));
        let oa = doc.anim.entry(&name);
        for prop in Prop::ALL {
            if only.is_some_and(|p| p != prop) {
                continue;
            }
            for axis in 0..3 {
                oa.curve_mut(prop, axis).insert(frame as f32, vals[prop as usize * 3 + axis], interp);
            }
        }
        n += 1;
    }
    n
}

/// Pose every animated object for the current frame. Returns true when a
/// transform changed.
pub fn evaluate(doc: &mut Doc) -> bool {
    let f = doc.frame as f32;
    let mut changed = false;
    for oa in &doc.anim.objects {
        let Some(i) = doc.scene.find_object(&oa.object) else { continue };
        let t = &mut doc.scene.objects[i].transform;
        let mut v = channel_values(t);
        let mut rot = false;
        for c in oa.curves.iter().filter(|c| !c.keys.is_empty()) {
            v[c.slot()] = c.evaluate(f);
            rot |= c.prop == Prop::Rotation;
        }
        let mut nt = *t;
        nt.location = Vec3::new(v[0], v[1], v[2]);
        if rot {
            nt.rotation = Quat::from_euler_xyz(v[3], v[4], v[5]);
        }
        nt.scale = Vec3::new(v[6], v[7], v[8]);
        if nt != *t {
            *t = nt;
            changed = true;
        }
    }
    if changed {
        doc.touch();
    }
    changed
}

fn snapshot(doc: &Doc) -> Vec<(String, Transform)> {
    doc.scene.selected.iter().filter_map(|&i| doc.scene.objects.get(i)).map(|o| (o.name.clone(), o.transform)).collect()
}

/// Per-frame bookkeeping, run before each frame is drawn and after its
/// operators: reverse playback, posing objects when the frame changed,
/// auto-keying and logging reports for the Info editor. Returns true when
/// something visible changed.
pub fn tick(doc: &mut Doc) -> bool {
    let mut changed = false;
    if let Some(last) = doc.anim.last_frame {
        let (s, e) = (doc.scene.frame_start, doc.scene.frame_end);
        if doc.playing && doc.anim.reverse && (doc.frame == last + 1 || (doc.frame == s && last >= e)) {
            doc.frame = if last <= s { e } else { last - 1 };
        }
    }
    if doc.anim.last_frame != Some(doc.frame) || doc.anim.dirty {
        changed |= doc.anim.last_frame.is_some_and(|l| l != doc.frame);
        changed |= evaluate(doc);
        changed |= doc.anim.dirty;
        doc.anim.last_frame = Some(doc.frame);
        doc.anim.dirty = false;
        doc.anim.seen = snapshot(doc);
    } else if doc.anim.auto_key && !doc.playing {
        let now = snapshot(doc);
        if now != doc.anim.seen {
            let moved: Vec<String> = now.iter().filter(|n| !doc.anim.seen.contains(n)).map(|n| n.0.clone()).collect();
            let keep = doc.scene.selected.clone();
            doc.scene.selected = moved.iter().filter_map(|n| doc.scene.find_object(n)).collect();
            insert_keys(doc, doc.frame, None);
            doc.scene.selected = keep;
            doc.anim.seen = now;
            changed = true;
        }
    }
    if doc.report != doc.info.last {
        doc.info.last = doc.report.clone();
        if !doc.report.is_empty() {
            let kind = classify(&doc.report);
            doc.info.reports.push(Report { kind, text: doc.report.clone() });
            changed = true;
        }
    }
    changed
}

fn classify(s: &str) -> ReportKind {
    let l = s.to_ascii_lowercase();
    if l.contains("error") || l.contains("cannot") || l.contains("failed") || l.contains("line ") && l.contains(':') {
        ReportKind::Error
    } else if l.contains("not available") || l.contains("nothing") || l.contains("no ") {
        ReportKind::Warning
    } else {
        ReportKind::Info
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    Info,
    Warning,
    Error,
    /// Text a script printed.
    Output,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub kind: ReportKind,
    pub text: String,
}

/// Report history, the shared clipboard and the editor mailbox.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub reports: Vec<Report>,
    last: String,
    pub clipboard: String,
    /// Operators addressed to one editor (`arg` = "@ID:rest"), drained by it.
    pub mailbox: Vec<Op>,
}

/// Report to the status bar and the Info log with an explicit kind.
pub fn report(doc: &mut Doc, kind: ReportKind, text: &str) {
    let line = text.lines().last().unwrap_or("").to_string();
    doc.report = line.clone();
    doc.info.last = line;
    for l in text.lines() {
        doc.info.reports.push(Report { kind, text: l.to_string() });
    }
}

/// A fresh id for an editor instance, used to address menu operators to it.
pub fn editor_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Take the mailbox operators addressed to editor `id`, as (op id, rest of arg).
pub fn take_mail(doc: &mut Doc, id: u64) -> Vec<(String, String)> {
    let tag = format!("@{id}:");
    let mut out = Vec::new();
    doc.info.mailbox.retain(|op| match op.arg.strip_prefix(&tag) {
        Some(rest) => {
            out.push((op.id.clone(), rest.to_string()));
            false
        }
        None => true,
    });
    out
}

/// An operator addressed to editor `id`.
pub fn mail(op: &str, id: u64, arg: &str) -> Op {
    Op::with(op, &format!("@{id}:{arg}"))
}

fn selected_names(doc: &Doc) -> Vec<String> {
    doc.scene.selected.iter().filter_map(|&i| doc.scene.objects.get(i)).map(|o| o.name.clone()).collect()
}

/// Frames that carry keys on the selected objects, sorted and unique.
pub fn selected_key_frames(doc: &Doc) -> Vec<f32> {
    let names = selected_names(doc);
    let mut f: Vec<f32> = doc.anim.objects.iter().filter(|o| names.contains(&o.object)).flat_map(|o| o.key_frames()).collect();
    f.sort_by(f32::total_cmp);
    f.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    f
}

/// Delete every selected key; curves left empty go away.
pub fn delete_selected_keys(doc: &mut Doc) -> usize {
    let mut n = 0;
    for o in &mut doc.anim.objects {
        for c in &mut o.curves {
            let before = c.keys.len();
            c.keys.retain(|k| !k.select);
            n += before - c.keys.len();
            c.recalc_handles();
        }
        o.curves.retain(|c| !c.keys.is_empty());
    }
    doc.anim.objects.retain(|o| !o.curves.is_empty());
    doc.anim.dirty = true;
    n
}

/// Animation operators. Returns false for names it does not know.
pub fn run(doc: &mut Doc, op: &Op) -> bool {
    let arg = op.arg.as_str();
    if arg.starts_with('@') {
        doc.info.mailbox.push(op.clone());
        return true;
    }
    match op.id.as_str() {
        "anim.keyframe_insert" | "anim.keyframe_insert_menu" | "action.keyframe_insert" | "graph.keyframe_insert" => {
            doc.push_undo("Insert Keyframes");
            let only = match arg {
                "Location" | "LOCATION" => Some(Prop::Location),
                "Rotation" | "ROTATION" => Some(Prop::Rotation),
                "Scaling" | "Scale" | "SCALE" => Some(Prop::Scale),
                _ => None,
            };
            let n = insert_keys(doc, doc.frame, only);
            doc.report = if n == 0 { "Nothing selected to insert keyframes on".into() } else { format!("Inserted keyframes on {n} object(s) at frame {}", doc.frame) };
        }
        "anim.keyframe_delete" | "anim.keyframe_delete_v3d" => {
            doc.push_undo("Delete Keyframes");
            let names = selected_names(doc);
            let f = doc.frame as f32;
            let mut n = 0;
            for o in doc.anim.objects.iter_mut().filter(|o| names.contains(&o.object)) {
                for c in &mut o.curves {
                    let before = c.keys.len();
                    c.keys.retain(|k| (k.frame - f).abs() >= 0.01);
                    n += before - c.keys.len();
                    c.recalc_handles();
                }
                o.curves.retain(|c| !c.keys.is_empty());
            }
            doc.anim.objects.retain(|o| !o.curves.is_empty());
            doc.report = if n == 0 { "No keyframes to delete on this frame".into() } else { format!("Deleted {n} keyframe(s)") };
        }
        "anim.keyframes_clear" | "anim.keyframe_clear_v3d" => {
            doc.push_undo("Clear Keyframes");
            let names = selected_names(doc);
            let before = doc.anim.objects.len();
            doc.anim.objects.retain(|o| !names.contains(&o.object));
            doc.report = format!("Cleared animation on {} object(s)", before - doc.anim.objects.len());
        }
        "screen.keyframe_jump" => {
            let frames = selected_key_frames(doc);
            let f = doc.frame as f32;
            let to = if arg == "PREV" { frames.iter().rev().find(|&&k| k < f - 0.01) } else { frames.iter().find(|&&k| k > f + 0.01) };
            match to {
                Some(&k) => doc.frame = k.round() as i32,
                None => doc.report = "No more keyframes to jump to in this direction".into(),
            }
        }
        "anim.change_frame" => {
            if let Ok(f) = arg.parse::<f32>() {
                doc.frame = f.round() as i32;
            }
        }
        "anim.default_interpolation" => {
            if let Some(i) = Interp::from_arg(arg) {
                doc.anim.interp = i;
            }
        }
        "anim.auto_keying_toggle" => doc.anim.auto_key = !doc.anim.auto_key,
        "anim.start_frame_set" => {
            doc.push_undo("Set Start Frame");
            doc.scene.frame_start = doc.frame.min(doc.scene.frame_end)
        }
        "anim.end_frame_set" => {
            doc.push_undo("Set End Frame");
            doc.scene.frame_end = doc.frame.max(doc.scene.frame_start)
        }
        "screen.animation_play_reverse" => {
            doc.anim.reverse = true;
            doc.playing = !doc.playing;
        }
        "action.delete" | "graph.delete" => {
            doc.push_undo("Delete Keyframes");
            let n = delete_selected_keys(doc);
            doc.report = format!("Deleted {n} keyframe(s)");
        }
        "action.select_all" | "graph.select_all" => {
            let all = doc.anim.any_selected_key();
            for k in doc.anim.keys_mut() {
                k.select = match arg {
                    "DESELECT" => false,
                    "INVERT" => !k.select,
                    "TOGGLE" => !all,
                    _ => true,
                };
            }
        }
        "action.interpolation_type" | "graph.interpolation_type" => {
            let Some(i) = Interp::from_arg(arg) else { return true };
            doc.push_undo("Interpolation Mode");
            for k in doc.anim.keys_mut().filter(|k| k.select) {
                k.interp = i;
            }
            doc.anim.dirty = true;
        }
        "action.frame_jump" | "graph.frame_jump" => {
            let sel: Vec<f32> = doc.anim.objects.iter().flat_map(|o| o.curves.iter().flat_map(|c| c.keys.iter().filter(|k| k.select).map(|k| k.frame))).collect();
            if !sel.is_empty() {
                doc.frame = (sel.iter().sum::<f32>() / sel.len() as f32).round() as i32;
            }
        }
        "action.keyframe_type" | "action.mirror" | "action.snap" | "graph.snap" | "action.duplicate" | "graph.duplicate" | "marker.add" | "marker.move" => {
            doc.report = format!("{} is not available yet", op.id);
        }
        _ => return false,
    }
    true
}

/// Take over the object animation of an imported .blend (location, Euler
/// rotation and scale curves), keeping Blender's key positions.
pub fn adopt_blend_actions(doc: &mut Doc, actions: &[sex_3d::io::blend::BlendAction]) {
    use sex_3d::io::blend::{INTERP_CONSTANT, INTERP_LINEAR};
    for a in actions {
        if !doc.scene.objects.iter().any(|o| o.name == a.object) {
            continue;
        }
        let obj = doc.anim.entry(&a.object);
        for c in &a.curves {
            let Some(prop) = Prop::ALL.into_iter().find(|p| p.path() == c.path) else { continue };
            if c.index > 2 {
                continue;
            }
            let curve = obj.curve_mut(prop, c.index);
            for k in &c.keys {
                let interp = match k.interpolation {
                    INTERP_CONSTANT => Interp::Constant,
                    INTERP_LINEAR => Interp::Linear,
                    _ => Interp::Bezier,
                };
                curve.insert(k.frame, k.value, interp);
            }
        }
    }
    doc.anim.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyframe_edits_undo_and_redo() {
        let mut d = Doc::default();
        d.frame = 4;
        d.run(&Op::new("anim.keyframe_insert"));
        assert!(!d.anim.objects.is_empty());
        d.run(&Op::new("ed.undo"));
        assert!(d.anim.objects.is_empty(), "undo removes the inserted keys");
        d.run(&Op::new("ed.redo"));
        assert!(!d.anim.objects.is_empty(), "redo brings them back");
        d.run(&Op::new("anim.keyframes_clear"));
        assert!(d.anim.objects.is_empty());
        d.run(&Op::new("ed.undo"));
        assert!(!d.anim.objects.is_empty());
    }

    #[test]
    fn bezier_curve_is_smooth_and_clamped() {
        let mut c = FCurve::new(Prop::Location, 0);
        c.insert(1.0, 0.0, Interp::Bezier);
        c.insert(21.0, 10.0, Interp::Bezier);
        assert_eq!(c.evaluate(1.0), 0.0);
        assert_eq!(c.evaluate(21.0), 10.0);
        // Flat end handles make an ease-in/ease-out S curve, symmetric in the middle.
        assert!((c.evaluate(11.0) - 5.0).abs() < 1e-3);
        assert!(c.evaluate(3.0) < 1.0, "eases in");
        // Never overshoots.
        c.insert(31.0, 10.5, Interp::Bezier);
        for f in 1..=31 {
            let v = c.evaluate(f as f32);
            assert!((-1e-4..=10.5 + 1e-4).contains(&v), "{f}: {v}");
        }
        c.keys[0].interp = Interp::Linear;
        assert!((c.evaluate(6.0) - 2.5).abs() < 1e-4);
        c.keys[0].interp = Interp::Constant;
        assert_eq!(c.evaluate(20.0), 0.0);
    }

    #[test]
    fn insert_evaluate_jump_delete() {
        let mut d = Doc::default();
        let cube = d.active().unwrap();
        d.select_only(Some(cube));
        d.frame = 1;
        assert!(run(&mut d, &Op::new("anim.keyframe_insert")));
        d.frame = 20;
        d.scene.objects[cube].transform.location = Vec3::new(4.0, 0.0, 2.0);
        run(&mut d, &Op::new("anim.keyframe_insert"));
        assert_eq!(d.anim.objects[0].curves.len(), 9);
        tick(&mut d);
        d.frame = 10;
        assert!(tick(&mut d));
        let x = d.scene.objects[cube].transform.location.x;
        assert!(x > 0.5 && x < 3.5, "{x}");
        d.frame = 5;
        run(&mut d, &Op::with("screen.keyframe_jump", "NEXT"));
        assert_eq!(d.frame, 20);
        run(&mut d, &Op::with("screen.keyframe_jump", "PREV"));
        assert_eq!(d.frame, 1);
        run(&mut d, &Op::new("anim.keyframe_delete"));
        assert_eq!(d.anim.objects[0].key_frames(), vec![20.0]);
        run(&mut d, &Op::new("anim.keyframes_clear"));
        assert!(d.anim.objects.is_empty());
    }

    #[test]
    fn auto_keying_and_reverse_playback() {
        let mut d = Doc::default();
        let cube = d.active().unwrap();
        d.anim.auto_key = true;
        tick(&mut d);
        d.scene.objects[cube].transform.location.z = 2.0;
        assert!(tick(&mut d));
        assert_eq!(d.anim.objects.len(), 1);
        assert_eq!(d.anim.objects[0].key_frames(), vec![1.0]);
        // Reverse playback: the loop steps +1, tick turns it into -1 (wrapping).
        d.playing = true;
        d.anim.reverse = true;
        d.frame = 2;
        tick(&mut d);
        assert_eq!(d.frame, d.scene.frame_end);
        d.frame = d.scene.frame_start;
        tick(&mut d);
        assert_eq!(d.frame, d.scene.frame_end - 1);
    }

    #[test]
    fn reports_are_logged() {
        let mut d = Doc::default();
        d.report = "Saved \"x.poly\"".into();
        tick(&mut d);
        report(&mut d, ReportKind::Error, "line 2: boom");
        tick(&mut d);
        assert_eq!(d.info.reports.len(), 2);
        assert_eq!(d.info.reports[1].kind, ReportKind::Error);
    }
}
