// SPDX-License-Identifier: GPL-3.0-only
//! Physics settings with Blender's names, defaults and panel layout:
//! RigidBodyWorld, RigidBodyObject, RigidBodyConstraint, FieldSettings
//! (PartDeflect), CollisionSettings, ClothSettings + ClothCollisionSettings,
//! SoftBodySettings, ParticleSettings, FluidDomainSettings /
//! FluidFlowSettings / FluidEffectorSettings, EffectorWeights and PointCache.
use super::props::{Prop, Props, Range, RefKind, Visitor};
use crate::prop_enum;

const DEG: f32 = std::f32::consts::PI / 180.0;
const FAC: Range = Range::new(0.0, 1.0, 0.01).slider();
const POS: Range = Range::new(0.0, 1.0e6, 0.1);
const ANY: Range = Range::new(-1.0e6, 1.0e6, 0.1);
const LEN: Range = Range::new(0.0, 1.0e4, 0.01).unit("m");
const ANG: Range = Range::new(-std::f32::consts::TAU, std::f32::consts::TAU, 0.0174533).unit("rad");

// ---------------------------------------------------------------------------
// Shared blocks

/// Blender's "Field Weights": how strongly each kind of effector acts.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectorWeights {
    /// Only effectors in this collection (empty: every effector).
    pub collection: String,
    pub gravity: f32,
    pub all: f32,
    pub force: f32,
    pub vortex: f32,
    pub magnetic: f32,
    pub harmonic: f32,
    pub charge: f32,
    pub wind: f32,
    pub turbulence: f32,
    pub drag: f32,
}

impl Default for EffectorWeights {
    fn default() -> Self {
        EffectorWeights { collection: String::new(), gravity: 1.0, all: 1.0, force: 1.0, vortex: 1.0, magnetic: 1.0, harmonic: 1.0, charge: 1.0, wind: 1.0, turbulence: 1.0, drag: 1.0 }
    }
}

impl EffectorWeights {
    pub fn of(&self, t: FieldType) -> f32 {
        let w = match t {
            FieldType::Force => self.force,
            FieldType::Wind => self.wind,
            FieldType::Vortex => self.vortex,
            FieldType::Magnetic => self.magnetic,
            FieldType::Harmonic => self.harmonic,
            FieldType::Charge => self.charge,
            FieldType::Turbulence => self.turbulence,
            FieldType::Drag => self.drag,
        };
        w * self.all
    }

    fn visit(&mut self, v: &mut dyn Visitor) {
        v.panel("field_weights", "Field Weights", false, None);
        v.prop("weight_collection", "Effector Collection", Prop::Ref(&mut self.collection, RefKind::Collection));
        let r = Range::new(-200.0, 200.0, 0.01).slider();
        v.prop("weight_gravity", "Gravity", Prop::Float(&mut self.gravity, r));
        v.prop("weight_all", "All", Prop::Float(&mut self.all, r));
        v.prop("weight_force", "Force", Prop::Float(&mut self.force, r));
        v.prop("weight_vortex", "Vortex", Prop::Float(&mut self.vortex, r));
        v.prop("weight_magnetic", "Magnetic", Prop::Float(&mut self.magnetic, r));
        v.prop("weight_harmonic", "Harmonic", Prop::Float(&mut self.harmonic, r));
        v.prop("weight_charge", "Charge", Prop::Float(&mut self.charge, r));
        v.prop("weight_wind", "Wind", Prop::Float(&mut self.wind, r));
        v.prop("weight_turbulence", "Turbulence", Prop::Float(&mut self.turbulence, r));
        v.prop("weight_drag", "Drag", Prop::Float(&mut self.drag, r));
        v.end();
    }
}

/// Blender's PointCache settings (the runtime frames live in
/// [`super::cache::Caches`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PointCacheSettings {
    pub frame_start: i32,
    pub frame_end: i32,
    /// Write baked frames to `//cache/` next to the `.poly` file.
    pub use_disk_cache: bool,
}

impl Default for PointCacheSettings {
    fn default() -> Self {
        PointCacheSettings { frame_start: 1, frame_end: 250, use_disk_cache: false }
    }
}

impl PointCacheSettings {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.panel("cache", "Cache", false, None);
        v.prop("cache_frame_start", "Simulation Start", Prop::Int(&mut self.frame_start, -100000, 1000000));
        v.prop("cache_frame_end", "End", Prop::Int(&mut self.frame_end, 1, 1000000));
        v.prop("use_disk_cache", "Disk Cache", Prop::Bool(&mut self.use_disk_cache));
        v.end();
    }
}

// ---------------------------------------------------------------------------
// Rigid bodies

prop_enum!(RigidBodyType { Active = ("ACTIVE", "Active"), Passive = ("PASSIVE", "Passive") } default Active);
prop_enum!(CollisionShape {
    Box = ("BOX", "Box"),
    Sphere = ("SPHERE", "Sphere"),
    Capsule = ("CAPSULE", "Capsule"),
    Cylinder = ("CYLINDER", "Cylinder"),
    Cone = ("CONE", "Cone"),
    ConvexHull = ("CONVEX_HULL", "Convex Hull"),
    Mesh = ("MESH", "Mesh"),
} default ConvexHull);
prop_enum!(MeshSource { Base = ("BASE", "Base"), Deform = ("DEFORM", "Deform"), Final = ("FINAL", "Final") } default Base);

/// Scene > Rigid Body World.
#[derive(Clone, Debug, PartialEq)]
pub struct RigidBodyWorld {
    pub enabled: bool,
    /// Blender's world collection. Objects here belong to one collection
    /// only, so every object with a rigid body takes part; the name is kept
    /// for files.
    pub collection: String,
    pub constraints: String,
    pub time_scale: f32,
    pub substeps_per_frame: i32,
    pub solver_iterations: i32,
    pub use_split_impulse: bool,
    pub effector_weights: EffectorWeights,
    pub point_cache: PointCacheSettings,
}

impl Default for RigidBodyWorld {
    fn default() -> Self {
        RigidBodyWorld {
            enabled: true,
            collection: "RigidBodyWorld".into(),
            constraints: String::new(),
            time_scale: 1.0,
            substeps_per_frame: 10,
            solver_iterations: 10,
            use_split_impulse: false,
            effector_weights: EffectorWeights::default(),
            point_cache: PointCacheSettings::default(),
        }
    }
}

impl Props for RigidBodyWorld {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.prop("enabled", "Enabled", Prop::Bool(&mut self.enabled));
        v.panel("rbw_settings", "Settings", true, None);
        v.prop("collection", "Collection", Prop::Ref(&mut self.collection, RefKind::Collection));
        v.prop("constraints", "Constraints", Prop::Ref(&mut self.constraints, RefKind::Collection));
        v.prop("time_scale", "Speed", Prop::Float(&mut self.time_scale, Range::new(0.0, 100.0, 0.1)));
        v.prop("use_split_impulse", "Split Impulse", Prop::Bool(&mut self.use_split_impulse));
        v.prop("substeps_per_frame", "Substeps Per Frame", Prop::Int(&mut self.substeps_per_frame, 1, 1000));
        v.prop("solver_iterations", "Solver Iterations", Prop::Int(&mut self.solver_iterations, 1, 1000));
        v.end();
        self.point_cache.visit(v);
        self.effector_weights.visit(v);
    }
}

/// Object > Physics > Rigid Body (Blender's RigidBodyObject).
#[derive(Clone, Debug, PartialEq)]
pub struct RigidBody {
    pub kind: RigidBodyType,
    /// "Dynamic": takes part in the simulation (off: stays where animated).
    pub enabled: bool,
    /// "Animated": moved by its animation, pushing others (kinematic).
    pub kinematic: bool,
    pub mass: f32,
    pub collision_shape: CollisionShape,
    pub mesh_source: MeshSource,
    pub friction: f32,
    /// Bounciness.
    pub restitution: f32,
    pub use_margin: bool,
    pub collision_margin: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub use_deactivation: bool,
    pub use_start_deactivated: bool,
    pub deactivate_linear_velocity: f32,
    pub deactivate_angular_velocity: f32,
}

impl Default for RigidBody {
    fn default() -> Self {
        RigidBody {
            kind: RigidBodyType::Active,
            enabled: true,
            kinematic: false,
            mass: 1.0,
            collision_shape: CollisionShape::ConvexHull,
            mesh_source: MeshSource::Base,
            friction: 0.5,
            restitution: 0.0,
            use_margin: false,
            collision_margin: 0.04,
            linear_damping: 0.04,
            angular_damping: 0.1,
            use_deactivation: false,
            use_start_deactivated: false,
            deactivate_linear_velocity: 0.4,
            deactivate_angular_velocity: 0.5,
        }
    }
}

impl Props for RigidBody {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.prop("type", "Type", Prop::Enum(&mut self.kind));
        v.panel("rb_settings", "Settings", true, None);
        let active = self.kind == RigidBodyType::Active;
        v.active(active);
        v.prop("mass", "Mass", Prop::Float(&mut self.mass, Range::new(0.001, 1.0e6, 0.1).unit("kg")));
        v.active(true);
        v.prop("enabled", "Dynamic", Prop::Bool(&mut self.enabled));
        v.prop("kinematic", "Animated", Prop::Bool(&mut self.kinematic));
        v.end();
        v.panel("rb_collisions", "Collisions", true, None);
        v.prop("collision_shape", "Shape", Prop::Enum(&mut self.collision_shape));
        v.active(self.collision_shape == CollisionShape::Mesh || self.collision_shape == CollisionShape::ConvexHull);
        v.prop("mesh_source", "Source", Prop::Enum(&mut self.mesh_source));
        v.active(true);
        v.panel("rb_surface", "Surface Response", true, None);
        v.prop("friction", "Friction", Prop::Float(&mut self.friction, Range::new(0.0, 1.0, 0.01).slider()));
        v.prop("restitution", "Bounciness", Prop::Float(&mut self.restitution, Range::new(0.0, 1.0, 0.01).slider()));
        v.end();
        let mut m = self.use_margin;
        v.panel("rb_sensitivity", "Sensitivity", false, Some(("use_margin", &mut m)));
        v.prop("collision_margin", "Margin", Prop::Float(&mut self.collision_margin, Range::new(0.0, 1.0, 0.01).unit("m")));
        v.end();
        self.use_margin = m;
        v.end();
        v.panel("rb_dynamics", "Dynamics", false, None);
        v.active(active);
        v.prop("linear_damping", "Damping Translation", Prop::Float(&mut self.linear_damping, FAC));
        v.prop("angular_damping", "Rotation", Prop::Float(&mut self.angular_damping, FAC));
        let mut d = self.use_deactivation;
        v.panel("rb_deactivation", "Deactivation", false, Some(("use_deactivation", &mut d)));
        v.prop("use_start_deactivated", "Start Deactivated", Prop::Bool(&mut self.use_start_deactivated));
        v.prop("deactivate_linear_velocity", "Velocity Linear", Prop::Float(&mut self.deactivate_linear_velocity, Range::new(0.0, 1.0e3, 0.1).unit("m/s")));
        v.prop("deactivate_angular_velocity", "Angular", Prop::Float(&mut self.deactivate_angular_velocity, Range::new(0.0, 1.0e3, 0.1).unit("rad/s")));
        v.end();
        self.use_deactivation = d;
        v.end();
    }
}

prop_enum!(ConstraintType {
    Fixed = ("FIXED", "Fixed"),
    Point = ("POINT", "Point"),
    Hinge = ("HINGE", "Hinge"),
    Slider = ("SLIDER", "Slider"),
    Piston = ("PISTON", "Piston"),
    Generic = ("GENERIC", "Generic"),
    GenericSpring = ("GENERIC_SPRING", "Generic Spring"),
    Motor = ("MOTOR", "Motor"),
} default Point);
prop_enum!(SpringType { Spring1 = ("SPRING1", "Blender 2.7"), Spring2 = ("SPRING2", "Blender 2.8") } default Spring2);

/// One axis of a Generic constraint: limit and spring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisLimit {
    pub use_limit: bool,
    pub lower: f32,
    pub upper: f32,
    pub use_spring: bool,
    pub stiffness: f32,
    pub damping: f32,
}

