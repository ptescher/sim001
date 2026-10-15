//! Procedural 3D model of the HEEWING T1-S Ranger, with moving control surfaces and props.

use simulation_aircraft::t1s_ranger::geom::*;
use crate::mesh_util::{airfoil_panel, cuboid, loft, GeoBuilder, PanelSpec};
use crate::Scene;
use bevy::prelude::*;
use bevy::solari::prelude::RaytracingMesh3d;

/// A plane in the view: 0 is yours, and 1 to 3 are the computer pilots' in a game.
#[derive(Component)]
pub struct PlaneRoot(pub usize);

/// What moves a control surface. The sim's V-tail has two ruddervators; the model's tail is a
/// stabiliser and a fin, so the elevator follows what the two have in common and the rudder
/// what they differ by.
#[derive(Clone, Copy)]
pub enum Drive {
    Surface(usize),
    Elevator,
    Rudder,
}

#[derive(Component)]
pub struct ControlSurfaceVis {
    pub plane: usize,
    pub drive: Drive,
    pub axis: Vec3,
}

/// A wing motor's tilt arm: the plane and the motor.
#[derive(Component)]
pub struct TiltVis(pub usize, pub usize);

/// The VTOL's rear rotor and its mast, which a plane without one doesn't show.
#[derive(Component)]
pub struct RearRotor(pub usize);

/// A propeller: the plane and the motor.
#[derive(Component)]
pub struct PropVis(pub usize, pub usize);

#[derive(Component)]
pub struct PropDisc(pub usize, pub usize, pub Handle<StandardMaterial>);

/// How many planes the view can show: yours and three computer pilots'.
pub const PLANES: usize = 4;

/// The prop drawn on every motor (the Ranger's is 3.8 inch, the VTOL's 5).
const ROTOR_RADIUS: f32 = 0.056;
/// Where the wing motors' tilt arms hinge, just behind the wing's leading edge.
const TILT_PIVOT_Z: f32 = -0.02;
/// The VTOL's rear rotor, where its mixer puts it, on a mast over the boom.
const REAR_ROTOR_Z: f32 = 0.24;
const REAR_ROTOR_Y: f32 = 0.066;

// The tail: a stabiliser across the boom's end with a swept fin on it.
const STAB_HALF_SPAN: f32 = 0.17;
const STAB_ROOT_X: f32 = 0.010;
const STAB_ROOT_CHORD: f32 = 0.095;
const STAB_TIP_CHORD: f32 = 0.062;
const STAB_LE_Z: f32 = 0.335;
const STAB_SWEEP: f32 = 0.03;
const STAB_Y: f32 = 0.028;
const ELEVATOR_CHORD_FRAC: f32 = 0.35;
const FIN_HEIGHT: f32 = 0.17;
const FIN_BASE_Y: f32 = 0.028;
const FIN_LE_Z: f32 = 0.31;
const FIN_SWEEP: f32 = 0.07;
const FIN_ROOT_CHORD: f32 = 0.12;
const FIN_TIP_CHORD: f32 = 0.07;
const RUDDER_CHORD_FRAC: f32 = 0.35;

/// Where a plane that isn't in play waits, out of sight under the ground.
const PARKED: Vec3 = Vec3::new(0.0, -500.0, 0.0);

pub struct Mats {
    /// The painted charcoal shell of the wings and tail, top and bottom.
    foam_top: Handle<StandardMaterial>,
    foam_bottom: Handle<StandardMaterial>,
    /// The control surfaces: a shade lighter, so their outlines read at a distance.
    surface: Handle<StandardMaterial>,
    accent: Handle<StandardMaterial>,
    fuselage: Handle<StandardMaterial>,
    carbon: Handle<StandardMaterial>,
    black: Handle<StandardMaterial>,
    metal: Handle<StandardMaterial>,
    canopy: Handle<StandardMaterial>,
    prop: Handle<StandardMaterial>,
}

/// Spawns a mesh that is both rasterized (G-buffer) and present in the ray tracing scene.
pub fn rt(mesh: Handle<Mesh>, mat: Handle<StandardMaterial>, t: Transform) -> impl Bundle {
    (Mesh3d(mesh.clone()), RaytracingMesh3d(mesh), MeshMaterial3d(mat), t)
}

