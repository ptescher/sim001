//! The games on the view (see domains/recreation): the pylons and start line of a race, the cones
//! round the arena for tag, a label over each computer pilot's plane, and the game panel. The
//! computer pilots' planes themselves are drawn by `model`.

use crate::camera::MainCamera;
use crate::mesh_util::{cuboid, finish};
use crate::model::rt;
use crate::theme::{P, alpha, color, geometry};
use crate::{SharedSim, SimSet, terrain};
use bevy::prelude::*;
use recreation_games::racing::{PYLON_HEIGHT, Status};
use recreation_games::{Clock, Gate, Kind, Session};

/// Where the course furniture waits when no game needs it, out of sight under the ground.
const PARKED: Vec3 = Vec3::new(0.0, -500.0, 0.0);
/// How many cones mark the edge of the tag arena.
const CONES: usize = groundstation_shared::feed::ARENA_CONES;
/// How long after GO the panel says GO, and how long the result stays in the middle (s).
const GO_SHOWN: f32 = 2.0;
const RESULT_SHOWN: f32 = 6.0;

#[derive(Component)]
struct Pylon(usize);
#[derive(Component)]
struct StartLine;
#[derive(Component)]
struct ArenaCone(usize);
/// The label over computer pilot `.0` (1 to 3).
#[derive(Component)]
struct Label(usize);
#[derive(Component)]
struct PanelText;
#[derive(Component)]
struct Panel;

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, spawn).add_systems(Update, update.after(crate::camera::update_camera).in_set(SimSet::Visual));
}

fn spawn(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let paint = |m: &mut Assets<StandardMaterial>, c: Color| m.add(StandardMaterial { base_color: c, perceptual_roughness: 0.5, ..default() });
    let orange = paint(&mut materials, Color::srgb(1.0, 0.35, 0.02));
    let white = paint(&mut materials, Color::srgb(0.95, 0.95, 0.95));

    // The pylons: 5 m tall, orange with a white top.
    let body = meshes.add(finish(Cylinder::new(0.3, PYLON_HEIGHT - 0.8).mesh().resolution(20).build()));
    let top = meshes.add(finish(Cylinder::new(0.3, 0.8).mesh().resolution(20).build()));
    for i in 0..3 {
        commands
            .spawn((Pylon(i), Transform::from_translation(PARKED), Visibility::default()))
            .with_children(|p| {
                p.spawn(rt(body.clone(), orange.clone(), Transform::from_xyz(0.0, 0.5 * (PYLON_HEIGHT - 0.8), 0.0)));
                p.spawn(rt(top.clone(), white.clone(), Transform::from_xyz(0.0, PYLON_HEIGHT - 0.4, 0.0)));
            });
    }
    // The start line: a white strip on the grass, scaled to its length when a race starts.
    commands.spawn((StartLine, rt(meshes.add(cuboid(Vec3::new(0.4, 0.03, 1.0))), white, Transform::from_translation(PARKED))));
    // Cones round the edge of the arena.
    let cone = meshes.add(finish(Cone { radius: 0.35, height: 0.9 }.mesh().resolution(12).build()));
    for i in 0..CONES {
        commands.spawn((ArenaCone(i), rt(cone.clone(), orange.clone(), Transform::from_translation(PARKED))));
    }

    let font = |s: f32| TextFont { font_size: FontSize::Px(s), ..default() };
    // A label over each computer pilot's plane.
    for i in 1..=3 {
        commands.spawn((
            Label(i),
            Text::default(),
            font(13.0),
            TextColor(color(P.text)),
            BackgroundColor(alpha(P.screen, 0.55)),
            Node { position_type: PositionType::Absolute, padding: UiRect::axes(px(4.0), px(1.0)), ..default() },
            Visibility::Hidden,
        ));
    }
    // The game panel, under the stick gauges on the right.
    commands.spawn((
        Panel,
        Node {
            position_type: PositionType::Absolute,
            top: px(120.0),
            right: px(10.0),
            padding: px(8.0).all(),
            border: px(1.0).all(),
            border_radius: BorderRadius::all(px(geometry::SCREEN_RADIUS)),
            ..default()
        },
        BackgroundColor(alpha(P.screen, 0.62)),
        BorderColor::all(alpha(P.bezel_edge, 0.9)),
        Visibility::Hidden,
        children![(PanelText, Text::default(), font(14.0), TextColor(color(P.text)))],
    ));
}