/// Object > Physics > Rigid Body Constraint.
#[derive(Clone, Debug, PartialEq)]
pub struct RigidBodyConstraint {
    pub kind: ConstraintType,
    pub enabled: bool,
    pub disable_collisions: bool,
    pub object1: String,
    pub object2: String,
    pub use_breaking: bool,
    pub breaking_threshold: f32,
    pub use_override_solver_iterations: bool,
    pub solver_iterations: i32,
    /// X, Y, Z translation then X, Y, Z rotation.
    pub axes: [AxisLimit; 6],
    pub spring_type: SpringType,
    pub use_motor_lin: bool,
    pub motor_lin_target_velocity: f32,
    pub motor_lin_max_impulse: f32,
    pub use_motor_ang: bool,
    pub motor_ang_target_velocity: f32,
    pub motor_ang_max_impulse: f32,
}

impl Default for RigidBodyConstraint {
    fn default() -> Self {
        let lin = AxisLimit { use_limit: false, lower: -1.0, upper: 1.0, use_spring: false, stiffness: 10.0, damping: 0.5 };
        let ang = AxisLimit { lower: -45.0 * DEG, upper: 45.0 * DEG, ..lin };
        RigidBodyConstraint {
            kind: ConstraintType::Point,
            enabled: true,
            disable_collisions: true,
            object1: String::new(),
            object2: String::new(),
            use_breaking: false,
            breaking_threshold: 10.0,
            use_override_solver_iterations: false,
            solver_iterations: 10,
            axes: [lin, lin, lin, ang, ang, ang],
            spring_type: SpringType::Spring2,
            use_motor_lin: false,
            motor_lin_target_velocity: 1.0,
            motor_lin_max_impulse: 1.0,
            use_motor_ang: false,
            motor_ang_target_velocity: 1.0,
            motor_ang_max_impulse: 1.0,
        }
    }
}

const AXIS_IDS: [[&str; 6]; 6] = [
    ["use_limit_lin_x", "limit_lin_x_lower", "limit_lin_x_upper", "use_spring_x", "spring_stiffness_x", "spring_damping_x"],
    ["use_limit_lin_y", "limit_lin_y_lower", "limit_lin_y_upper", "use_spring_y", "spring_stiffness_y", "spring_damping_y"],
    ["use_limit_lin_z", "limit_lin_z_lower", "limit_lin_z_upper", "use_spring_z", "spring_stiffness_z", "spring_damping_z"],
    ["use_limit_ang_x", "limit_ang_x_lower", "limit_ang_x_upper", "use_spring_ang_x", "spring_stiffness_ang_x", "spring_damping_ang_x"],
    ["use_limit_ang_y", "limit_ang_y_lower", "limit_ang_y_upper", "use_spring_ang_y", "spring_stiffness_ang_y", "spring_damping_ang_y"],
    ["use_limit_ang_z", "limit_ang_z_lower", "limit_ang_z_upper", "use_spring_ang_z", "spring_stiffness_ang_z", "spring_damping_ang_z"],
];
const AXIS_LABELS: [&str; 6] = ["X Linear", "Y", "Z", "X Angular", "Y", "Z"];

impl Props for RigidBodyConstraint {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.panel("rbc_settings", "Settings", true, None);
        v.prop("type", "Type", Prop::Enum(&mut self.kind));
        v.prop("enabled", "Enabled", Prop::Bool(&mut self.enabled));
        v.prop("disable_collisions", "Disable Collisions", Prop::Bool(&mut self.disable_collisions));
        v.end();
        v.panel("rbc_objects", "Objects", true, None);
        v.prop("object1", "First", Prop::Ref(&mut self.object1, RefKind::Object));
        v.prop("object2", "Second", Prop::Ref(&mut self.object2, RefKind::Object));
        v.end();
        let mut ob = self.use_override_solver_iterations;
        v.panel("rbc_override", "Override Iterations", false, Some(("use_override_solver_iterations", &mut ob)));
        v.prop("solver_iterations", "Iterations", Prop::Int(&mut self.solver_iterations, 1, 1000));
        v.end();
        self.use_override_solver_iterations = ob;
        let mut br = self.use_breaking;
        v.panel("rbc_breaking", "Breakable", false, Some(("use_breaking", &mut br)));
        v.prop("breaking_threshold", "Threshold", Prop::Float(&mut self.breaking_threshold, POS));
        v.end();
        self.use_breaking = br;
        let k = self.kind;
        let has_limits = matches!(k, ConstraintType::Hinge | ConstraintType::Slider | ConstraintType::Piston | ConstraintType::Generic | ConstraintType::GenericSpring);
        v.panel("rbc_limits", "Limits", false, None);
        v.active(has_limits);
        for (i, a) in self.axes.iter_mut().enumerate() {
            let ids = AXIS_IDS[i];
            let r = if i < 3 { Range::new(-1.0e4, 1.0e4, 0.1).unit("m") } else { ANG };
            v.prop(ids[0], AXIS_LABELS[i], Prop::Bool(&mut a.use_limit));
            v.prop(ids[1], "Lower", Prop::Float(&mut a.lower, r));
            v.prop(ids[2], "Upper", Prop::Float(&mut a.upper, r));
        }
        v.end();
        v.panel("rbc_motor", "Motor", false, None);
        v.active(k == ConstraintType::Motor);
        v.prop("use_motor_ang", "Angular", Prop::Bool(&mut self.use_motor_ang));
        v.prop("motor_ang_target_velocity", "Target Velocity", Prop::Float(&mut self.motor_ang_target_velocity, ANY));
        v.prop("motor_ang_max_impulse", "Max Impulse", Prop::Float(&mut self.motor_ang_max_impulse, POS));
        v.prop("use_motor_lin", "Linear", Prop::Bool(&mut self.use_motor_lin));
        v.prop("motor_lin_target_velocity", "Target Velocity", Prop::Float(&mut self.motor_lin_target_velocity, ANY));
        v.prop("motor_lin_max_impulse", "Max Impulse", Prop::Float(&mut self.motor_lin_max_impulse, POS));
        v.end();
        v.panel("rbc_springs", "Springs", false, None);
        v.active(k == ConstraintType::GenericSpring);
        v.prop("spring_type", "Type", Prop::Enum(&mut self.spring_type));
        for (i, a) in self.axes.iter_mut().enumerate() {
            let ids = AXIS_IDS[i];
            v.prop(ids[3], AXIS_LABELS[i], Prop::Bool(&mut a.use_spring));
            v.prop(ids[4], "Stiffness", Prop::Float(&mut a.stiffness, POS));
            v.prop(ids[5], "Damping", Prop::Float(&mut a.damping, POS));
        }
        v.end();
    }
}

// ---------------------------------------------------------------------------
// Force fields

prop_enum!(FieldType {
    Force = ("FORCE", "Force"),
    Wind = ("WIND", "Wind"),
    Vortex = ("VORTEX", "Vortex"),
    Magnetic = ("MAGNET", "Magnetic"),
    Harmonic = ("HARMONIC", "Harmonic"),
    Charge = ("CHARGE", "Charge"),
    Turbulence = ("TURBULENCE", "Turbulence"),
    Drag = ("DRAG", "Drag"),
} default Force);
prop_enum!(FieldShape { Point = ("POINT", "Point"), Plane = ("PLANE", "Plane") } default Point);
prop_enum!(FalloffType { Sphere = ("SPHERE", "Sphere"), Tube = ("TUBE", "Tube"), Cone = ("CONE", "Cone") } default Sphere);
prop_enum!(ZDirection { Both = ("BOTH", "+Z & -Z"), Positive = ("POSITIVE", "+Z"), Negative = ("NEGATIVE", "-Z") } default Both);

/// Object > Physics > Force Field (Blender's FieldSettings / PartDeflect).
#[derive(Clone, Debug, PartialEq)]
pub struct FieldSettings {
    pub kind: FieldType,
    pub shape: FieldShape,
    pub strength: f32,
    pub flow: f32,
    pub noise: f32,
    pub seed: i32,
    /// Turbulence noise size.
    pub size: f32,
    pub use_global_coords: bool,
    pub apply_to_location: bool,
    pub apply_to_rotation: bool,
    pub use_absorption: bool,
    /// Harmonic: damping and rest length.
    pub harmonic_damping: f32,
    pub rest_length: f32,
    /// Vortex: inflow.
    pub inflow: f32,
    /// Drag.
    pub linear_drag: f32,
    pub quadratic_drag: f32,
    pub falloff_type: FalloffType,
    pub z_direction: ZDirection,
    pub falloff_power: f32,
    pub use_min_distance: bool,
    pub distance_min: f32,
    pub use_max_distance: bool,
    pub distance_max: f32,
    pub radial_falloff: f32,
    pub use_radial_min: bool,
    pub radial_min: f32,
    pub use_radial_max: bool,
    pub radial_max: f32,
}

impl Default for FieldSettings {
    fn default() -> Self {
        FieldSettings {
            kind: FieldType::Force,
            shape: FieldShape::Point,
            strength: 1.0,
            flow: 0.0,
            noise: 0.0,
            seed: 0,
            size: 1.0,
            use_global_coords: false,
            apply_to_location: true,
            apply_to_rotation: true,
            use_absorption: false,
            harmonic_damping: 0.0,
            rest_length: 0.0,
            inflow: 0.0,
            linear_drag: 0.0,
            quadratic_drag: 0.0,
            falloff_type: FalloffType::Sphere,
            z_direction: ZDirection::Both,
            falloff_power: 0.0,
            use_min_distance: false,
            distance_min: 0.0,
            use_max_distance: false,
            distance_max: 0.0,
            radial_falloff: 0.0,
            use_radial_min: false,
            radial_min: 0.0,
            use_radial_max: false,
            radial_max: 0.0,
        }
    }
}

impl FieldSettings {
    /// Blender's defaults for a new field of type `kind` (Drag and Harmonic
    /// start with their own strengths).
    pub fn new(kind: FieldType) -> FieldSettings {
        let mut f = FieldSettings { kind, ..Default::default() };
        if kind == FieldType::Drag {
            f.linear_drag = 1.0;
        }
        if kind == FieldType::Wind || kind == FieldType::Vortex {
            f.shape = FieldShape::Plane;
        }
        f
    }
}

impl Props for FieldSettings {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.prop("type", "Type", Prop::Enum(&mut self.kind));
        v.panel("field_settings", "Settings", true, None);
        v.prop("shape", "Shape", Prop::Enum(&mut self.shape));
        let k = self.kind;
        if k == FieldType::Drag {
            v.prop("linear_drag", "Linear", Prop::Float(&mut self.linear_drag, Range::new(-2.0, 2.0, 0.01)));
            v.prop("quadratic_drag", "Quadratic", Prop::Float(&mut self.quadratic_drag, Range::new(-2.0, 2.0, 0.01)));
        } else {
            v.prop("strength", "Strength", Prop::Float(&mut self.strength, ANY));
        }
        if k == FieldType::Turbulence {
            v.prop("size", "Size", Prop::Float(&mut self.size, Range::new(0.0, 1.0e3, 0.1)));
            v.prop("use_global_coords", "Global", Prop::Bool(&mut self.use_global_coords));
        }
        if k == FieldType::Harmonic {
            v.prop("harmonic_damping", "Damping", Prop::Float(&mut self.harmonic_damping, Range::new(0.0, 10.0, 0.01)));
            v.prop("rest_length", "Rest Length", Prop::Float(&mut self.rest_length, LEN));
        }
        if k == FieldType::Vortex {
            v.prop("inflow", "Inflow", Prop::Float(&mut self.inflow, Range::new(-10.0, 10.0, 0.01)));
        }
        v.active(!matches!(k, FieldType::Harmonic | FieldType::Charge | FieldType::Drag | FieldType::Magnetic));
        v.prop("flow", "Flow", Prop::Float(&mut self.flow, Range::new(0.0, 10.0, 0.01)));
        v.active(true);
        v.prop("noise", "Noise Amount", Prop::Float(&mut self.noise, Range::new(0.0, 10.0, 0.01)));
        v.prop("seed", "Seed", Prop::Int(&mut self.seed, 1, 128));
        v.prop("apply_to_location", "Affect Location", Prop::Bool(&mut self.apply_to_location));
        v.prop("apply_to_rotation", "Affect Rotation", Prop::Bool(&mut self.apply_to_rotation));
        v.prop("use_absorption", "Absorption", Prop::Bool(&mut self.use_absorption));
        v.end();
        v.panel("field_falloff", "Falloff", false, None);
        v.prop("falloff_type", "Shape", Prop::Enum(&mut self.falloff_type));
        v.prop("z_direction", "Z Direction", Prop::Enum(&mut self.z_direction));
        v.prop("falloff_power", "Power", Prop::Float(&mut self.falloff_power, Range::new(0.0, 10.0, 0.1)));
        v.prop("use_min_distance", "Min Distance", Prop::Bool(&mut self.use_min_distance));
        v.prop("distance_min", "Distance", Prop::Float(&mut self.distance_min, LEN));
        v.prop("use_max_distance", "Max Distance", Prop::Bool(&mut self.use_max_distance));
        v.prop("distance_max", "Distance", Prop::Float(&mut self.distance_max, LEN));
        if self.falloff_type != FalloffType::Sphere {
            v.panel("field_falloff_radial", if self.falloff_type == FalloffType::Cone { "Angular" } else { "Radial" }, false, None);
            v.prop("radial_falloff", "Power", Prop::Float(&mut self.radial_falloff, Range::new(0.0, 10.0, 0.1)));
            v.prop("use_radial_min", "Min", Prop::Bool(&mut self.use_radial_min));
            v.prop("radial_min", "Distance", Prop::Float(&mut self.radial_min, Range::new(0.0, 1.0e3, 0.01)));
            v.prop("use_radial_max", "Max", Prop::Bool(&mut self.use_radial_max));
            v.prop("radial_max", "Distance", Prop::Float(&mut self.radial_max, Range::new(0.0, 1.0e3, 0.01)));
            v.end();
        }
        v.end();
    }
}