/// Spawns plane `plane`, with its markings (wing tips, nose stripe, ailerons, ruddervators) in
/// `accent`.
pub fn spawn_plane(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, plane: usize, accent: Color) {
    let mat = |m: &mut Assets<StandardMaterial>, c: Color, rough: f32, metal: f32| {
        m.add(StandardMaterial {
            base_color: c,
            perceptual_roughness: rough,
            metallic: metal,
            double_sided: true,
            cull_mode: None,
            ..default()
        })
    };
    let m = Mats {
        // Matte charcoal paint, close to the real plane's: dark, soft, a little sheen.
        foam_top: mat(materials, Color::srgb(0.32, 0.33, 0.36), 0.5, 0.0),
        foam_bottom: mat(materials, Color::srgb(0.27, 0.28, 0.30), 0.55, 0.0),
        surface: mat(materials, Color::srgb(0.40, 0.41, 0.44), 0.48, 0.0),
        accent: mat(materials, accent, 0.45, 0.0),
        fuselage: mat(materials, Color::srgb(0.30, 0.31, 0.34), 0.46, 0.0),
        carbon: mat(materials, Color::srgb(0.04, 0.042, 0.048), 0.3, 0.2),
        black: mat(materials, Color::srgb(0.02, 0.02, 0.024), 0.5, 0.0),
        metal: mat(materials, Color::srgb(0.30, 0.30, 0.33), 0.28, 1.0),
        canopy: mat(materials, Color::srgb(0.01, 0.015, 0.03), 0.06, 0.0),
        prop: mat(materials, Color::srgb(0.08, 0.08, 0.09), 0.45, 0.0),
    };

    let root = commands.spawn((PlaneRoot(plane), Transform::from_translation(PARKED), Visibility::default())).id();
    let mut children: Vec<Entity> = Vec::new();
    let add = |children: &mut Vec<Entity>, commands: &mut Commands, b: (Handle<Mesh>, Handle<StandardMaterial>, Transform)| {
        children.push(commands.spawn(rt(b.0, b.1, b.2)).id());
    };

    // Fuselage: a broad, flat-topped pod that the wing blends into, with a chin camera.
    let pod = meshes.add(loft(
        &[
            (NOSE_Z, 0.006, 0.006, 0.002),
            (NOSE_Z + 0.008, 0.030, 0.024, 0.004),
            (NOSE_Z + 0.028, 0.050, 0.037, 0.008),
            (-0.20, 0.064, 0.045, 0.011),
            (-0.12, 0.072, 0.050, 0.014),
            (-0.03, 0.072, 0.050, 0.014),
            (0.05, 0.062, 0.044, 0.016),
            (0.11, 0.040, 0.032, 0.020),
            (0.15, 0.017, 0.017, 0.024),
        ],
        0.0,
        36,
        false,
    ));
    add(&mut children, commands, (pod, m.fuselage.clone(), Transform::IDENTITY));
    // Carbon tail boom, with the accent collar where it meets the pod.
    let boom = meshes.add(loft(&[(0.14, 0.0095, 0.0095, 0.024), (TAIL_END_Z, 0.0075, 0.0075, 0.028)], 0.0, 16, true));
    add(&mut children, commands, (boom, m.carbon.clone(), Transform::IDENTITY));
    let collar = meshes.add(loft(&[(0.148, 0.0125, 0.0125, 0.0245), (0.172, 0.0125, 0.0125, 0.0255)], 0.0, 20, true));
    add(&mut children, commands, (collar, m.accent.clone(), Transform::IDENTITY));
    // Cooling vent behind the nose, and the carry handle on the back.
    let vent = meshes.add(cuboid(Vec3::new(0.046, 0.003, 0.026)));
    add(&mut children, commands, (vent, m.black.clone(), Transform::from_xyz(0.0, 0.0555, -0.225).with_rotation(Quat::from_rotation_x(-0.18))));
    let post = meshes.add(cuboid(Vec3::new(0.005, 0.016, 0.006)));
    let bar = meshes.add(cuboid(Vec3::new(0.05, 0.004, 0.007)));
    for x in [-0.023, 0.023] {
        add(&mut children, commands, (post.clone(), m.carbon.clone(), Transform::from_xyz(x, 0.064, -0.075)));
    }
    add(&mut children, commands, (bar, m.carbon.clone(), Transform::from_xyz(0.0, 0.074, -0.075)));
    // FPV camera: a dark chin window and its lens.
    let chin = meshes.add(cuboid(Vec3::new(0.036, 0.020, 0.012)));
    add(&mut children, commands, (chin, m.canopy.clone(), Transform::from_xyz(0.0, -0.004, NOSE_Z + 0.022).with_rotation(Quat::from_rotation_x(-0.35))));
    let lens = meshes.add(loft(&[(0.0, 0.007, 0.007, 0.0), (0.008, 0.0075, 0.0075, 0.0)], 0.0, 16, true));
    add(&mut children, commands, (lens, m.metal.clone(), Transform::from_xyz(0.0, -0.002, NOSE_Z + 0.01).with_rotation(Quat::from_rotation_x(-0.35))));
    // Wings: inner panel (full chord), outer panel (ahead of aileron) and aileron.
    for side in [-1.0f32, 1.0] {
        let up = Vec3::new(-side * WING_DIHEDRAL.sin(), WING_DIHEDRAL.cos(), 0.0);
        let out = Vec3::new(side * WING_DIHEDRAL.cos(), WING_DIHEDRAL.sin(), 0.0);
        let le = |s: f32| Vec3::new(0.0, WING_Y, wing_le_z(s)) + out * s;
        let pieces = [
            (0.0, AILERON_START, 0.0, 1.0, false),
            (AILERON_START, WING_HALF_SPAN, 0.0, 1.0 - AILERON_CHORD_FRAC, false),
        ];
        for (s0, s1, x0, x1, _) in pieces {
            let (top, bot) = airfoil_panel(&PanelSpec {
                root_le: le(s0),
                tip_le: le(s1),
                root_chord: wing_chord(s0),
                tip_chord: wing_chord(s1),
                up,
                x0,
                x1,
                thickness: 0.11,
                camber: 0.03,
                origin: Vec3::ZERO,
            });
            add(&mut children, commands, (meshes.add(top), m.foam_top.clone(), Transform::IDENTITY));
            add(&mut children, commands, (meshes.add(bot), m.foam_bottom.clone(), Transform::IDENTITY));
        }
        // Wingtip accent, top and bottom (a plane seen from the field is mostly underside).
        let (tip_top, tip_bot) = airfoil_panel(&PanelSpec {
            root_le: le(WING_HALF_SPAN - 0.08) + up * 0.003,
            tip_le: le(WING_HALF_SPAN) + up * 0.003,
            root_chord: wing_chord(WING_HALF_SPAN - 0.08),
            tip_chord: wing_chord(WING_HALF_SPAN),
            up,
            x0: 0.0,
            x1: 1.0 - AILERON_CHORD_FRAC - 0.01,
            thickness: 0.11,
            camber: 0.03,
            origin: Vec3::ZERO,
        });
        add(&mut children, commands, (meshes.add(tip_top), m.accent.clone(), Transform::IDENTITY));
        add(&mut children, commands, (meshes.add(tip_bot), m.accent.clone(), Transform::from_translation(-up * 0.0060)));

        // Aileron, hinged at its leading edge.
        let hx = 1.0 - AILERON_CHORD_FRAC;
        let hinge_root = le(AILERON_START) + Vec3::Z * hx * wing_chord(AILERON_START) + up * 0.004;
        let hinge_tip = le(WING_HALF_SPAN) + Vec3::Z * hx * wing_chord(WING_HALF_SPAN) + up * 0.003;
        let (top, bot) = airfoil_panel(&PanelSpec {
            root_le: le(AILERON_START + 0.003),
            tip_le: le(WING_HALF_SPAN - 0.003),
            root_chord: wing_chord(AILERON_START),
            tip_chord: wing_chord(WING_HALF_SPAN),
            up,
            x0: hx + 0.005,
            x1: 1.0,
            thickness: 0.11,
            camber: 0.03,
            origin: hinge_root,
        });
        let mut axis = (hinge_tip - hinge_root).normalize();
        if axis.x < 0.0 {
            axis = -axis;
        }
        let pivot = commands
            .spawn((
                ControlSurfaceVis { plane, drive: Drive::Surface(if side < 0.0 { 1 } else { 3 }), axis },
                Transform::from_translation(hinge_root),
                Visibility::default(),
            ))
            .id();
        let t = commands.spawn(rt(meshes.add(top), m.surface.clone(), Transform::IDENTITY)).id();
        let b = commands.spawn(rt(meshes.add(bot), m.surface.clone(), Transform::IDENTITY)).id();
        commands.entity(pivot).add_children(&[t, b]);
        children.push(pivot);

        // The wing motor on its tilt arm: the servo stays in the wing, and the arm, motor and
        // prop turn on it, forward for flight and up for a hover.
        let mx = side * MOTOR_X;
        let my = MOTOR_Y + MOTOR_X * WING_DIHEDRAL.sin();
        let hinge = Vec3::new(mx, my, TILT_PIVOT_Z);
        let reach = MOTOR_Z - TILT_PIVOT_Z;
        let servo = meshes.add(cuboid(Vec3::new(0.012, 0.018, 0.028)));
        add(&mut children, commands, (servo, m.black.clone(), Transform::from_xyz(mx + side * 0.011, my - 0.006, TILT_PIVOT_Z + 0.004)));
        let tilt_pivot = commands
            .spawn((TiltVis(plane, if side < 0.0 { 0 } else { 1 }), Transform::from_translation(hinge), Visibility::default()))
            .id();
        let mut on_arm: Vec<Entity> = Vec::new();
        let arm = meshes.add(cuboid(Vec3::new(0.010, 0.008, -reach)));
        on_arm.push(commands.spawn(rt(arm, m.carbon.clone(), Transform::from_xyz(0.0, 0.0, 0.5 * reach))).id());
        let nacelle = meshes.add(loft(
            &[(reach + 0.022, 0.011, 0.011, 0.0), (reach + 0.04, 0.016, 0.017, 0.0), (reach + 0.075, 0.016, 0.017, 0.0), (reach + 0.1, 0.008, 0.008, 0.0)],
            0.0,
            16,
            false,
        ));
        on_arm.push(commands.spawn(rt(nacelle, m.black.clone(), Transform::IDENTITY)).id());
        let bell = meshes.add(loft(&[(reach + 0.004, 0.0115, 0.0115, 0.0), (reach + 0.024, 0.0115, 0.0115, 0.0)], 0.0, 20, true));
        on_arm.push(commands.spawn(rt(bell, m.metal.clone(), Transform::IDENTITY)).id());
        let motor_index = if side < 0.0 { 0 } else { 1 };
        on_arm.extend(spawn_rotor(commands, meshes, materials, &m, plane, motor_index, Vec3::new(0.0, 0.0, reach), side < 0.0));
        commands.entity(tilt_pivot).add_children(&on_arm);
        children.push(tilt_pivot);
    }

    // Tail: the horizontal stabiliser with its elevators, and the fin with its rudder.
    for side in [-1.0f32, 1.0] {
        let le = |s: f32| Vec3::new(side * (STAB_ROOT_X + s), STAB_Y, STAB_LE_Z + STAB_SWEEP * s / STAB_HALF_SPAN);
        let chord = |s: f32| STAB_ROOT_CHORD + (STAB_TIP_CHORD - STAB_ROOT_CHORD) * s / STAB_HALF_SPAN;
        let hx = 1.0 - ELEVATOR_CHORD_FRAC;
        let panel = |s0: f32, s1: f32, x0: f32, x1: f32, lift: f32, origin: Vec3| {
            airfoil_panel(&PanelSpec {
                root_le: le(s0) + Vec3::Y * lift,
                tip_le: le(s1) + Vec3::Y * lift,
                root_chord: chord(s0),
                tip_chord: chord(s1),
                up: Vec3::Y,
                x0,
                x1,
                thickness: 0.08,
                camber: 0.0,
                origin,
            })
        };
        let (top, bot) = panel(0.0, STAB_HALF_SPAN, 0.0, hx, 0.0, Vec3::ZERO);
        add(&mut children, commands, (meshes.add(top), m.foam_top.clone(), Transform::IDENTITY));
        add(&mut children, commands, (meshes.add(bot), m.foam_bottom.clone(), Transform::IDENTITY));
        let (cap_top, cap_bot) = panel(STAB_HALF_SPAN - 0.04, STAB_HALF_SPAN, 0.0, hx - 0.01, 0.0025, Vec3::ZERO);
        add(&mut children, commands, (meshes.add(cap_top), m.accent.clone(), Transform::IDENTITY));
        add(&mut children, commands, (meshes.add(cap_bot), m.accent.clone(), Transform::from_xyz(0.0, -0.005, 0.0)));
        let hinge_root = le(0.004) + Vec3::Z * hx * chord(0.004);
        let hinge_tip = le(STAB_HALF_SPAN) + Vec3::Z * hx * chord(STAB_HALF_SPAN);
        let (top, bot) = airfoil_panel(&PanelSpec {
            root_le: le(0.006),
            tip_le: le(STAB_HALF_SPAN - 0.002),
            root_chord: chord(0.006),
            tip_chord: chord(STAB_HALF_SPAN),
            up: Vec3::Y,
            x0: hx + 0.006,
            x1: 1.0,
            thickness: 0.08,
            camber: 0.0,
            origin: hinge_root,
        });
        let mut axis = (hinge_tip - hinge_root).normalize();
        if axis.x < 0.0 {
            axis = -axis;
        }
        let pivot = commands
            .spawn((ControlSurfaceVis { plane, drive: Drive::Elevator, axis }, Transform::from_translation(hinge_root), Visibility::default()))
            .id();
        let t = commands.spawn(rt(meshes.add(top), m.surface.clone(), Transform::IDENTITY)).id();
        let b = commands.spawn(rt(meshes.add(bot), m.surface.clone(), Transform::IDENTITY)).id();
        commands.entity(pivot).add_children(&[t, b]);
        children.push(pivot);
    }

    // The fin: swept, on the boom, with the rudder hinged at the back.
    {
        let le = |h: f32| Vec3::new(0.0, FIN_BASE_Y + h, FIN_LE_Z + FIN_SWEEP * h / FIN_HEIGHT);
        let chord = |h: f32| FIN_ROOT_CHORD + (FIN_TIP_CHORD - FIN_ROOT_CHORD) * h / FIN_HEIGHT;
        let hx = 1.0 - RUDDER_CHORD_FRAC;
        let panel = |h0: f32, h1: f32, x0: f32, x1: f32, side: f32, origin: Vec3| {
            airfoil_panel(&PanelSpec {
                root_le: le(h0) + Vec3::X * side,
                tip_le: le(h1) + Vec3::X * side,
                root_chord: chord(h0),
                tip_chord: chord(h1),
                up: Vec3::X,
                x0,
                x1,
                thickness: 0.07,
                camber: 0.0,
                origin,
            })
        };
        let (top, bot) = panel(0.0, FIN_HEIGHT, 0.0, hx, 0.0, Vec3::ZERO);
        add(&mut children, commands, (meshes.add(top), m.foam_top.clone(), Transform::IDENTITY));
        add(&mut children, commands, (meshes.add(bot), m.foam_top.clone(), Transform::IDENTITY));
        let (cap_a, cap_b) = panel(FIN_HEIGHT - 0.04, FIN_HEIGHT, 0.0, hx - 0.01, 0.0025, Vec3::ZERO);
        add(&mut children, commands, (meshes.add(cap_a), m.accent.clone(), Transform::IDENTITY));
        add(&mut children, commands, (meshes.add(cap_b), m.accent.clone(), Transform::from_xyz(-0.005, 0.0, 0.0)));
        let hinge_root = le(0.004) + Vec3::Z * hx * chord(0.004);
        let hinge_tip = le(FIN_HEIGHT) + Vec3::Z * hx * chord(FIN_HEIGHT);
        let (a, b) = panel(0.006, FIN_HEIGHT - 0.002, hx + 0.006, 1.0, 0.0, hinge_root);
        let pivot = commands
            .spawn((
                ControlSurfaceVis { plane, drive: Drive::Rudder, axis: (hinge_tip - hinge_root).normalize() },
                Transform::from_translation(hinge_root),
                Visibility::default(),
            ))
            .id();
        let ta = commands.spawn(rt(meshes.add(a), m.surface.clone(), Transform::IDENTITY)).id();
        let tb = commands.spawn(rt(meshes.add(b), m.surface.clone(), Transform::IDENTITY)).id();
        commands.entity(pivot).add_children(&[ta, tb]);
        children.push(pivot);
    }

    // The VTOL's rear rotor, on a mast over the boom: only shown for a plane that has one.
    {
        let rotor = commands
            .spawn((
                RearRotor(plane),
                Transform::from_xyz(0.0, REAR_ROTOR_Y, REAR_ROTOR_Z).with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                Visibility::default(),
            ))
            .id();
        let mut parts: Vec<Entity> = Vec::new();
        let bell = meshes.add(loft(&[(0.004, 0.0115, 0.0115, 0.0), (0.026, 0.0115, 0.0115, 0.0)], 0.0, 20, true));
        parts.push(commands.spawn(rt(bell, m.metal.clone(), Transform::IDENTITY)).id());
        let mast = meshes.add(cuboid(Vec3::new(0.009, 0.009, REAR_ROTOR_Y - 0.026)));
        parts.push(commands.spawn(rt(mast, m.carbon.clone(), Transform::from_xyz(0.0, 0.0, 0.5 * (REAR_ROTOR_Y - 0.026) + 0.026))).id());
        parts.extend(spawn_rotor(commands, meshes, materials, &m, plane, 2, Vec3::ZERO, true));
        commands.entity(rotor).add_children(&parts);
        children.push(rotor);
    }

    commands.entity(root).add_children(&children);
}