fn on_ground(p: Vec3) -> Vec3 {
    Vec3::new(p.x, terrain::ground_height(p.x, p.z), p.z)
}

/// m:ss.s
fn clock(t: f32) -> String {
    let t = t.max(0.0);
    format!("{}:{:04.1}", (t / 60.0) as u32, t % 60.0)
}

fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, x) if x != 11 => "st",
        (2, x) if x != 12 => "nd",
        (3, x) if x != 13 => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// What the game panel says.
pub fn panel_text(g: &Session) -> String {
    let mut lines = vec![];
    match (g.kind, g.clock) {
        (Kind::Race, _) => lines.push(format!("RACE  {} laps", g.course.laps)),
        (Kind::Tag, _) => lines.push("TAG".to_string()),
    }
    match g.clock {
        Clock::Waiting => lines.push("TAKE OFF".into()),
        Clock::CountingDown { left } => lines.push(format!("GO IN {}", left.ceil() as u32)),
        Clock::Running { time } if time < GO_SHOWN => lines.push("GO!".into()),
        Clock::Running { time } | Clock::Stopped { time } => lines.push(clock(time)),
    }
    if let Some(race) = g.race_ref() {
        let you = &race.racers[0];
        let next = match you.next {
            Gate::Pylon(i) => format!("PYLON {}", i + 1),
            Gate::Line => "START LINE".into(),
        };
        lines.push(format!("LAP {}/{}  CUTS {}  NEXT {next}", you.laps, g.course.laps, you.cuts));
        for (place, i) in race.standings().into_iter().enumerate() {
            let r = &race.racers[i];
            let state = match r.status {
                Status::Racing => format!("L{}", r.laps),
                Status::Finished { corrected, .. } => clock(corrected),
                Status::Disqualified => "DQ".into(),
                Status::Out => "OUT".into(),
            };
            lines.push(format!("{} {:<8} {state}", place + 1, g.name(i).to_uppercase()));
        }
        match (g.clock, you.status) {
            (Clock::Stopped { .. }, Status::Finished { time, corrected }) => {
                let cuts = if you.cuts > 0 { format!(" (cut: {})", clock(corrected)) } else { String::new() };
                lines.push(format!("FINISHED {} in {}{cuts}", ordinal(race.place(0)), clock(time)));
            }
            (_, Status::Disqualified) => lines.push("DISQUALIFIED: two cuts".into()),
            (_, Status::Out) => lines.push("OUT OF THE RACE".into()),
            _ => {}
        }
    }
    if let Some(tag) = g.tag_ref() {
        lines.push(if tag.it == 0 { "YOU ARE IT".into() } else { format!("IT  {}", g.name(tag.it).to_uppercase()) });
        lines.push(format!("{} left", clock(tag.rules.length - tag.time)));
        for (place, i) in tag.standings().into_iter().enumerate() {
            let mark = if i == tag.it { " <" } else { "" };
            lines.push(format!("{} {:<8} {}{mark}", place + 1, g.name(i).to_uppercase(), clock(tag.players[i].it_time)));
        }
        if tag.over() {
            lines.push(format!("TIME! You were it for {}: {}", clock(tag.players[0].it_time), ordinal(tag.standings().iter().position(|&i| i == 0).unwrap() + 1)));
        }
    }
    lines.join("\n")
}