// ---------------------------------------------------------------------------
// Collision

/// Object > Physics > Collision (Blender's CollisionSettings, carried by
/// the Collision modifier).
#[derive(Clone, Debug, PartialEq)]
pub struct CollisionSettings {
    pub use_collision: bool,
    /// Particles: "Field Absorption".
    pub absorption: f32,
    pub permeability: f32,
    pub stickiness: f32,
    pub use_particle_kill: bool,
    pub damping_factor: f32,
    pub damping_random: f32,
    pub friction_factor: f32,
    pub friction_random: f32,
    /// Soft body and cloth.
    pub damping: f32,
    pub thickness_outer: f32,
    pub thickness_inner: f32,
    pub cloth_friction: f32,
    pub use_culling: bool,
    pub use_normal: bool,
}

impl Default for CollisionSettings {
    fn default() -> Self {
        CollisionSettings {
            use_collision: true,
            absorption: 0.0,
            permeability: 0.0,
            stickiness: 0.0,
            use_particle_kill: false,
            damping_factor: 0.0,
            damping_random: 0.0,
            friction_factor: 0.0,
            friction_random: 0.0,
            damping: 0.1,
            thickness_outer: 0.02,
            thickness_inner: 0.2,
            cloth_friction: 5.0,
            use_culling: true,
            use_normal: false,
        }
    }
}

impl Props for CollisionSettings {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.panel("collision_settings", "Settings", true, None);
        v.prop("use", "Enabled", Prop::Bool(&mut self.use_collision));
        v.prop("absorption", "Field Absorption", Prop::Float(&mut self.absorption, FAC));
        v.end();
        v.panel("collision_particle", "Particle", true, None);
        v.prop("permeability", "Permeability", Prop::Float(&mut self.permeability, FAC));
        v.prop("stickiness", "Stickiness", Prop::Float(&mut self.stickiness, Range::new(0.0, 10.0, 0.01)));
        v.prop("use_particle_kill", "Kill Particles", Prop::Bool(&mut self.use_particle_kill));
        v.prop("damping_factor", "Damping", Prop::Float(&mut self.damping_factor, FAC));
        v.prop("damping_random", "Randomize", Prop::Float(&mut self.damping_random, FAC));
        v.prop("friction_factor", "Friction", Prop::Float(&mut self.friction_factor, FAC));
        v.prop("friction_random", "Randomize", Prop::Float(&mut self.friction_random, FAC));
        v.end();
        v.panel("collision_softbody", "Softbody & Cloth", true, None);
        v.prop("damping", "Damping", Prop::Float(&mut self.damping, FAC));
        v.prop("thickness_outer", "Thickness Outer", Prop::Float(&mut self.thickness_outer, Range::new(0.001, 1.0, 0.001).unit("m")));
        v.prop("thickness_inner", "Inner", Prop::Float(&mut self.thickness_inner, Range::new(0.001, 1.0, 0.001).unit("m")));
        v.prop("cloth_friction", "Friction", Prop::Float(&mut self.cloth_friction, Range::new(0.0, 80.0, 0.1)));
        v.prop("use_culling", "Single Sided", Prop::Bool(&mut self.use_culling));
        v.prop("use_normal", "Override Normals", Prop::Bool(&mut self.use_normal));
        v.end();
    }
}

// ---------------------------------------------------------------------------
// Cloth

prop_enum!(BendingModel { Angular = ("ANGULAR", "Angular"), Linear = ("LINEAR", "Linear") } default Angular);

/// Object > Physics > Cloth (ClothSettings and ClothCollisionSettings).
#[derive(Clone, Debug, PartialEq)]
pub struct ClothSettings {
    pub quality: i32,
    pub time_scale: f32,
    pub mass: f32,
    pub air_damping: f32,
    pub bending_model: BendingModel,
    pub tension_stiffness: f32,
    pub compression_stiffness: f32,
    pub shear_stiffness: f32,
    pub bending_stiffness: f32,
    pub tension_damping: f32,
    pub compression_damping: f32,
    pub shear_damping: f32,
    pub bending_damping: f32,
    pub use_pressure: bool,
    pub uniform_pressure_force: f32,
    pub vertex_group_mass: String,
    pub pin_stiffness: f32,
    pub shrink_min: f32,
    pub vertex_group_shrink: String,
    pub use_sewing_springs: bool,
    pub sewing_force_max: f32,
    pub use_dynamic_mesh: bool,
    pub use_collision: bool,
    pub collision_quality: i32,
    pub distance_min: f32,
    pub impulse_clamp: f32,
    pub collision_collection: String,
    pub use_self_collision: bool,
    pub self_friction: f32,
    pub self_distance_min: f32,
    pub effector_weights: EffectorWeights,
    pub point_cache: PointCacheSettings,
}

impl Default for ClothSettings {
    fn default() -> Self {
        ClothSettings {
            quality: 5,
            time_scale: 1.0,
            mass: 0.3,
            air_damping: 1.0,
            bending_model: BendingModel::Angular,
            tension_stiffness: 15.0,
            compression_stiffness: 15.0,
            shear_stiffness: 5.0,
            bending_stiffness: 0.5,
            tension_damping: 5.0,
            compression_damping: 5.0,
            shear_damping: 5.0,
            bending_damping: 0.5,
            use_pressure: false,
            uniform_pressure_force: 0.0,
            vertex_group_mass: String::new(),
            pin_stiffness: 1.0,
            shrink_min: 0.0,
            vertex_group_shrink: String::new(),
            use_sewing_springs: false,
            sewing_force_max: 0.0,
            use_dynamic_mesh: false,
            use_collision: true,
            collision_quality: 2,
            distance_min: 0.015,
            impulse_clamp: 0.0,
            collision_collection: String::new(),
            use_self_collision: false,
            self_friction: 5.0,
            self_distance_min: 0.015,
            effector_weights: EffectorWeights::default(),
            point_cache: PointCacheSettings::default(),
        }
    }
}

impl Props for ClothSettings {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.prop("quality", "Quality Steps", Prop::Int(&mut self.quality, 1, 80));
        v.prop("time_scale", "Speed Multiplier", Prop::Float(&mut self.time_scale, Range::new(0.0, 10.0, 0.1)));
        v.panel("cloth_physical", "Physical Properties", true, None);
        v.prop("mass", "Vertex Mass", Prop::Float(&mut self.mass, Range::new(0.0, 10.0, 0.01).unit("kg")));
        v.prop("air_damping", "Air Viscosity", Prop::Float(&mut self.air_damping, Range::new(0.0, 10.0, 0.1)));
        v.prop("bending_model", "Bending Model", Prop::Enum(&mut self.bending_model));
        v.panel("cloth_stiffness", "Stiffness", true, None);
        v.prop("tension_stiffness", "Tension", Prop::Float(&mut self.tension_stiffness, Range::new(0.0, 1.0e4, 0.1)));
        v.prop("compression_stiffness", "Compression", Prop::Float(&mut self.compression_stiffness, Range::new(0.0, 1.0e4, 0.1)));
        v.prop("shear_stiffness", "Shear", Prop::Float(&mut self.shear_stiffness, Range::new(0.0, 1.0e4, 0.1)));
        v.prop("bending_stiffness", "Bending", Prop::Float(&mut self.bending_stiffness, Range::new(0.0, 1.0e4, 0.1)));
        v.end();
        v.panel("cloth_damping", "Damping", true, None);
        v.prop("tension_damping", "Tension", Prop::Float(&mut self.tension_damping, Range::new(0.0, 50.0, 0.1)));
        v.prop("compression_damping", "Compression", Prop::Float(&mut self.compression_damping, Range::new(0.0, 50.0, 0.1)));
        v.prop("shear_damping", "Shear", Prop::Float(&mut self.shear_damping, Range::new(0.0, 50.0, 0.1)));
        v.prop("bending_damping", "Bending", Prop::Float(&mut self.bending_damping, Range::new(0.0, 1.0e3, 0.1)));
        v.end();
        v.end();
        v.panel("cloth_shape", "Shape", false, None);
        v.prop("vertex_group_mass", "Pin Group", Prop::Ref(&mut self.vertex_group_mass, RefKind::VertexGroup));
        v.prop("pin_stiffness", "Stiffness", Prop::Float(&mut self.pin_stiffness, Range::new(0.0, 50.0, 0.1)));
        let mut sew = self.use_sewing_springs;
        v.prop("use_sewing_springs", "Sewing", Prop::Bool(&mut sew));
        self.use_sewing_springs = sew;
        v.prop("sewing_force_max", "Max Sewing Force", Prop::Float(&mut self.sewing_force_max, Range::new(0.0, 1.0e4, 0.1)));
        v.prop("vertex_group_shrink", "Shrinking Group", Prop::Ref(&mut self.vertex_group_shrink, RefKind::VertexGroup));
        v.prop("shrink_min", "Shrinking Factor", Prop::Float(&mut self.shrink_min, Range::new(-1.0, 1.0, 0.01)));
        v.prop("use_dynamic_mesh", "Dynamic Mesh", Prop::Bool(&mut self.use_dynamic_mesh));
        v.end();
        let mut c = self.use_collision;
        v.panel("cloth_collisions", "Collisions", false, None);
        v.prop("collision_quality", "Quality", Prop::Int(&mut self.collision_quality, 1, 20));
        v.panel("cloth_object_collisions", "Object Collisions", true, Some(("use_collision", &mut c)));
        v.prop("distance_min", "Distance", Prop::Float(&mut self.distance_min, Range::new(0.001, 1.0, 0.001).unit("m")));
        v.prop("impulse_clamp", "Impulse Clamping", Prop::Float(&mut self.impulse_clamp, Range::new(0.0, 100.0, 0.1)));
        v.prop("collision_collection", "Collision Collection", Prop::Ref(&mut self.collision_collection, RefKind::Collection));
        v.end();
        self.use_collision = c;
        let mut sc = self.use_self_collision;
        v.panel("cloth_self_collisions", "Self Collisions", true, Some(("use_self_collision", &mut sc)));
        v.prop("self_friction", "Friction", Prop::Float(&mut self.self_friction, Range::new(0.0, 80.0, 0.1)));
        v.prop("self_distance_min", "Distance", Prop::Float(&mut self.self_distance_min, Range::new(0.001, 0.1, 0.001).unit("m")));
        v.end();
        self.use_self_collision = sc;
        v.end();
        let mut pr = self.use_pressure;
        v.panel("cloth_pressure", "Pressure", false, Some(("use_pressure", &mut pr)));
        v.prop("uniform_pressure_force", "Pressure", Prop::Float(&mut self.uniform_pressure_force, Range::new(-1.0e4, 1.0e4, 0.1)));
        v.end();
        self.use_pressure = pr;
        self.point_cache.visit(v);
        self.effector_weights.visit(v);
    }
}

// ---------------------------------------------------------------------------
// Soft body

/// Object > Physics > Soft Body.
#[derive(Clone, Debug, PartialEq)]
pub struct SoftBodySettings {
    pub collision_collection: String,
    pub friction: f32,
    pub mass: f32,
    pub vertex_group_mass: String,
    pub speed: f32,
    pub use_goal: bool,
    pub vertex_group_goal: String,
    pub goal_spring: f32,
    pub goal_friction: f32,
    pub goal_default: f32,
    pub goal_min: f32,
    pub goal_max: f32,
    pub use_edges: bool,
    pub pull: f32,
    pub push: f32,
    pub damping: f32,
    pub plastic: f32,
    pub bend: f32,
    pub spring_length: f32,
    pub use_edge_collision: bool,
    pub use_face_collision: bool,
    pub use_self_collision: bool,
    pub ball_size: f32,
    pub ball_stiff: f32,
    pub ball_damp: f32,
    pub step_min: i32,
    pub step_max: i32,
    pub effector_weights: EffectorWeights,
    pub point_cache: PointCacheSettings,
}