/// A propeller with its spinner and blurred disc, spinning on `at`, to be a child of the motor.
fn spawn_rotor(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    m: &Mats,
    plane: usize,
    motor: usize,
    at: Vec3,
    clockwise: bool,
) -> Vec<Entity> {
    let prop_pivot = commands.spawn((PropVis(plane, motor), Transform::from_translation(at), Visibility::default())).id();
    let blade = meshes.add(prop_blades(ROTOR_RADIUS, if clockwise { 1.0 } else { -1.0 }));
    let spinner = meshes.add(loft(&[(-0.016, 0.001, 0.001, 0.0), (-0.009, 0.0075, 0.0075, 0.0), (0.0, 0.0105, 0.0105, 0.0)], 0.0, 18, false));
    let bl = commands.spawn(rt(blade, m.prop.clone(), Transform::from_xyz(0.0, 0.0, -0.002))).id();
    let sp = commands.spawn(rt(spinner, m.accent.clone(), Transform::IDENTITY)).id();
    commands.entity(prop_pivot).add_children(&[bl, sp]);
    // Blurred prop disc (raster only, translucent).
    let disc_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.1, 0.1, 0.1, 0.0),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.6,
        double_sided: true,
        cull_mode: None,
        unlit: false,
        ..default()
    });
    let disc_mesh = meshes.add(Circle::new(ROTOR_RADIUS).mesh().build());
    let disc = commands
        .spawn((PropDisc(plane, motor, disc_mat.clone()), Mesh3d(disc_mesh), MeshMaterial3d(disc_mat), Transform::from_translation(at + Vec3::new(0.0, 0.0, -0.003))))
        .id();
    vec![prop_pivot, disc]
}

