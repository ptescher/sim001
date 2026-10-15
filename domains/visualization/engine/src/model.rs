//! Procedural 3D model of the HEEWING T1-S Ranger, with moving control surfaces and props.

use simulation_aircraft::t1s_ranger::geom::*;
use crate::mesh_util::{airfoil_panel, cuboid, loft, PanelSpec};
use crate::Scene;
use bevy::prelude::*;
use bevy::solari::prelude::RaytracingMesh3d;

/// A plane in the view: 0 is yours, and 1 to 3 are the computer pilots' in a game.
#[derive(Component)]
pub struct PlaneRoot(pub usize);

#[derive(Component)]
pub struct ControlSurfaceVis {
    pub plane: usize,
    pub surface: usize,
    pub axis: Vec3,
}

/// A propeller: the plane and the motor.
#[derive(Component)]
pub struct PropVis(pub usize, pub usize);

#[derive(Component)]
pub struct PropDisc(pub usize, pub usize, pub Handle<StandardMaterial>);

/// How many planes the view can show: yours and three computer pilots'.
pub const PLANES: usize = 4;

/// Where a plane that isn't in play waits, out of sight under the ground.
const PARKED: Vec3 = Vec3::new(0.0, -500.0, 0.0);

pub struct Mats {
    foam_top: Handle<StandardMaterial>,
    foam_bottom: Handle<StandardMaterial>,
    accent: Handle<StandardMaterial>,
    fuselage: Handle<StandardMaterial>,
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
        foam_top: mat(materials, Color::srgb(0.92, 0.93, 0.94), 0.75, 0.0),
        foam_bottom: mat(materials, Color::srgb(0.10, 0.11, 0.13), 0.8, 0.0),
        accent: mat(materials, accent, 0.6, 0.0),
        fuselage: mat(materials, Color::srgb(0.62, 0.65, 0.68), 0.7, 0.0),
        black: mat(materials, Color::srgb(0.03, 0.03, 0.035), 0.5, 0.0),
        metal: mat(materials, Color::srgb(0.55, 0.56, 0.6), 0.3, 1.0),
        canopy: mat(materials, Color::srgb(0.02, 0.03, 0.05), 0.08, 0.0),
        prop: mat(materials, Color::srgb(0.08, 0.08, 0.09), 0.45, 0.0),
    };

    let root = commands.spawn((PlaneRoot(plane), Transform::from_translation(PARKED), Visibility::default())).id();
    let mut children: Vec<Entity> = Vec::new();
    let add = |children: &mut Vec<Entity>, commands: &mut Commands, b: (Handle<Mesh>, Handle<StandardMaterial>, Transform)| {
        children.push(commands.spawn(rt(b.0, b.1, b.2)).id());
    };

    // Fuselage pod + tail boom.
    let pod = meshes.add(loft(
        &[
            (NOSE_Z, 0.004, 0.004, 0.0),
            (NOSE_Z + 0.012, 0.024, 0.026, 0.0),
            (NOSE_Z + 0.04, 0.038, 0.042, 0.002),
            (-0.17, 0.047, 0.054, 0.006),
            (-0.08, 0.050, 0.058, 0.008),
            (0.0, 0.048, 0.054, 0.008),
            (0.07, 0.038, 0.042, 0.012),
            (0.13, 0.020, 0.022, 0.02),
            (0.16, 0.012, 0.012, 0.024),
        ],
        0.0,
        28,
        false,
    ));
    add(&mut children, commands, (pod, m.fuselage.clone(), Transform::IDENTITY));
    let boom = meshes.add(loft(
        &[(0.15, 0.009, 0.009, 0.024), (TAIL_END_Z, 0.007, 0.007, 0.028)],
        0.0,
        12,
        true,
    ));
    add(&mut children, commands, (boom, m.black.clone(), Transform::IDENTITY));
    // Canopy / hatch.
    let canopy = meshes.add(loft(
        &[
            (-0.215, 0.006, 0.004, 0.040),
            (-0.19, 0.028, 0.02, 0.046),
            (-0.10, 0.034, 0.026, 0.050),
            (-0.03, 0.026, 0.016, 0.050),
            (-0.005, 0.006, 0.004, 0.046),
        ],
        0.0,
        20,
        false,
    ));
    add(&mut children, commands, (canopy, m.canopy.clone(), Transform::IDENTITY));
    // FPV camera + lens at the nose.
    let cam_body = meshes.add(cuboid(Vec3::new(0.022, 0.022, 0.02)));
    add(&mut children, commands, (cam_body, m.black.clone(), Transform::from_xyz(0.0, 0.012, NOSE_Z + 0.022)));
    let lens = meshes.add(loft(&[(0.0, 0.007, 0.007, 0.0), (0.012, 0.007, 0.007, 0.0)], 0.0, 12, true));
    add(
        &mut children,
        commands,
        (lens, m.metal.clone(), Transform::from_xyz(0.0, 0.016, NOSE_Z + 0.004).with_rotation(Quat::from_rotation_x(20f32.to_radians()))),
    );
    // Orange nose stripe for orientation.
    let stripe = meshes.add(loft(
        &[(-0.205, 0.0475, 0.0545, 0.006), (-0.185, 0.048, 0.0555, 0.006)],
        0.0,
        28,
        false,
    ));
    add(&mut children, commands, (stripe, m.accent.clone(), Transform::IDENTITY));

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
                thickness: 0.10,
                camber: 0.03,
                origin: Vec3::ZERO,
            });
            let top_mat = if s0 > 0.0 { m.foam_top.clone() } else { m.foam_top.clone() };
            add(&mut children, commands, (meshes.add(top), top_mat, Transform::IDENTITY));
            add(&mut children, commands, (meshes.add(bot), m.foam_bottom.clone(), Transform::IDENTITY));
        }
        // Wingtip accent (top skin only, outermost 6 cm).
        let (tip_top, _) = airfoil_panel(&PanelSpec {
            root_le: le(WING_HALF_SPAN - 0.06) + up * 0.0015,
            tip_le: le(WING_HALF_SPAN) + up * 0.0015,
            root_chord: wing_chord(WING_HALF_SPAN - 0.06),
            tip_chord: wing_chord(WING_HALF_SPAN),
            up,
            x0: 0.0,
            x1: 1.0 - AILERON_CHORD_FRAC - 0.01,
            thickness: 0.10,
            camber: 0.03,
            origin: Vec3::ZERO,
        });
        add(&mut children, commands, (meshes.add(tip_top), m.accent.clone(), Transform::IDENTITY));

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
            thickness: 0.10,
            camber: 0.03,
            origin: hinge_root,
        });
        let mut axis = (hinge_tip - hinge_root).normalize();
        if axis.x < 0.0 {
            axis = -axis;
        }
        let pivot = commands
            .spawn((
                ControlSurfaceVis { plane, surface: if side < 0.0 { 1 } else { 3 }, axis },
                Transform::from_translation(hinge_root),
                Visibility::default(),
            ))
            .id();
        let t = commands.spawn(rt(meshes.add(top), m.accent.clone(), Transform::IDENTITY)).id();
        let b = commands.spawn(rt(meshes.add(bot), m.foam_bottom.clone(), Transform::IDENTITY)).id();
        commands.entity(pivot).add_children(&[t, b]);
        children.push(pivot);

        // Motor nacelle, motor bell, spinner and prop.
        let mx = side * MOTOR_X;
        let my = MOTOR_Y + MOTOR_X * WING_DIHEDRAL.sin();
        let nacelle = meshes.add(loft(
            &[
                (MOTOR_Z + 0.022, 0.011, 0.011, my),
                (MOTOR_Z + 0.04, 0.016, 0.017, my),
                (wing_le_z(MOTOR_X) + 0.03, 0.016, 0.017, my - 0.002),
                (wing_le_z(MOTOR_X) + 0.09, 0.006, 0.006, my - 0.004),
            ],
            mx,
            16,
            false,
        ));
        add(&mut children, commands, (nacelle, m.foam_bottom.clone(), Transform::IDENTITY));
        let bell = meshes.add(loft(&[(MOTOR_Z + 0.004, 0.0095, 0.0095, my), (MOTOR_Z + 0.022, 0.0095, 0.0095, my)], mx, 16, true));
        add(&mut children, commands, (bell, m.metal.clone(), Transform::IDENTITY));
        let motor_index = if side < 0.0 { 0 } else { 1 };
        let prop_pivot = commands
            .spawn((PropVis(plane, motor_index), Transform::from_xyz(mx, my, MOTOR_Z), Visibility::default()))
            .id();
        let blade = meshes.add(cuboid(Vec3::new(2.0 * PROP_RADIUS, 0.010, 0.0025)));
        let spinner = meshes.add(loft(&[(-0.014, 0.001, 0.001, 0.0), (-0.008, 0.007, 0.007, 0.0), (0.0, 0.0085, 0.0085, 0.0)], 0.0, 14, false));
        let pitch = if side < 0.0 { 0.25 } else { -0.25 };
        let bl = commands
            .spawn(rt(blade, m.prop.clone(), Transform::from_xyz(0.0, 0.0, -0.002).with_rotation(Quat::from_rotation_x(pitch))))
            .id();
        let sp = commands.spawn(rt(spinner, m.black.clone(), Transform::IDENTITY)).id();
        commands.entity(prop_pivot).add_children(&[bl, sp]);
        children.push(prop_pivot);
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
        let disc_mesh = meshes.add(Circle::new(PROP_RADIUS).mesh().build());
        let disc = commands
            .spawn((
                PropDisc(plane, motor_index, disc_mat.clone()),
                Mesh3d(disc_mesh),
                MeshMaterial3d(disc_mat),
                Transform::from_xyz(mx, my, MOTOR_Z - 0.003),
            ))
            .id();
        children.push(disc);
    }

    // V-tail panels with ruddervators.
    for side in [-1.0f32, 1.0] {
        let up = Vec3::new(-side * TAIL_DIHEDRAL.sin(), TAIL_DIHEDRAL.cos(), 0.0);
        let out = Vec3::new(side * TAIL_DIHEDRAL.cos(), TAIL_DIHEDRAL.sin(), 0.0);
        let le = |s: f32| Vec3::new(side * TAIL_ROOT_X, TAIL_Y, tail_le_z(s)) + out * s;
        let hx = 1.0 - RUDDERVATOR_CHORD_FRAC;
        let (top, bot) = airfoil_panel(&PanelSpec {
            root_le: le(0.0),
            tip_le: le(TAIL_SPAN),
            root_chord: TAIL_ROOT_CHORD,
            tip_chord: TAIL_TIP_CHORD,
            up,
            x0: 0.0,
            x1: hx,
            thickness: 0.08,
            camber: 0.0,
            origin: Vec3::ZERO,
        });
        add(&mut children, commands, (meshes.add(top), m.foam_top.clone(), Transform::IDENTITY));
        add(&mut children, commands, (meshes.add(bot), m.foam_bottom.clone(), Transform::IDENTITY));
        let hinge_root = le(0.004) + Vec3::Z * hx * tail_chord(0.004);
        let hinge_tip = le(TAIL_SPAN) + Vec3::Z * hx * TAIL_TIP_CHORD;
        let (top, bot) = airfoil_panel(&PanelSpec {
            root_le: le(0.006),
            tip_le: le(TAIL_SPAN - 0.002),
            root_chord: tail_chord(0.006),
            tip_chord: TAIL_TIP_CHORD,
            up,
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
            .spawn((
                ControlSurfaceVis { plane, surface: if side < 0.0 { 4 } else { 5 }, axis },
                Transform::from_translation(hinge_root),
                Visibility::default(),
            ))
            .id();
        let t = commands.spawn(rt(meshes.add(top), m.accent.clone(), Transform::IDENTITY)).id();
        let b = commands.spawn(rt(meshes.add(bot), m.foam_bottom.clone(), Transform::IDENTITY)).id();
        commands.entity(pivot).add_children(&[t, b]);
        children.push(pivot);
    }

    commands.entity(root).add_children(&children);
}

pub fn sync_plane(
    scene: Res<Scene>,
    mut roots: Query<(&PlaneRoot, &mut Transform)>,
    mut surfaces: Query<(&ControlSurfaceVis, &mut Transform), Without<PlaneRoot>>,
    mut props: Query<(&PropVis, &mut Transform), (Without<PlaneRoot>, Without<ControlSurfaceVis>)>,
    discs: Query<&PropDisc>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    time: Res<Time>,
    mut angles: Local<[[f32; 2]; PLANES]>,
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
        if let Some(s) = state(cs.plane) {
            t.rotation = Quat::from_axis_angle(cs.axis, s.deflection.get(cs.surface).copied().unwrap_or(0.0));
        }
    }
    for (p, mut t) in &mut props {
        let Some(s) = state(p.0) else { continue };
        // Real rpm would alias at display rate; show a slowed-down spin and let the
        // translucent disc carry the "fast" look.
        let dir = if p.1 == 0 { 1.0 } else { -1.0 };
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