impl Default for SoftBodySettings {
    fn default() -> Self {
        SoftBodySettings {
            collision_collection: String::new(),
            friction: 0.5,
            mass: 1.0,
            vertex_group_mass: String::new(),
            speed: 1.0,
            use_goal: true,
            vertex_group_goal: String::new(),
            goal_spring: 0.5,
            goal_friction: 0.0,
            goal_default: 0.7,
            goal_min: 0.0,
            goal_max: 1.0,
            use_edges: true,
            pull: 0.5,
            push: 0.5,
            damping: 0.5,
            plastic: 0.0,
            bend: 0.0,
            spring_length: 0.0,
            use_edge_collision: false,
            use_face_collision: false,
            use_self_collision: false,
            ball_size: 0.49,
            ball_stiff: 1.0,
            ball_damp: 0.5,
            step_min: 10,
            step_max: 300,
            effector_weights: EffectorWeights::default(),
            point_cache: PointCacheSettings::default(),
        }
    }
}

impl Props for SoftBodySettings {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.prop("collision_collection", "Collision Collection", Prop::Ref(&mut self.collision_collection, RefKind::Collection));
        v.panel("sb_object", "Object", true, None);
        v.prop("friction", "Friction", Prop::Float(&mut self.friction, Range::new(0.0, 50.0, 0.1)));
        v.prop("mass", "Mass", Prop::Float(&mut self.mass, Range::new(0.0, 50000.0, 0.1).unit("kg")));
        v.prop("vertex_group_mass", "Control Point", Prop::Ref(&mut self.vertex_group_mass, RefKind::VertexGroup));
        v.end();
        v.panel("sb_simulation", "Simulation", true, None);
        v.prop("speed", "Speed", Prop::Float(&mut self.speed, Range::new(0.01, 100.0, 0.1)));
        v.end();
        let mut g = self.use_goal;
        v.panel("sb_goal", "Goal", true, Some(("use_goal", &mut g)));
        v.prop("vertex_group_goal", "Vertex Group", Prop::Ref(&mut self.vertex_group_goal, RefKind::VertexGroup));
        v.prop("goal_spring", "Stiffness", Prop::Float(&mut self.goal_spring, Range::new(0.0, 0.999, 0.01).slider()));
        v.prop("goal_friction", "Damping", Prop::Float(&mut self.goal_friction, Range::new(0.0, 50.0, 0.1)));
        v.prop("goal_default", "Default", Prop::Float(&mut self.goal_default, FAC));
        v.prop("goal_min", "Minimum", Prop::Float(&mut self.goal_min, FAC));
        v.prop("goal_max", "Maximum", Prop::Float(&mut self.goal_max, FAC));
        v.end();
        self.use_goal = g;
        let mut e = self.use_edges;
        v.panel("sb_edges", "Edges", true, Some(("use_edges", &mut e)));
        v.prop("pull", "Pull", Prop::Float(&mut self.pull, Range::new(0.0, 0.999, 0.01).slider()));
        v.prop("push", "Push", Prop::Float(&mut self.push, Range::new(0.0, 0.999, 0.01).slider()));
        v.prop("damping", "Damp", Prop::Float(&mut self.damping, Range::new(0.0, 50.0, 0.1)));
        v.prop("plastic", "Plasticity", Prop::Float(&mut self.plastic, Range::new(0.0, 100.0, 1.0)));
        v.prop("bend", "Bending", Prop::Float(&mut self.bend, Range::new(0.0, 10.0, 0.1)));
        v.prop("spring_length", "Length", Prop::Float(&mut self.spring_length, Range::new(0.0, 200.0, 1.0)));
        v.prop("use_edge_collision", "Edge Collision", Prop::Bool(&mut self.use_edge_collision));
        v.prop("use_face_collision", "Face Collision", Prop::Bool(&mut self.use_face_collision));
        v.end();
        self.use_edges = e;
        let mut s = self.use_self_collision;
        v.panel("sb_self_collision", "Self Collision", false, Some(("use_self_collision", &mut s)));
        v.prop("ball_size", "Ball Size", Prop::Float(&mut self.ball_size, Range::new(-10.0, 10.0, 0.01)));
        v.prop("ball_stiff", "Impact", Prop::Float(&mut self.ball_stiff, Range::new(0.001, 100.0, 0.1)));
        v.prop("ball_damp", "Damping", Prop::Float(&mut self.ball_damp, Range::new(0.001, 1.0, 0.01)));
        v.end();
        self.use_self_collision = s;
        v.panel("sb_solver", "Solver", false, None);
        v.prop("step_min", "Step Size Min", Prop::Int(&mut self.step_min, 0, 30000));
        v.prop("step_max", "Max", Prop::Int(&mut self.step_max, 0, 30000));
        v.end();
        self.point_cache.visit(v);
        self.effector_weights.visit(v);
    }
}

// ---------------------------------------------------------------------------
// Particles

prop_enum!(ParticleType { Emitter = ("EMITTER", "Emitter"), Hair = ("HAIR", "Hair") } default Emitter);
prop_enum!(EmitFrom { Vert = ("VERT", "Verts"), Face = ("FACE", "Faces"), Volume = ("VOLUME", "Volume") } default Face);
prop_enum!(Distribution { Jittered = ("JIT", "Jittered"), Random = ("RAND", "Random"), Grid = ("GRID", "Grid") } default Jittered);
prop_enum!(ParticlePhysics { None = ("NO", "None"), Newton = ("NEWTON", "Newtonian") } default Newton);
prop_enum!(Integrator { Euler = ("EULER", "Euler"), Verlet = ("VERLET", "Verlet"), Midpoint = ("MIDPOINT", "Midpoint"), Rk4 = ("RK4", "RK4") } default Midpoint);
prop_enum!(RenderType {
    None = ("NONE", "None"),
    Halo = ("HALO", "Halo"),
    Line = ("LINE", "Line"),
    Path = ("PATH", "Path"),
    Object = ("OBJECT", "Object"),
    Collection = ("COLLECTION", "Collection"),
} default Halo);
prop_enum!(DisplayMethod {
    None = ("NONE", "None"),
    Rendered = ("RENDER", "As Rendered"),
    Point = ("DOT", "Point"),
    Circle = ("CIRC", "Circle"),
    Cross = ("CROSS", "Cross"),
    Axis = ("AXIS", "Axis"),
} default Rendered);
prop_enum!(ChildType { None = ("NONE", "None"), Simple = ("SIMPLE", "Simple"), Interpolated = ("INTERPOLATED", "Interpolated") } default None);
prop_enum!(RotationMode {
    None = ("NONE", "None"),
    Normal = ("NOR", "Normal"),
    NormalTangent = ("NOR_TAN", "Normal-Tangent"),
    Velocity = ("VEL", "Velocity / Hair"),
    GlobalX = ("GLOB_X", "Global X"),
    GlobalY = ("GLOB_Y", "Global Y"),
    GlobalZ = ("GLOB_Z", "Global Z"),
} default Velocity);
prop_enum!(AngularVelocityMode { None = ("NONE", "None"), Velocity = ("VELOCITY", "Velocity"), Random = ("RAND", "Random"), GlobalZ = ("GLOBAL_Z", "Global Z") } default Velocity);

/// Blender's ParticleSettings (the part a particle system shares).
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleSettings {
    pub name: String,
    pub kind: ParticleType,
    pub count: i32,
    pub frame_start: f32,
    pub frame_end: f32,
    pub lifetime: f32,
    pub lifetime_random: f32,
    pub hair_length: f32,
    pub hair_step: i32,
    pub emit_from: EmitFrom,
    pub use_emit_random: bool,
    pub use_even_distribution: bool,
    pub distribution: Distribution,
    pub jitter_factor: f32,
    pub userjit: i32,
    pub grid_resolution: i32,
    pub normal_factor: f32,
    pub tangent_factor: f32,
    pub tangent_phase: f32,
    pub object_align_factor: [f32; 3],
    pub object_factor: f32,
    pub factor_random: f32,
    pub use_rotations: bool,
    pub rotation_mode: RotationMode,
    pub rotation_factor_random: f32,
    pub phase_factor: f32,
    pub phase_factor_random: f32,
    pub use_dynamic_rotation: bool,
    pub angular_velocity_mode: AngularVelocityMode,
    pub angular_velocity_factor: f32,
    pub physics_type: ParticlePhysics,
    pub mass: f32,
    pub use_multiply_size_mass: bool,
    pub particle_size: f32,
    pub size_random: f32,
    pub drag_factor: f32,
    pub brownian_factor: f32,
    pub damping: f32,
    pub integrator: Integrator,
    pub timestep: f32,
    pub subframes: i32,
    pub use_size_deflect: bool,
    pub use_die_on_collision: bool,
    pub collision_collection: String,
    pub render_type: RenderType,
    pub material_slot: i32,
    pub use_render_emitter: bool,
    pub show_unborn: bool,
    pub use_dead: bool,
    pub instance_object: String,
    pub instance_collection: String,
    pub use_rotation_instance: bool,
    pub use_scale_instance: bool,
    pub use_collection_pick_random: bool,
    pub line_length_tail: f32,
    pub line_length_head: f32,
    pub display_method: DisplayMethod,
    pub display_percentage: i32,
    pub display_size: f32,
    pub display_step: i32,
    pub render_step: i32,
    pub root_radius: f32,
    pub tip_radius: f32,
    pub radius_scale: f32,
    pub child_type: ChildType,
    pub child_nbr: i32,
    pub rendered_child_count: i32,
    pub child_length: f32,
    pub child_radius: f32,
    pub child_roundness: f32,
    pub clump_factor: f32,
    pub roughness_1: f32,
    pub effector_weights: EffectorWeights,
}

impl Default for ParticleSettings {
    fn default() -> Self {
        ParticleSettings {
            name: "ParticleSettings".into(),
            kind: ParticleType::Emitter,
            count: 1000,
            frame_start: 1.0,
            frame_end: 200.0,
            lifetime: 50.0,
            lifetime_random: 0.0,
            hair_length: 4.0,
            hair_step: 5,
            emit_from: EmitFrom::Face,
            use_emit_random: true,
            use_even_distribution: true,
            distribution: Distribution::Jittered,
            jitter_factor: 1.0,
            userjit: 0,
            grid_resolution: 10,
            normal_factor: 1.0,
            tangent_factor: 0.0,
            tangent_phase: 0.0,
            object_align_factor: [0.0; 3],
            object_factor: 0.0,
            factor_random: 0.0,
            use_rotations: false,
            rotation_mode: RotationMode::Velocity,
            rotation_factor_random: 0.0,
            phase_factor: 0.0,
            phase_factor_random: 0.0,
            use_dynamic_rotation: false,
            angular_velocity_mode: AngularVelocityMode::Velocity,
            angular_velocity_factor: 0.0,
            physics_type: ParticlePhysics::Newton,
            mass: 1.0,
            use_multiply_size_mass: false,
            particle_size: 0.05,
            size_random: 0.0,
            drag_factor: 0.0,
            brownian_factor: 0.0,
            damping: 0.0,
            integrator: Integrator::Midpoint,
            timestep: 0.04,
            subframes: 0,
            use_size_deflect: false,
            use_die_on_collision: false,
            collision_collection: String::new(),
            render_type: RenderType::Halo,
            material_slot: 0,
            use_render_emitter: true,
            show_unborn: false,
            use_dead: false,
            instance_object: String::new(),
            instance_collection: String::new(),
            use_rotation_instance: false,
            use_scale_instance: false,
            use_collection_pick_random: false,
            line_length_tail: 0.0,
            line_length_head: 1.0,
            display_method: DisplayMethod::Rendered,
            display_percentage: 100,
            display_size: 0.1,
            display_step: 3,
            render_step: 3,
            root_radius: 1.0,
            tip_radius: 0.0,
            radius_scale: 0.01,
            child_type: ChildType::None,
            child_nbr: 10,
            rendered_child_count: 100,
            child_length: 1.0,
            child_radius: 0.2,
            child_roundness: 0.0,
            clump_factor: 0.0,
            roughness_1: 0.0,
            effector_weights: EffectorWeights::default(),
        }
    }
}