pub fn sync_plane(
    scene: Res<Scene>,
    mut roots: Query<(&PlaneRoot, &mut Transform)>,
    mut surfaces: Query<(&ControlSurfaceVis, &mut Transform), Without<PlaneRoot>>,
    mut tilts: Query<(&TiltVis, &mut Transform), (Without<PlaneRoot>, Without<ControlSurfaceVis>)>,
    mut rears: Query<(&RearRotor, &mut Transform), (Without<PlaneRoot>, Without<ControlSurfaceVis>, Without<TiltVis>)>,
    mut props: Query<(&PropVis, &mut Transform), (Without<PlaneRoot>, Without<ControlSurfaceVis>, Without<TiltVis>, Without<RearRotor>)>,
    discs: Query<&PropDisc>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    time: Res<Time>,
    mut angles: Local<[[f32; 3]; PLANES]>,
) {
    // Plane 0 is yours; the others are other aircraft (the computer pilots', in a game).
    let state = |plane: usize| scene.vehicles.get(plane).and_then(Option::as_ref);
    for (root, mut t) in &mut roots {
        match state(root.0) {
            Some(s) => {
                t.translation = s.pos;
                t.rotation = s.rot;
            }
            None => t.translation = PARKED,
        }
    }
    for (cs, mut t) in &mut surfaces {
        // Surfaces are modelled "trailing edge down = positive"; rotating about an axis
        // pointing roughly +X by a positive angle moves the trailing edge (+Z) down.
        let Some(s) = state(cs.plane) else { continue };
        let d = |i: usize| s.deflection.get(i).copied().unwrap_or(0.0);
        let angle = match cs.drive {
            Drive::Surface(i) => d(i),
            // The V-tail's two panels (4 left, 5 right) both trail down for the elevator, and
            // one down and one up for the rudder (right panel down is yaw right, trailing edge to the
            // left, which is a turn about +Y by a negative angle).
            Drive::Elevator => 0.5 * (d(4) + d(5)),
            Drive::Rudder => -0.5 * (d(5) - d(4)),
        };
        t.rotation = Quat::from_axis_angle(cs.axis, angle);
    }
    for (tv, mut t) in &mut tilts {
        let Some(s) = state(tv.0) else { continue };
        // Tilt is how far the motor has turned from pointing forward, nose-up about +X.
        t.rotation = Quat::from_rotation_x(s.tilt.get(tv.1).copied().unwrap_or(0.0));
    }
    for (r, mut t) in &mut rears {
        // A plane without a third motor has no rear rotor: shrink it to nothing.
        let has = state(r.0).is_some_and(|s| s.spool.len() >= 3);
        t.scale = Vec3::splat(if has { 1.0 } else { 1e-4 });
    }
    for (p, mut t) in &mut props {
        let Some(s) = state(p.0) else { continue };
        // Real rpm would alias at display rate; show a slowed-down spin and let the
        // translucent disc carry the "fast" look.
        let dir = if p.1 == 1 { -1.0 } else { 1.0 };
        let a = &mut angles[p.0][p.1];
        *a = (*a + dir * s.spool.get(p.1).copied().unwrap_or(0.0).sqrt() * 21.0 * time.delta_secs()) % std::f32::consts::TAU;
        t.rotation = Quat::from_rotation_z(*a);
    }
    for d in &discs {
        let Some(s) = state(d.0) else { continue };
        if let Some(mut m) = materials.get_mut(&d.2) {
            let a = (s.spool.get(d.1).copied().unwrap_or(0.0) * 0.55).clamp(0.0, 0.4);
            m.base_color = Color::srgba(0.12, 0.12, 0.13, if s.crashed { 0.0 } else { a });
        }
    }
}