/// What the middle of the HUD says about the game, if anything: the countdown, GO, and the
/// result when it comes.
pub fn centre_text(g: &Session, plane_in_hand: bool) -> Option<String> {
    let what = match g.kind {
        Kind::Race => "RACE",
        Kind::Tag => "TAG",
    };
    match g.clock {
        Clock::Waiting => Some(format!("{what} STARTS WHEN YOU TAKE OFF")),
        Clock::CountingDown { left } => Some(format!("{what} STARTS IN {}", left.ceil() as u32)),
        Clock::Running { time } if time < GO_SHOWN && plane_in_hand => Some("GO! Throw your plane".into()),
        Clock::Stopped { time } if g.since_go() - time < RESULT_SHOWN => {
            let last = panel_text(g);
            last.lines().last().map(str::to_string)
        }
        _ => None,
    }
}

#[allow(clippy::type_complexity)]
fn update(
    sim: Res<SharedSim>,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut furniture: ParamSet<(
        Query<(&Pylon, &mut Transform)>,
        Query<&mut Transform, With<StartLine>>,
        Query<(&ArenaCone, &mut Transform)>,
    )>,
    mut labels: Query<(&Label, &mut Text, &mut TextColor, &mut Node, &mut Visibility), Without<Panel>>,
    mut panel: Single<&mut Visibility, With<Panel>>,
    mut panel_text_q: Single<&mut Text, (With<PanelText>, Without<Label>)>,
) {
    let c = sim.0.lock().unwrap();
    let game = c.game.as_ref();
    let course = game.filter(|g| g.kind == Kind::Race).map(|g| &g.course);
    let arena = game.filter(|g| g.kind == Kind::Tag).map(|g| g.arena);

    for (p, mut t) in &mut furniture.p0() {
        t.translation = course.map_or(PARKED, |c| on_ground(c.pylons[p.0]));
    }
    if let Ok(mut t) = furniture.p1().single_mut() {
        match course {
            Some(c) => {
                let (a, b) = (on_ground(c.line[0]), on_ground(c.line[1]));
                let along = b - a;
                t.translation = (a + b) * 0.5 + Vec3::Y * 0.02;
                t.rotation = Quat::from_rotation_y(along.x.atan2(along.z));
                t.scale = Vec3::new(1.0, 1.0, along.length());
            }
            None => t.translation = PARKED,
        }
    }
    for (cone, mut t) in &mut furniture.p2() {
        t.translation = arena.map_or(PARKED, |a| {
            let angle = cone.0 as f32 / CONES as f32 * std::f32::consts::TAU;
            on_ground(a.centre + Vec3::new(angle.cos(), 0.0, angle.sin()) * a.radius) + Vec3::Y * 0.45
        });
    }

    // Labels over the computer pilots' planes, where the camera sees them.
    let view = camera.single().ok();
    for (label, mut text, mut tint, mut node, mut vis) in &mut labels {
        let Some(g) = game else {
            *vis = Visibility::Hidden;
            continue;
        };
        let o = &g.opponents[label.0 - 1];
        let ndc = view.and_then(|(cam, at)| cam.world_to_ndc(at, o.state.pos + Vec3::Y * 0.8));
        let Some(ndc) = ndc.filter(|n| n.z > 0.0 && n.z < 1.0 && n.x.abs() < 1.0 && n.y.abs() < 1.0) else {
            *vis = Visibility::Hidden;
            continue;
        };
        *vis = Visibility::Visible;
        node.left = percent((ndc.x + 1.0) * 50.0);
        node.top = percent((1.0 - ndc.y) * 50.0);
        let it = g.tag_ref().is_some_and(|t| t.it == label.0);
        let extra = match g.race_ref() {
            Some(r) => format!(" {}", ordinal(r.place(label.0))),
            None if it => " IT".into(),
            None => String::new(),
        };
        *text = Text::new(format!("{}{extra}", o.name));
        let [r, gr, b] = o.colour;
        tint.0 = if it { color(P.warning) } else { Color::linear_rgb(r, gr, b).lighter(0.25) };
    }

    match game {
        Some(g) => {
            **panel = Visibility::Visible;
            **panel_text_q = Text::new(panel_text(g));
        }
        None => **panel = Visibility::Hidden,
    }
}