impl ParticleSettings {
    /// Blender's settings when the type is switched to Hair.
    pub fn hair() -> ParticleSettings {
        ParticleSettings { kind: ParticleType::Hair, render_type: RenderType::Path, ..Default::default() }
    }
}

impl Props for ParticleSettings {
    fn visit(&mut self, v: &mut dyn Visitor) {
        let hair = self.kind == ParticleType::Hair;
        v.prop("type", "Type", Prop::Enum(&mut self.kind));
        v.panel("part_emission", "Emission", true, None);
        v.prop("count", "Number", Prop::Int(&mut self.count, 0, 10_000_000));
        if hair {
            v.prop("hair_length", "Hair Length", Prop::Float(&mut self.hair_length, Range::new(0.0, 1.0e3, 0.01).unit("m")));
            v.prop("hair_step", "Segments", Prop::Int(&mut self.hair_step, 2, 50));
        } else {
            v.prop("frame_start", "Frame Start", Prop::Float(&mut self.frame_start, Range::new(-1.0e6, 1.0e6, 1.0)));
            v.prop("frame_end", "End", Prop::Float(&mut self.frame_end, Range::new(-1.0e6, 1.0e6, 1.0)));
            v.prop("lifetime", "Lifetime", Prop::Float(&mut self.lifetime, Range::new(1.0, 1.0e6, 1.0)));
            v.prop("lifetime_random", "Lifetime Randomness", Prop::Float(&mut self.lifetime_random, FAC));
        }
        v.panel("part_source", "Source", true, None);
        v.prop("emit_from", "Emit From", Prop::Enum(&mut self.emit_from));
        v.prop("use_emit_random", "Random Order", Prop::Bool(&mut self.use_emit_random));
        v.prop("use_even_distribution", "Even Distribution", Prop::Bool(&mut self.use_even_distribution));
        v.active(self.emit_from != EmitFrom::Vert);
        v.prop("distribution", "Distribution", Prop::Enum(&mut self.distribution));
        v.prop("jitter_factor", "Jittering Amount", Prop::Float(&mut self.jitter_factor, Range::new(0.0, 2.0, 0.01)));
        v.prop("userjit", "Particles/Face", Prop::Int(&mut self.userjit, 0, 1000));
        v.prop("grid_resolution", "Resolution", Prop::Int(&mut self.grid_resolution, 1, 250));
        v.end();
        v.end();
        if !hair {
            v.panel("part_velocity", "Velocity", true, None);
            v.prop("normal_factor", "Normal", Prop::Float(&mut self.normal_factor, Range::new(-1000.0, 1000.0, 0.1).unit("m/s")));
            v.prop("tangent_factor", "Tangent", Prop::Float(&mut self.tangent_factor, Range::new(-1000.0, 1000.0, 0.1).unit("m/s")));
            v.prop("tangent_phase", "Tangent Phase", Prop::Float(&mut self.tangent_phase, Range::new(-1.0, 1.0, 0.01)));
            v.prop("object_align_factor", "Object Aligned", Prop::Vec3(&mut self.object_align_factor, Range::new(-200.0, 200.0, 0.1).unit("m/s")));
            v.prop("object_factor", "Object Velocity", Prop::Float(&mut self.object_factor, Range::new(-200.0, 200.0, 0.1)));
            v.prop("factor_random", "Randomize", Prop::Float(&mut self.factor_random, Range::new(0.0, 200.0, 0.1)));
            v.end();
            let mut r = self.use_rotations;
            v.panel("part_rotation", "Rotation", false, Some(("use_rotations", &mut r)));
            v.prop("rotation_mode", "Orientation Axis", Prop::Enum(&mut self.rotation_mode));
            v.prop("rotation_factor_random", "Randomize", Prop::Float(&mut self.rotation_factor_random, FAC));
            v.prop("phase_factor", "Phase", Prop::Float(&mut self.phase_factor, Range::new(-1.0, 1.0, 0.01)));
            v.prop("phase_factor_random", "Randomize Phase", Prop::Float(&mut self.phase_factor_random, Range::new(0.0, 2.0, 0.01)));
            v.prop("use_dynamic_rotation", "Dynamic", Prop::Bool(&mut self.use_dynamic_rotation));
            v.prop("angular_velocity_mode", "Angular Velocity Axis", Prop::Enum(&mut self.angular_velocity_mode));
            v.prop("angular_velocity_factor", "Amount", Prop::Float(&mut self.angular_velocity_factor, Range::new(-200.0, 200.0, 0.1)));
            v.end();
            self.use_rotations = r;
            v.panel("part_physics", "Physics", true, None);
            v.prop("physics_type", "Physics Type", Prop::Enum(&mut self.physics_type));
            v.active(self.physics_type == ParticlePhysics::Newton);
            v.panel("part_physics_properties", "Properties", true, None);
            v.prop("mass", "Mass", Prop::Float(&mut self.mass, Range::new(0.001, 1.0e5, 0.1).unit("kg")));
            v.prop("use_multiply_size_mass", "Multiply Mass with Size", Prop::Bool(&mut self.use_multiply_size_mass));
            v.prop("particle_size", "Size", Prop::Float(&mut self.particle_size, Range::new(0.001, 1.0e5, 0.01).unit("m")));
            v.prop("size_random", "Random Size", Prop::Float(&mut self.size_random, FAC));
            v.end();
            v.panel("part_physics_forces", "Forces", true, None);
            v.prop("brownian_factor", "Brownian", Prop::Float(&mut self.brownian_factor, Range::new(0.0, 200.0, 0.1)));
            v.prop("drag_factor", "Drag", Prop::Float(&mut self.drag_factor, FAC));
            v.prop("damping", "Damp", Prop::Float(&mut self.damping, FAC));
            v.end();
            v.panel("part_physics_integration", "Integration", false, None);
            v.prop("integrator", "Integrator", Prop::Enum(&mut self.integrator));
            v.prop("timestep", "Timestep", Prop::Float(&mut self.timestep, Range::new(0.0001, 100.0, 0.01)));
            v.prop("subframes", "Subframes", Prop::Int(&mut self.subframes, 0, 1000));
            v.end();
            v.panel("part_physics_deflection", "Deflection", false, None);
            v.prop("use_size_deflect", "Size Deflect", Prop::Bool(&mut self.use_size_deflect));
            v.prop("use_die_on_collision", "Die on Hit", Prop::Bool(&mut self.use_die_on_collision));
            v.prop("collision_collection", "Collision Collection", Prop::Ref(&mut self.collision_collection, RefKind::Collection));
            v.end();
            v.end();
        }
        v.panel("part_render", "Render", true, None);
        v.prop("render_type", "Render As", Prop::Enum(&mut self.render_type));
        v.prop("material_slot", "Material", Prop::Int(&mut self.material_slot, 0, 32767));
        v.prop("use_render_emitter", "Show Emitter", Prop::Bool(&mut self.use_render_emitter));
        v.prop("show_unborn", "Show Unborn", Prop::Bool(&mut self.show_unborn));
        v.prop("use_dead", "Show Dead", Prop::Bool(&mut self.use_dead));
        v.prop("render_step", "Steps", Prop::Int(&mut self.render_step, 0, 20));
        v.prop("line_length_tail", "Tail Length", Prop::Float(&mut self.line_length_tail, Range::new(0.0, 100.0, 0.01)));
        v.prop("line_length_head", "Head Length", Prop::Float(&mut self.line_length_head, Range::new(0.0, 100.0, 0.01)));
        v.panel("part_render_object", "Object", true, None);
        v.active(self.render_type == RenderType::Object);
        v.prop("instance_object", "Instance Object", Prop::Ref(&mut self.instance_object, RefKind::Object));
        v.prop("use_rotation_instance", "Global Rotation", Prop::Bool(&mut self.use_rotation_instance));
        v.prop("use_scale_instance", "Scale", Prop::Bool(&mut self.use_scale_instance));
        v.end();
        v.panel("part_render_collection", "Collection", true, None);
        v.active(self.render_type == RenderType::Collection);
        v.prop("instance_collection", "Instance Collection", Prop::Ref(&mut self.instance_collection, RefKind::Collection));
        v.prop("use_collection_pick_random", "Pick Random", Prop::Bool(&mut self.use_collection_pick_random));
        v.end();
        v.panel("part_render_hair", "Hair Shape", false, None);
        v.active(self.render_type == RenderType::Path);
        v.prop("root_radius", "Root", Prop::Float(&mut self.root_radius, Range::new(0.0, 1.0e3, 0.01)));
        v.prop("tip_radius", "Tip", Prop::Float(&mut self.tip_radius, Range::new(0.0, 1.0e3, 0.01)));
        v.prop("radius_scale", "Radius Scale", Prop::Float(&mut self.radius_scale, Range::new(0.0, 1.0e3, 0.001)));
        v.end();
        v.end();
        v.panel("part_display", "Viewport Display", false, None);
        v.prop("display_method", "Display As", Prop::Enum(&mut self.display_method));
        v.prop("display_size", "Size", Prop::Float(&mut self.display_size, Range::new(0.0, 1.0e3, 0.01).unit("m")));
        v.prop("display_percentage", "Amount", Prop::Int(&mut self.display_percentage, 0, 100));
        v.prop("display_step", "Strand Steps", Prop::Int(&mut self.display_step, 0, 10));
        v.end();
        v.panel("part_children", "Children", false, None);
        v.prop("child_type", "Children", Prop::Enum(&mut self.child_type));
        v.active(self.child_type != ChildType::None);
        v.prop("child_nbr", "Display Amount", Prop::Int(&mut self.child_nbr, 0, 100000));
        v.prop("rendered_child_count", "Render Amount", Prop::Int(&mut self.rendered_child_count, 0, 100000));
        v.prop("child_length", "Length", Prop::Float(&mut self.child_length, FAC));
        v.prop("child_radius", "Radius", Prop::Float(&mut self.child_radius, Range::new(0.0, 100.0, 0.01).unit("m")));
        v.prop("child_roundness", "Roundness", Prop::Float(&mut self.child_roundness, FAC));
        v.prop("clump_factor", "Clump", Prop::Float(&mut self.clump_factor, Range::new(-1.0, 1.0, 0.01).slider()));
        v.prop("roughness_1", "Roughness Uniform", Prop::Float(&mut self.roughness_1, Range::new(0.0, 100.0, 0.01)));
        v.end();
        self.effector_weights.visit(v);
    }
}

/// One particle system on an object (Blender's ParticleSystem with its own
/// settings; settings data-blocks are not shared between systems here).
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleSystem {
    pub name: String,
    pub seed: i32,
    pub settings: ParticleSettings,
    pub point_cache: PointCacheSettings,
}

impl Default for ParticleSystem {
    fn default() -> Self {
        ParticleSystem { name: "ParticleSystem".into(), seed: 0, settings: ParticleSettings::default(), point_cache: PointCacheSettings::default() }
    }
}

impl Props for ParticleSystem {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.prop("seed", "Seed", Prop::Int(&mut self.seed, 0, i32::MAX));
        self.settings.visit(v);
        if self.settings.kind == ParticleType::Emitter {
            self.point_cache.visit(v);
        }
    }
}

// ---------------------------------------------------------------------------
// Fluid

prop_enum!(FluidType { Domain = ("DOMAIN", "Domain"), Flow = ("FLOW", "Flow"), Effector = ("EFFECTOR", "Effector") } default Domain);
prop_enum!(DomainType { Gas = ("GAS", "Gas"), Liquid = ("LIQUID", "Liquid") } default Gas);
prop_enum!(FlowType { Smoke = ("SMOKE", "Smoke"), Both = ("BOTH", "Fire + Smoke"), Fire = ("FIRE", "Fire"), Liquid = ("LIQUID", "Liquid") } default Smoke);
prop_enum!(FlowBehavior { Inflow = ("INFLOW", "Inflow"), Outflow = ("OUTFLOW", "Outflow"), Geometry = ("GEOMETRY", "Geometry") } default Geometry);
prop_enum!(EffectorType { Collision = ("COLLISION", "Collision"), Guide = ("GUIDE", "Guide") } default Collision);
prop_enum!(FlowSource { Mesh = ("MESH", "Mesh"), Particles = ("PARTICLES", "Particle System") } default Mesh);
prop_enum!(SimulationMethod { Flip = ("FLIP", "FLIP"), Apic = ("APIC", "APIC") } default Flip);
prop_enum!(MeshGenerator { Improved = ("IMPROVED", "Final"), Union = ("UNION", "Preview") } default Improved);
prop_enum!(FluidCacheType { Replay = ("REPLAY", "Replay"), Modular = ("MODULAR", "Modular"), All = ("ALL", "All") } default Replay);