/// Two propeller blades along ±X, tapered and twisted, with their face towards +Z. `hand` is 1 or
/// -1 for a prop that turns one way or the other.
fn prop_blades(radius: f32, hand: f32) -> Mesh {
    let mut b = GeoBuilder::default();
    let stations = 7;
    for side in [1.0f32, -1.0] {
        let mut rows = Vec::new();
        for i in 0..=stations {
            let u = i as f32 / stations as f32;
            let r = 0.007 + (radius - 0.007) * u;
            // Widest a third of the way out, thin at the tip; pitched most at the root.
            let chord = 0.012 * (0.6 + 0.4 * (u / 0.35).min(1.0)) * (1.0 - 0.45 * u);
            let twist = (0.62 - 0.42 * u) * hand;
            let (c, s_) = (twist.cos(), twist.sin());
            let at = |t: f32| Vec3::new(side * r, (t - 0.35) * chord * c, (t - 0.35) * chord * s_ * 0.6);
            rows.push((b.vert(at(0.0), Vec2::new(0.0, u)), b.vert(at(1.0), Vec2::new(1.0, u))));
        }
        for i in 0..stations {
            b.quad(rows[i].0, rows[i].1, rows[i + 1].1, rows[i + 1].0, Vec3::Z * side.signum().abs());
        }
    }
    b.build()
}