/// Object > Physics > Fluid: the Domain, Flow and Effector settings (only
/// the block for `fluid_type` is used, as in Blender's Fluid modifier).
#[derive(Clone, Debug, PartialEq)]
pub struct FluidSettings {
    pub fluid_type: FluidType,
    // Domain
    pub domain_type: DomainType,
    pub resolution_max: i32,
    pub time_scale: f32,
    pub cfl_condition: f32,
    pub use_adaptive_timesteps: bool,
    pub timesteps_max: i32,
    pub timesteps_min: i32,
    /// Blender's "Buoyancy Density" and "Heat".
    pub alpha: f32,
    pub beta: f32,
    pub vorticity: f32,
    pub use_dissolve_smoke: bool,
    pub dissolve_speed: f32,
    pub use_dissolve_smoke_log: bool,
    pub burning_rate: f32,
    pub flame_smoke: f32,
    pub flame_vorticity: f32,
    pub flame_max_temp: f32,
    pub use_collision_border: bool,
    /// Gas: "Empty Space" (densities below are empty).
    pub clipping: f32,
    /// Domain gravity, used when the scene's gravity is off.
    pub gravity: [f32; 3],
    pub delete_in_obstacle: bool,
    /// Liquid: Border Collisions (Blender: all open by default; Quick
    /// Liquid closes them). Front is -Y, right +X, top +Z.
    pub use_collision_border_front: bool,
    pub use_collision_border_back: bool,
    pub use_collision_border_right: bool,
    pub use_collision_border_left: bool,
    pub use_collision_border_top: bool,
    pub use_collision_border_bottom: bool,
    // Liquid
    pub use_flip_particles: bool,
    pub simulation_method: SimulationMethod,
    pub flip_ratio: f32,
    pub sys_particle_maximum: i32,
    pub particle_radius: f32,
    pub particle_number: i32,
    pub particle_randomness: f32,
    pub particle_max: i32,
    pub particle_min: i32,
    pub particle_band_width: f32,
    pub use_fractions: bool,
    pub fractions_distance: f32,
    pub fractions_threshold: f32,
    // Liquid > Mesh
    pub use_mesh: bool,
    pub mesh_scale: i32,
    pub mesh_particle_radius: f32,
    pub use_speed_vectors: bool,
    pub mesh_generator: MeshGenerator,
    pub mesh_smoothen_pos: i32,
    pub mesh_smoothen_neg: i32,
    pub mesh_concave_upper: f32,
    pub mesh_concave_lower: f32,
    // Liquid > Diffusion
    pub use_diffusion: bool,
    pub viscosity_base: f32,
    pub viscosity_exponent: i32,
    pub surface_tension: f32,
    pub use_viscosity: bool,
    pub viscosity_value: f32,
    // Collections
    pub fluid_group: String,
    pub effector_group: String,
    // Cache
    pub cache_type: FluidCacheType,
    pub cache_frame_offset: i32,
    pub cache_resumable: bool,
    pub cache_frame_start: i32,
    pub cache_frame_end: i32,
    pub display_thickness: f32,
    pub effector_weights: EffectorWeights,
    // Flow
    pub flow_type: FlowType,
    pub flow_behavior: FlowBehavior,
    pub use_flow: bool,
    /// Inflow/Outflow: "Use Flow" (Blender's `use_inflow`).
    pub use_inflow: bool,
    /// Flow and Effector: "Sampling Substeps".
    pub subframes: i32,
    pub flow_source: FlowSource,
    pub use_absolute: bool,
    pub surface_distance: f32,
    pub use_plane_init: bool,
    pub volume_density: f32,
    pub density: f32,
    pub smoke_color: [f32; 3],
    pub fuel_amount: f32,
    pub temperature: f32,
    pub use_initial_velocity: bool,
    pub velocity_factor: f32,
    pub velocity_normal: f32,
    pub velocity_coord: [f32; 3],
    // Effector
    pub effector_type: EffectorType,
    pub use_effector: bool,
    pub effector_surface_distance: f32,
}

impl Default for FluidSettings {
    fn default() -> Self {
        FluidSettings {
            fluid_type: FluidType::Domain,
            domain_type: DomainType::Gas,
            resolution_max: 32,
            time_scale: 1.0,
            cfl_condition: 4.0,
            use_adaptive_timesteps: true,
            timesteps_max: 4,
            timesteps_min: 1,
            alpha: 1.0,
            beta: 1.0,
            vorticity: 0.0,
            use_dissolve_smoke: false,
            dissolve_speed: 5.0,
            use_dissolve_smoke_log: false,
            burning_rate: 0.75,
            flame_smoke: 1.0,
            flame_vorticity: 0.5,
            flame_max_temp: 3.0,
            use_collision_border: true,
            clipping: 1e-6,
            gravity: [0.0, 0.0, -9.81],
            delete_in_obstacle: false,
            use_collision_border_front: false,
            use_collision_border_back: false,
            use_collision_border_right: false,
            use_collision_border_left: false,
            use_collision_border_top: false,
            use_collision_border_bottom: false,
            use_flip_particles: false,
            simulation_method: SimulationMethod::Flip,
            flip_ratio: 0.97,
            sys_particle_maximum: 0,
            particle_radius: 1.0,
            particle_number: 2,
            particle_randomness: 0.1,
            particle_max: 16,
            particle_min: 8,
            particle_band_width: 3.0,
            use_fractions: false,
            fractions_distance: 0.5,
            fractions_threshold: 0.05,
            use_mesh: true,
            mesh_scale: 2,
            mesh_particle_radius: 2.0,
            use_speed_vectors: false,
            mesh_generator: MeshGenerator::Improved,
            mesh_smoothen_pos: 1,
            mesh_smoothen_neg: 1,
            mesh_concave_upper: 3.5,
            mesh_concave_lower: 0.4,
            use_diffusion: false,
            viscosity_base: 1.0,
            viscosity_exponent: 6,
            surface_tension: 0.0,
            use_viscosity: false,
            viscosity_value: 0.05,
            fluid_group: String::new(),
            effector_group: String::new(),
            cache_type: FluidCacheType::Replay,
            cache_frame_offset: 0,
            cache_resumable: false,
            cache_frame_start: 1,
            cache_frame_end: 250,
            display_thickness: 1.0,
            effector_weights: EffectorWeights::default(),
            flow_type: FlowType::Smoke,
            flow_behavior: FlowBehavior::Geometry,
            use_flow: true,
            use_inflow: true,
            subframes: 0,
            flow_source: FlowSource::Mesh,
            use_absolute: true,
            surface_distance: 1.5,
            use_plane_init: false,
            volume_density: 0.0,
            density: 1.0,
            smoke_color: [0.7, 0.7, 0.7],
            fuel_amount: 1.0,
            temperature: 1.0,
            use_initial_velocity: false,
            velocity_factor: 1.0,
            velocity_normal: 0.0,
            velocity_coord: [0.0; 3],
            effector_type: EffectorType::Collision,
            use_effector: true,
            effector_surface_distance: 0.0,
        }
    }
}

impl FluidSettings {
    pub fn new(t: FluidType) -> FluidSettings {
        FluidSettings { fluid_type: t, ..Default::default() }
    }
}

impl FluidSettings {
    /// The liquid's kinematic viscosity in m²/s (Diffusion's Base ×
    /// 10^-Exponent, plus the High Viscosity Solver's Strength).
    pub fn kinematic_viscosity(&self) -> f32 {
        let mut nu = 0.0;
        if self.use_diffusion {
            nu += self.viscosity_base * 10f32.powi(-self.viscosity_exponent);
        }
        if self.use_viscosity {
            nu += self.viscosity_value;
        }
        nu
    }

    /// Closed domain sides: [-X, +X, -Y, +Y, -Z, +Z].
    pub fn closed_borders(&self) -> [bool; 6] {
        [
            self.use_collision_border_left,
            self.use_collision_border_right,
            self.use_collision_border_front,
            self.use_collision_border_back,
            self.use_collision_border_bottom,
            self.use_collision_border_top,
        ]
    }
}

impl Props for FluidSettings {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.prop("fluid_type", "Type", Prop::Enum(&mut self.fluid_type));
        match self.fluid_type {
            FluidType::Domain => {
                let liquid = self.domain_type == DomainType::Liquid;
                v.panel("fluid_settings", "Settings", true, None);
                v.prop("domain_type", "Domain Type", Prop::Enum(&mut self.domain_type));
                v.prop("resolution_max", "Resolution Divisions", Prop::Int(&mut self.resolution_max, 6, 512));
                v.prop("time_scale", "Time Scale", Prop::Float(&mut self.time_scale, Range::new(0.0001, 10.0, 0.1)));
                v.prop("use_adaptive_timesteps", "Use Adaptive Time Steps", Prop::Bool(&mut self.use_adaptive_timesteps));
                v.active(self.use_adaptive_timesteps);
                v.prop("cfl_condition", "CFL Number", Prop::Float(&mut self.cfl_condition, Range::new(0.0, 10.0, 0.1)));
                v.prop("timesteps_max", "Timesteps Maximum", Prop::Int(&mut self.timesteps_max, 1, 100));
                v.prop("timesteps_min", "Minimum", Prop::Int(&mut self.timesteps_min, 1, 100));
                v.active(true);
                v.prop("gravity", "Gravity", Prop::Vec3(&mut self.gravity, Range::new(-1000.0, 1000.0, 0.1).unit("m/s²")));
                if !liquid {
                    v.prop("clipping", "Empty Space", Prop::Float(&mut self.clipping, Range::new(0.0, 1.0, 0.001)));
                    v.prop("use_collision_border", "Border Collisions", Prop::Bool(&mut self.use_collision_border));
                }
                v.prop("delete_in_obstacle", "Delete in Obstacle", Prop::Bool(&mut self.delete_in_obstacle));
                if liquid {
                    v.panel("fluid_borders", "Border Collisions", false, None);
                    v.prop("use_collision_border_front", "Front", Prop::Bool(&mut self.use_collision_border_front));
                    v.prop("use_collision_border_back", "Back", Prop::Bool(&mut self.use_collision_border_back));
                    v.prop("use_collision_border_right", "Right", Prop::Bool(&mut self.use_collision_border_right));
                    v.prop("use_collision_border_left", "Left", Prop::Bool(&mut self.use_collision_border_left));
                    v.prop("use_collision_border_top", "Top", Prop::Bool(&mut self.use_collision_border_top));
                    v.prop("use_collision_border_bottom", "Bottom", Prop::Bool(&mut self.use_collision_border_bottom));
                    v.end();
                } else {
                    v.panel("fluid_gas", "Gas", true, None);
                    v.prop("alpha", "Buoyancy Density", Prop::Float(&mut self.alpha, Range::new(-5.0, 5.0, 0.1)));
                    v.prop("beta", "Heat", Prop::Float(&mut self.beta, Range::new(-5.0, 5.0, 0.1)));
                    v.prop("vorticity", "Vorticity", Prop::Float(&mut self.vorticity, Range::new(0.0, 4.0, 0.01)));
                    v.end();
                    let mut d = self.use_dissolve_smoke;
                    v.panel("fluid_dissolve", "Dissolve", false, Some(("use_dissolve_smoke", &mut d)));
                    v.prop("dissolve_speed", "Time", Prop::Float(&mut self.dissolve_speed, Range::new(1.0, 10000.0, 1.0)));
                    v.prop("use_dissolve_smoke_log", "Slow", Prop::Bool(&mut self.use_dissolve_smoke_log));
                    v.end();
                    self.use_dissolve_smoke = d;
                    v.panel("fluid_fire", "Fire", false, None);
                    v.prop("burning_rate", "Reaction Speed", Prop::Float(&mut self.burning_rate, Range::new(0.01, 4.0, 0.01)));
                    v.prop("flame_smoke", "Flame Smoke", Prop::Float(&mut self.flame_smoke, Range::new(0.0, 8.0, 0.1)));
                    v.prop("flame_vorticity", "Vorticity", Prop::Float(&mut self.flame_vorticity, Range::new(0.0, 2.0, 0.1)));
                    v.prop("flame_max_temp", "Temperature Maximum", Prop::Float(&mut self.flame_max_temp, Range::new(1.0, 10.0, 0.1)));
                    v.end();
                }
                v.end();
                if liquid {
                    v.panel("fluid_liquid", "Liquid", true, None);
                    v.prop("use_flip_particles", "Show FLIP Particles", Prop::Bool(&mut self.use_flip_particles));
                    v.prop("simulation_method", "Simulation Method", Prop::Enum(&mut self.simulation_method));
                    v.active(self.simulation_method == SimulationMethod::Flip);
                    v.prop("flip_ratio", "FLIP Ratio", Prop::Float(&mut self.flip_ratio, FAC));
                    v.active(true);
                    v.prop("sys_particle_maximum", "System Maximum", Prop::Int(&mut self.sys_particle_maximum, 0, i32::MAX));
                    v.prop("particle_radius", "Particle Radius", Prop::Float(&mut self.particle_radius, Range::new(0.0, 10.0, 0.1)));
                    v.prop("particle_number", "Sampling", Prop::Int(&mut self.particle_number, 1, 5));
                    v.prop("particle_randomness", "Randomness", Prop::Float(&mut self.particle_randomness, Range::new(0.0, 10.0, 0.01)));
                    v.prop("particle_max", "Particles Maximum", Prop::Int(&mut self.particle_max, 0, 1000));
                    v.prop("particle_min", "Minimum", Prop::Int(&mut self.particle_min, 0, 1000));
                    v.prop("particle_band_width", "Narrow Band Width", Prop::Float(&mut self.particle_band_width, Range::new(0.0, 1000.0, 0.1)));
                    v.prop("use_fractions", "Fractional Obstacles", Prop::Bool(&mut self.use_fractions));
                    v.active(self.use_fractions);
                    v.prop("fractions_distance", "Obstacle Distance", Prop::Float(&mut self.fractions_distance, Range::new(-5.0, 5.0, 0.01)));
                    v.prop("fractions_threshold", "Threshold", Prop::Float(&mut self.fractions_threshold, Range::new(0.001, 1.0, 0.01)));
                    v.active(true);
                    let mut m = self.use_mesh;
                    v.panel("fluid_mesh", "Mesh", false, Some(("use_mesh", &mut m)));
                    v.prop("mesh_scale", "Upres Factor", Prop::Int(&mut self.mesh_scale, 1, 4));
                    v.prop("mesh_particle_radius", "Particle Radius", Prop::Float(&mut self.mesh_particle_radius, Range::new(0.0, 10.0, 0.1)));
                    v.prop("use_speed_vectors", "Speed Vectors", Prop::Bool(&mut self.use_speed_vectors));
                    v.prop("mesh_generator", "Mesh Generator", Prop::Enum(&mut self.mesh_generator));
                    v.active(self.mesh_generator == MeshGenerator::Improved);
                    v.prop("mesh_smoothen_pos", "Smoothing Positive", Prop::Int(&mut self.mesh_smoothen_pos, 0, 100));
                    v.prop("mesh_smoothen_neg", "Negative", Prop::Int(&mut self.mesh_smoothen_neg, 0, 100));
                    v.prop("mesh_concave_upper", "Concavity Upper", Prop::Float(&mut self.mesh_concave_upper, Range::new(0.0, 10.0, 0.1)));
                    v.prop("mesh_concave_lower", "Lower", Prop::Float(&mut self.mesh_concave_lower, Range::new(0.0, 10.0, 0.1)));
                    v.active(true);
                    v.end();
                    self.use_mesh = m;
                    let mut df = self.use_diffusion;
                    v.panel("fluid_diffusion", "Diffusion", false, Some(("use_diffusion", &mut df)));
                    v.prop("viscosity_base", "Base", Prop::Float(&mut self.viscosity_base, Range::new(0.0, 10.0, 0.1)));
                    v.prop("viscosity_exponent", "Exponent", Prop::Int(&mut self.viscosity_exponent, 0, 10));
                    v.prop("surface_tension", "Surface Tension", Prop::Float(&mut self.surface_tension, Range::new(0.0, 100.0, 0.01)));
                    let mut hv = self.use_viscosity;
                    v.panel("fluid_viscosity", "High Viscosity Solver", false, Some(("use_viscosity", &mut hv)));
                    v.prop("viscosity_value", "Strength", Prop::Float(&mut self.viscosity_value, Range::new(0.0, 10.0, 0.01)));
                    v.end();
                    self.use_viscosity = hv;
                    v.end();
                    self.use_diffusion = df;
                    v.end();
                }
                v.panel("fluid_collections", "Collections", false, None);
                v.prop("fluid_group", "Flow", Prop::Ref(&mut self.fluid_group, RefKind::Collection));
                v.prop("effector_group", "Effector", Prop::Ref(&mut self.effector_group, RefKind::Collection));
                v.end();
                v.panel("fluid_cache", "Cache", false, None);
                v.prop("cache_frame_start", "Frame Start", Prop::Int(&mut self.cache_frame_start, -100000, 1000000));
                v.prop("cache_frame_end", "End", Prop::Int(&mut self.cache_frame_end, 1, 1000000));
                v.active(self.cache_type != FluidCacheType::Replay);
                v.prop("cache_frame_offset", "Offset", Prop::Int(&mut self.cache_frame_offset, -1000000, 1000000));
                v.active(true);
                v.prop("cache_type", "Type", Prop::Enum(&mut self.cache_type));
                v.prop("cache_resumable", "Resumable", Prop::Bool(&mut self.cache_resumable));
                v.end();
                v.panel("fluid_display", "Viewport Display", false, None);
                v.prop("display_thickness", "Thickness", Prop::Float(&mut self.display_thickness, Range::new(0.001, 1000.0, 0.1)));
                v.end();
                self.effector_weights.visit(v);
            }
            FluidType::Flow => {
                let liquid = self.flow_type == FlowType::Liquid;
                v.panel("fluid_settings", "Settings", true, None);
                v.prop("flow_type", "Flow Type", Prop::Enum(&mut self.flow_type));
                v.prop("flow_behavior", "Flow Behavior", Prop::Enum(&mut self.flow_behavior));
                if self.flow_behavior != FlowBehavior::Geometry {
                    v.prop("use_inflow", "Use Flow", Prop::Bool(&mut self.use_inflow));
                }
                v.prop("use_flow", "Enabled", Prop::Bool(&mut self.use_flow));
                v.prop("subframes", "Sampling Substeps", Prop::Int(&mut self.subframes, 0, 200));
                if !liquid && self.flow_behavior != FlowBehavior::Outflow {
                    if matches!(self.flow_type, FlowType::Smoke | FlowType::Both) {
                        v.prop("smoke_color", "Smoke Color", Prop::Color(&mut self.smoke_color));
                    }
                    v.prop("use_absolute", "Absolute Density", Prop::Bool(&mut self.use_absolute));
                    if matches!(self.flow_type, FlowType::Smoke | FlowType::Both) {
                        v.prop("temperature", "Initial Temperature", Prop::Float(&mut self.temperature, Range::new(-10.0, 10.0, 0.1)));
                        v.prop("density", "Density", Prop::Float(&mut self.density, Range::new(0.0, 10.0, 0.1)));
                    }
                    if matches!(self.flow_type, FlowType::Fire | FlowType::Both) {
                        v.prop("fuel_amount", "Fuel", Prop::Float(&mut self.fuel_amount, Range::new(0.0, 10.0, 0.1)));
                    }
                }
                v.panel("fluid_flow_source", "Flow Source", false, None);
                v.prop("flow_source", "Flow Source", Prop::Enum(&mut self.flow_source));
                v.prop("use_plane_init", "Is Planar", Prop::Bool(&mut self.use_plane_init));
                v.prop("surface_distance", "Surface Emission", Prop::Float(&mut self.surface_distance, Range::new(0.0, 10.0, 0.1)));
                if !liquid {
                    v.prop("volume_density", "Volume Emission", Prop::Float(&mut self.volume_density, FAC));
                }
                v.end();
                v.end();
                let mut iv = self.use_initial_velocity;
                v.panel("fluid_initial_velocity", "Initial Velocity", false, Some(("use_initial_velocity", &mut iv)));
                v.prop("velocity_factor", "Source", Prop::Float(&mut self.velocity_factor, Range::new(-100.0, 100.0, 0.1)));
                v.prop("velocity_normal", "Normal", Prop::Float(&mut self.velocity_normal, Range::new(-100.0, 100.0, 0.1)));
                v.prop("velocity_coord", "Initial", Prop::Vec3(&mut self.velocity_coord, Range::new(-1000.0, 1000.0, 0.1).unit("m/s")));
                v.end();
                self.use_initial_velocity = iv;
            }
            FluidType::Effector => {
                v.panel("fluid_settings", "Settings", true, None);
                v.prop("effector_type", "Effector Type", Prop::Enum(&mut self.effector_type));
                v.prop("subframes", "Sampling Substeps", Prop::Int(&mut self.subframes, 0, 200));
                v.prop("effector_surface_distance", "Surface Thickness", Prop::Float(&mut self.effector_surface_distance, Range::new(0.0, 10.0, 0.01)));
                v.prop("use_effector", "Use Effector", Prop::Bool(&mut self.use_effector));
                v.prop("use_plane_init", "Is Planar", Prop::Bool(&mut self.use_plane_init));
                v.end();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dynamic Paint

prop_enum!(DpUiType { Canvas = ("CANVAS", "Canvas"), Brush = ("BRUSH", "Brush") } default Canvas);
prop_enum!(DpSurfaceFormat { Vertex = ("VERTEX", "Vertex"), Image = ("IMAGE", "Image Sequence") } default Vertex);
prop_enum!(DpSurfaceType { Paint = ("PAINT", "Paint"), Displace = ("DISPLACE", "Displace"), Weight = ("WEIGHT", "Weight"), Wave = ("WAVE", "Waves") } default Paint);
prop_enum!(DpInitColorType { None = ("NONE", "None"), Color = ("COLOR", "Color") } default None);
prop_enum!(DpPaintSource {
    Volume = ("VOLUME", "Mesh Volume"),
    Distance = ("DISTANCE", "Proximity"),
    VolumeDistance = ("VOLUME_DISTANCE", "Mesh Volume + Proximity"),
    Point = ("POINT", "Object Center"),
} default Volume);
prop_enum!(DpFalloff { Smooth = ("SMOOTH", "Smooth"), Constant = ("CONSTANT", "Constant") } default Constant);
prop_enum!(DpWaveType { Change = ("CHANGE", "Depth Change"), Depth = ("DEPTH", "Obstacle"), Force = ("FORCE", "Force"), Reflect = ("REFLECT", "Reflect Only") } default Depth);

/// Object > Physics > Dynamic Paint (Blender's DynamicPaintModifier with
/// its canvas, one surface, and its brush settings).
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicPaintSettings {
    pub ui_type: DpUiType,
    pub use_canvas: bool,
    pub use_brush: bool,
    // Canvas surface
    pub surface_name: String,
    pub surface_format: DpSurfaceFormat,
    pub image_resolution: i32,
    pub use_antialiasing: bool,
    pub frame_start: i32,
    pub frame_end: i32,
    pub frame_substeps: i32,
    pub surface_type: DpSurfaceType,
    pub depth_clamp: f32,
    pub displace_factor: f32,
    pub use_incremental_displace: bool,
    pub use_wave_open_border: bool,
    pub wave_timescale: f32,
    pub wave_speed: f32,
    pub wave_damping: f32,
    pub wave_spring: f32,
    pub wave_smoothness: f32,
    pub brush_collection: String,
    pub brush_influence_scale: f32,
    pub brush_radius_scale: f32,
    pub use_drying: bool,
    pub dry_speed: i32,
    pub color_dry_threshold: f32,
    pub use_dry_log: bool,
    pub use_dissolve: bool,
    pub dissolve_speed: i32,
    pub use_dissolve_log: bool,
    pub use_output_a: bool,
    pub output_name_a: String,
    pub use_output_b: bool,
    pub output_name_b: String,
    pub init_color_type: DpInitColorType,
    pub init_color: [f32; 3],
    pub use_spread: bool,
    pub spread_speed: f32,
    pub color_spread_speed: f32,
    pub use_drip: bool,
    pub drip_velocity: f32,
    pub drip_acceleration: f32,
    pub use_shrink: bool,
    pub shrink_speed: f32,
    // Brush
    pub paint_color: [f32; 3],
    pub paint_alpha: f32,
    pub paint_wetness: f32,
    pub use_absolute_alpha: bool,
    pub use_paint_erase: bool,
    pub paint_source: DpPaintSource,
    pub paint_distance: f32,
    pub proximity_falloff: DpFalloff,
    pub invert_proximity: bool,
    pub use_negative_volume: bool,
    pub wave_type: DpWaveType,
    pub wave_factor: f32,
    pub wave_clamp: f32,
    pub use_smudge: bool,
    pub smudge_strength: f32,
}

impl Default for DynamicPaintSettings {
    fn default() -> Self {
        DynamicPaintSettings {
            ui_type: DpUiType::Canvas,
            use_canvas: false,
            use_brush: false,
            surface_name: "Surface".into(),
            surface_format: DpSurfaceFormat::Vertex,
            image_resolution: 256,
            use_antialiasing: false,
            frame_start: 1,
            frame_end: 250,
            frame_substeps: 0,
            surface_type: DpSurfaceType::Paint,
            depth_clamp: 0.0,
            displace_factor: 1.0,
            use_incremental_displace: false,
            use_wave_open_border: false,
            wave_timescale: 1.0,
            wave_speed: 1.0,
            wave_damping: 0.04,
            wave_spring: 0.2,
            wave_smoothness: 1.0,
            brush_collection: String::new(),
            brush_influence_scale: 1.0,
            brush_radius_scale: 1.0,
            use_drying: true,
            dry_speed: 14,
            color_dry_threshold: 1.0,
            use_dry_log: true,
            use_dissolve: false,
            dissolve_speed: 250,
            use_dissolve_log: true,
            use_output_a: true,
            output_name_a: "dp_paintmap".into(),
            use_output_b: true,
            output_name_b: "dp_wetmap".into(),
            init_color_type: DpInitColorType::None,
            init_color: [0.0, 0.0, 0.0],
            use_spread: false,
            spread_speed: 1.0,
            color_spread_speed: 1.0,
            use_drip: false,
            drip_velocity: 1.0,
            drip_acceleration: 1.0,
            use_shrink: false,
            shrink_speed: 1.0,
            paint_color: [0.15, 0.15, 0.15],
            paint_alpha: 1.0,
            paint_wetness: 1.0,
            use_absolute_alpha: false,
            use_paint_erase: false,
            paint_source: DpPaintSource::Volume,
            paint_distance: 1.0,
            proximity_falloff: DpFalloff::Constant,
            invert_proximity: false,
            use_negative_volume: false,
            wave_type: DpWaveType::Depth,
            wave_factor: 1.0,
            wave_clamp: 0.0,
            use_smudge: false,
            smudge_strength: 0.3,
        }
    }
}

impl DynamicPaintSettings {
    /// The vertex group a Weight surface writes (Blender renames the first
    /// output to `dp_weight` for Weight surfaces).
    pub fn weight_group(&self) -> &str {
        if self.output_name_a == "dp_paintmap" {
            "dp_weight"
        } else {
            &self.output_name_a
        }
    }
}

impl Props for DynamicPaintSettings {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.prop("ui_type", "Type", Prop::Enum(&mut self.ui_type));
        v.panel("dp_settings", "Settings", true, None);
        match self.ui_type {
            DpUiType::Canvas => {
                v.prop("use_canvas", "Canvas", Prop::Bool(&mut self.use_canvas));
                if self.use_canvas {
                    v.prop("surface_name", "Surface", Prop::Ref(&mut self.surface_name, RefKind::Text));
                    v.prop("surface_format", "Format", Prop::Enum(&mut self.surface_format));
                    if self.surface_format == DpSurfaceFormat::Image {
                        v.prop("image_resolution", "Resolution", Prop::Int(&mut self.image_resolution, 16, 4096));
                        v.prop("use_antialiasing", "Anti-Aliasing", Prop::Bool(&mut self.use_antialiasing));
                    }
                    v.prop("frame_start", "Frame Start", Prop::Int(&mut self.frame_start, 1, 1048574));
                    v.prop("frame_end", "End", Prop::Int(&mut self.frame_end, 1, 1048574));
                    v.prop("frame_substeps", "Substeps", Prop::Int(&mut self.frame_substeps, 0, 20));
                }
            }
            DpUiType::Brush => {
                v.prop("use_brush", "Brush", Prop::Bool(&mut self.use_brush));
                if self.use_brush {
                    v.prop("paint_color", "Base Color", Prop::Color(&mut self.paint_color));
                    v.prop("paint_alpha", "Alpha", Prop::Float(&mut self.paint_alpha, FAC));
                    v.prop("paint_wetness", "Wetness", Prop::Float(&mut self.paint_wetness, FAC));
                    v.prop("use_absolute_alpha", "Absolute Alpha", Prop::Bool(&mut self.use_absolute_alpha));
                    v.prop("use_paint_erase", "Erase Paint", Prop::Bool(&mut self.use_paint_erase));
                }
            }
        }
        v.end();
        if self.ui_type == DpUiType::Canvas && self.use_canvas {
            let t = self.surface_type;
            v.panel("dp_surface", "Surface", true, None);
            v.prop("surface_type", "Surface Type", Prop::Enum(&mut self.surface_type));
            match t {
                DpSurfaceType::Displace => {
                    v.prop("depth_clamp", "Max Displace", Prop::Float(&mut self.depth_clamp, Range::new(0.0, 50.0, 0.01)));
                    v.prop("displace_factor", "Displace Factor", Prop::Float(&mut self.displace_factor, Range::new(-50.0, 50.0, 0.1)));
                    v.prop("use_incremental_displace", "Incremental", Prop::Bool(&mut self.use_incremental_displace));
                }
                DpSurfaceType::Wave => {
                    v.prop("use_wave_open_border", "Open Borders", Prop::Bool(&mut self.use_wave_open_border));
                    v.prop("wave_timescale", "Timescale", Prop::Float(&mut self.wave_timescale, Range::new(0.01, 3.0, 0.01)));
                    v.prop("wave_speed", "Speed", Prop::Float(&mut self.wave_speed, Range::new(0.01, 5.0, 0.01)));
                    v.prop("wave_damping", "Damping", Prop::Float(&mut self.wave_damping, FAC));
                    v.prop("wave_spring", "Spring", Prop::Float(&mut self.wave_spring, FAC));
                    v.prop("wave_smoothness", "Smoothness", Prop::Float(&mut self.wave_smoothness, Range::new(0.0, 10.0, 0.1)));
                }
                _ => {}
            }
            v.prop("brush_collection", "Brush Collection", Prop::Ref(&mut self.brush_collection, RefKind::Collection));
            v.prop("brush_influence_scale", "Scale Influence", Prop::Float(&mut self.brush_influence_scale, FAC));
            v.prop("brush_radius_scale", "Radius", Prop::Float(&mut self.brush_radius_scale, FAC));
            if t == DpSurfaceType::Paint {
                let mut d = self.use_drying;
                v.panel("dp_dry", "Dry", false, Some(("use_drying", &mut d)));
                v.prop("dry_speed", "Time", Prop::Int(&mut self.dry_speed, 1, 10000));
                v.prop("color_dry_threshold", "Color", Prop::Float(&mut self.color_dry_threshold, FAC));
                v.prop("use_dry_log", "Slow", Prop::Bool(&mut self.use_dry_log));
                v.end();
                self.use_drying = d;
            }
            if t != DpSurfaceType::Wave {
                let mut d = self.use_dissolve;
                v.panel("dp_dissolve", "Dissolve", false, Some(("use_dissolve", &mut d)));
                v.prop("dissolve_speed", "Time", Prop::Int(&mut self.dissolve_speed, 1, 10000));
                v.prop("use_dissolve_log", "Slow", Prop::Bool(&mut self.use_dissolve_log));
                v.end();
                self.use_dissolve = d;
            }
            v.end();
            if matches!(t, DpSurfaceType::Paint | DpSurfaceType::Weight) {
                v.panel("dp_output", "Output", false, None);
                if t == DpSurfaceType::Paint {
                    let mut a = self.use_output_a;
                    v.panel("dp_output_a", "Paintmaps", true, Some(("use_output_a", &mut a)));
                    v.prop("output_name_a", "Name", Prop::Ref(&mut self.output_name_a, RefKind::Text));
                    v.end();
                    self.use_output_a = a;
                    let mut b = self.use_output_b;
                    v.panel("dp_output_b", "Wetmaps", true, Some(("use_output_b", &mut b)));
                    v.prop("output_name_b", "Name", Prop::Ref(&mut self.output_name_b, RefKind::Text));
                    v.end();
                    self.use_output_b = b;
                } else {
                    v.prop("output_name_a", "Vertex Group", Prop::Ref(&mut self.output_name_a, RefKind::Text));
                }
                v.end();
            }
            if t == DpSurfaceType::Paint {
                v.panel("dp_initial_color", "Initial Color", false, None);
                v.prop("init_color_type", "Type", Prop::Enum(&mut self.init_color_type));
                if self.init_color_type == DpInitColorType::Color {
                    v.prop("init_color", "Color", Prop::Color(&mut self.init_color));
                }
                v.end();
                v.panel("dp_effects", "Effects", false, None);
                let mut sp = self.use_spread;
                v.panel("dp_spread", "Spread", false, Some(("use_spread", &mut sp)));
                v.prop("spread_speed", "Speed", Prop::Float(&mut self.spread_speed, Range::new(0.001, 10.0, 0.1)));
                v.prop("color_spread_speed", "Color", Prop::Float(&mut self.color_spread_speed, Range::new(0.0, 2.0, 0.1)));
                v.end();
                self.use_spread = sp;
                let mut dr = self.use_drip;
                v.panel("dp_drip", "Drip", false, Some(("use_drip", &mut dr)));
                v.prop("drip_velocity", "Velocity", Prop::Float(&mut self.drip_velocity, Range::new(-200.0, 200.0, 0.1).slider()));
                v.prop("drip_acceleration", "Acceleration", Prop::Float(&mut self.drip_acceleration, Range::new(-200.0, 200.0, 0.1).slider()));
                v.end();
                self.use_drip = dr;
                let mut sh = self.use_shrink;
                v.panel("dp_shrink", "Shrink", false, Some(("use_shrink", &mut sh)));
                v.prop("shrink_speed", "Speed", Prop::Float(&mut self.shrink_speed, Range::new(0.001, 10.0, 0.1)));
                v.end();
                self.use_shrink = sh;
                v.end();
            }
            v.panel("dp_cache", "Cache", false, None);
            v.end();
        }
        if self.ui_type == DpUiType::Brush && self.use_brush {
            v.panel("dp_brush_source", "Source", true, None);
            v.prop("paint_source", "Paint", Prop::Enum(&mut self.paint_source));
            if self.paint_source != DpPaintSource::Volume {
                v.prop("paint_distance", "Distance", Prop::Float(&mut self.paint_distance, Range::new(0.0, 500.0, 0.1).unit("m")));
                v.prop("proximity_falloff", "Falloff", Prop::Enum(&mut self.proximity_falloff));
                if self.paint_source == DpPaintSource::VolumeDistance {
                    v.prop("invert_proximity", "Inner Proximity", Prop::Bool(&mut self.invert_proximity));
                    v.prop("use_negative_volume", "Negate Volume", Prop::Bool(&mut self.use_negative_volume));
                }
            }
            v.end();
            let mut sm = self.use_smudge;
            v.panel("dp_brush_smudge", "Smudge", false, Some(("use_smudge", &mut sm)));
            v.prop("smudge_strength", "Strength", Prop::Float(&mut self.smudge_strength, FAC));
            v.end();
            self.use_smudge = sm;
            v.panel("dp_brush_wave", "Waves", false, None);
            v.prop("wave_type", "Type", Prop::Enum(&mut self.wave_type));
            if self.wave_type != DpWaveType::Reflect {
                v.prop("wave_factor", "Factor", Prop::Float(&mut self.wave_factor, Range::new(-2.0, 2.0, 0.01)));
                v.prop("wave_clamp", "Clamp Waves", Prop::Float(&mut self.wave_clamp, Range::new(0.0, 50.0, 0.01)));
            }
            v.end();
        }
    }
}

/// Scene physics settings that are not a simulation of their own.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneGravity {
    pub use_gravity: bool,
    pub gravity: [f32; 3],
}

impl Default for SceneGravity {
    fn default() -> Self {
        SceneGravity { use_gravity: true, gravity: [0.0, 0.0, -9.81] }
    }
}

impl Props for SceneGravity {
    fn visit(&mut self, v: &mut dyn Visitor) {
        v.prop("use_gravity", "Gravity", Prop::Bool(&mut self.use_gravity));
        v.prop("gravity", "Gravity", Prop::Vec3(&mut self.gravity, Range::new(-200.0, 200.0, 0.1).unit("m/s²")));
    }
}
