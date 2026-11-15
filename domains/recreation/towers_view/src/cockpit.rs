//! The cockpit: `towers_view --fly blue:2` takes the seat of one drone as its pilot
//! (`Join::Unit`), shows that drone's nose camera full screen with a small HUD, and sends the
//! pilot's sticks to the server.
//!
//! The picture is made from what the drone's seat is told (`Mission`): the drone's own place and
//! attitude, and the enemy drones its camera sees. No other drone is drawn, and the team's map and
//! the truth are not read. The HUD shows the speed, the height, the battery, whether the drone
//! is in contact, and the mission: the commander's order and queue, and the tower's name with its
//! bearing and distance.
//!
//! Keys: W / S throttle up and down, Left / Right roll, Up / Down pitch (Up pushes the stick
//! forward: nose down), A / D yaw, Space centres the sticks with the throttle at half.
//! A gamepad or the RadioMaster TX15 (a USB joystick) is read as well, and wins while it moves.

use bevy::input::gamepad::{Gamepad, GamepadAxis};
use bevy::prelude::*;
use recreation_towers::seat::{Join, Mission, Sticks};
use recreation_towers::{Order, Team};
use recreation_towers_ground::pilot::target_text;
use recreation_towers_ground::seat::{Seat, Status};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use visualization_engine::camera::{CamMode, CamState};
use visualization_engine::{EngineSet, Pose, Scene};

/// The sticks are sent this often (Hz): the server gives the drone back after 2 s without them.
const SEND_HZ: f32 = 30.0;
/// How fast a key moves a stick (per second), and how fast it comes back when it is let go.
const KEY_RATE: f32 = 2.5;
const KEY_RETURN: f32 = 4.0;
/// How fast W and S move the throttle (per second).
const THROTTLE_RATE: f32 = 0.5;
/// The pilot's own plane is in this slot of the scene, and the engine's FPV camera follows it.
const OWN_SLOT: usize = 0;

/// The drone to fly: its team and its number in the team, from 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fly {
    pub team: Team,
    pub drone: usize,
}

/// `blue:2` is the second Blue drone: the team, a colon, and the drone's number from 1, as the
/// game's names say it (Blue2).
pub fn parse(s: &str) -> Result<Fly, String> {
    let bad = || format!("--fly takes a team and a drone, like blue:2 (the second Blue drone), not {s:?}");
    let (team, drone) = s.split_once(':').ok_or_else(bad)?;
    let team = match team.to_lowercase().as_str() {
        "blue" => Team::Blue,
        "red" => Team::Red,
        _ => return Err(bad()),
    };
    let drone: usize = drone.parse().map_err(|_| bad())?;
    if drone == 0 {
        return Err(bad());
    }
    Ok(Fly { team, drone: drone - 1 })
}

#[derive(Resource)]
struct Cockpit {
    seat: Arc<Seat>,
    fly: Fly,
    /// The sticks as the pilot holds them now, sent 30 times a second by a thread of its own.
    sticks: Arc<Mutex<Sticks>>,
    /// Their last value from the keys (they move by degrees).
    keys: Sticks,
}

#[derive(Component)]
struct Readout;
#[derive(Component)]
struct MissionText;
#[derive(Component)]
struct Notice;

pub fn plugin(app: &mut App, fly: Fly, server: &str) {
    let seat = Seat::connect(server, Join::Unit { team: fly.team, drone: fly.drone });
    let sticks = Arc::new(Mutex::new(Sticks::CENTRED));
    {
        let (seat, sticks) = (seat.clone(), sticks.clone());
        std::thread::spawn(move || {
            while !seat.stopped() {
                let s = *sticks.lock().unwrap();
                seat.sticks(s);
                std::thread::sleep(Duration::from_secs_f32(1.0 / SEND_HZ));
            }
        });
    }
    app.insert_resource(Cockpit { seat, fly, sticks, keys: Sticks::CENTRED })
        .add_systems(Startup, (look_through_the_nose, hud))
        .add_systems(Update, (read_sticks, publish.before(EngineSet::Draw), show));
}

/// The engine's FPV camera is a nose camera, on the plane in the first slot.
fn look_through_the_nose(mut cs: ResMut<CamState>) {
    cs.mode = CamMode::Fpv;
}

/// How a drone's body sits in the view's world, from the quaternion the drone reports.
fn attitude(q: [f32; 4]) -> Quat {
    let q = Quat::from_xyzw(q[0], q[1], q[2], q[3]);
    if q.length_squared() > 0.5 { q.normalize() } else { Quat::IDENTITY }
}

/// The scene: the drone itself in the first slot, and the enemy drones its camera sees in the slots
/// of the other team's liveries. Nothing else: a pilot sees what the drone sees.
fn publish(cockpit: Res<Cockpit>, mut scene: ResMut<Scene>) {
    scene.vehicles.fill(None);
    let Some(m) = cockpit.seat.state().mission.clone() else { return };
    let to_world = |at: glam::Vec2, alt: f32| Vec3::new(at.y, alt, -at.x);
    let own = Pose {
        pos: to_world(m.obs.at, m.obs.alt),
        rot: attitude(m.attitude),
        spool: vec![0.6; 3],
        tilt: vec![0.0, 0.0, 0.0],
        ..default()
    };
    scene.vehicles[OWN_SLOT] = Some(own);
    // The Red drones of a Blue pilot are in the slots from 5, the Blue drones of a Red pilot from 1
    // (the liveries of the view: Blue's first, then Red's).
    let first = if cockpit.fly.team == Team::Blue { 5 } else { 1 };
    for (k, e) in m.obs.enemies.iter().enumerate() {
        if let Some(slot) = scene.vehicles.get_mut(first + k) {
            let (spool, tilt) = match e.class {
                recreation_towers::Class::Light => (vec![0.6; 4], vec![]),
                _ => (vec![0.6; 3], vec![0.0, 0.0, 0.0]),
            };
            *slot = Some(Pose { pos: to_world(m.obs.at + e.offset, e.alt), spool, tilt, ..default() });
        }
    }
}

// ---- the sticks ----

/// What the keys hold: each stick's direction, -1, 0 or 1, and the throttle's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Keys {
    pub roll: f32,
    /// Positive: the stick pulled back, nose up.
    pub pitch: f32,
    pub yaw: f32,
    pub throttle: f32,
    pub centre: bool,
}

/// The sticks one step on from `now`, with `keys` held for `dt` seconds: a stick moves toward the
/// direction of its key and comes back to the middle when it is let go, as a spring does; the
/// throttle stays where it is.
pub fn step_keys(now: Sticks, keys: Keys, dt: f32) -> Sticks {
    if keys.centre {
        return Sticks::CENTRED;
    }
    let towards = |x: f32, dir: f32| {
        if dir == 0.0 {
            let back = KEY_RETURN * dt;
            if x.abs() <= back { 0.0 } else { x - back * x.signum() }
        } else {
            (x + dir * KEY_RATE * dt).clamp(-1.0, 1.0)
        }
    };
    Sticks {
        roll: towards(now.roll, keys.roll),
        pitch: towards(now.pitch, keys.pitch),
        yaw: towards(now.yaw, keys.yaw),
        throttle: (now.throttle + keys.throttle * THROTTLE_RATE * dt).clamp(0.0, 1.0),
    }
}

/// The raw axes of a joystick or gamepad, as Bevy reads them (up is positive on a gamepad).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Axes {
    pub left_x: f32,
    pub left_y: f32,
    pub right_x: f32,
    pub right_y: f32,
    /// The fourth channel of a radio's joystick (rudder), which has no name in Bevy.
    pub other: f32,
}

/// The sticks of a radio that is a USB joystick (a RadioMaster TX15 with EdgeTX, channels AETR):
/// roll and pitch on the left stick axes, the throttle absolute on the right stick's X and the rudder
/// on the fourth channel, as the sim's default mapping has them. A radio's elevator axis is positive
/// for the stick pulled back.
pub fn radio_sticks(a: Axes) -> Sticks {
    Sticks { roll: a.left_x, pitch: a.left_y, throttle: 0.5 + 0.5 * a.right_x, yaw: a.other }.clamped()
}

/// The sticks of a gamepad in mode 2: the right stick rolls and pitches (up pushes the stick forward,
/// nose down), the left stick's X is the rudder, and its Y moves the throttle (the throttle is `held`
/// and moves by `dt`).
pub fn gamepad_sticks(a: Axes, held: f32, dt: f32) -> Sticks {
    Sticks {
        roll: a.right_x,
        pitch: -a.right_y,
        throttle: (held + a.left_y * THROTTLE_RATE * 1.4 * dt).clamp(0.0, 1.0),
        yaw: a.left_x,
    }
    .clamped()
}

/// The names that mark a joystick as a radio.
const RADIOS: [&str; 9] = ["tx15", "edgetx", "radiomaster", "opentx", "frsky", "jumper", "tx16", "boxer", "zorro"];

fn is_radio(name: &str) -> bool {
    let n = name.to_lowercase();
    RADIOS.iter().any(|r| n.contains(r))
}

/// Whether a stick is off the middle, so that a pad that is only lying there does not take over.
fn moved(s: Sticks) -> bool {
    s.roll.abs() > 0.1 || s.pitch.abs() > 0.1 || s.yaw.abs() > 0.1
}

fn read_sticks(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    pads: Query<(&Gamepad, Option<&Name>)>,
    mut cockpit: ResMut<Cockpit>,
) {
    let dt = time.delta_secs().min(0.1);
    let held = |k: KeyCode| keyboard.pressed(k);
    let dir = |plus: KeyCode, minus: KeyCode| f32::from(held(plus)) - f32::from(held(minus));
    let keys = Keys {
        roll: dir(KeyCode::ArrowRight, KeyCode::ArrowLeft),
        // The Down arrow pulls the stick back (nose up).
        pitch: dir(KeyCode::ArrowDown, KeyCode::ArrowUp),
        yaw: dir(KeyCode::KeyD, KeyCode::KeyA),
        throttle: dir(KeyCode::KeyW, KeyCode::KeyS),
        centre: keyboard.just_pressed(KeyCode::Space),
    };
    let from_keys = step_keys(cockpit.keys, keys, dt);
    cockpit.keys = from_keys;
    let mut sticks = from_keys;
    for (pad, name) in &pads {
        let get = |axis: GamepadAxis| pad.get_unclamped(axis).unwrap_or(0.0);
        let axes = Axes {
            left_x: get(GamepadAxis::LeftStickX),
            left_y: get(GamepadAxis::LeftStickY),
            right_x: get(GamepadAxis::RightStickX),
            right_y: get(GamepadAxis::RightStickY),
            other: get(GamepadAxis::Other(0x0001_0033)),
        };
        let from_pad = if name.is_some_and(|n| is_radio(n.as_str())) {
            radio_sticks(axes)
        } else {
            gamepad_sticks(axes, cockpit.keys.throttle, dt)
        };
        if moved(from_pad) {
            sticks = from_pad;
            // A key then goes on from where the pad left the throttle.
            cockpit.keys.throttle = from_pad.throttle;
        }
    }
    *cockpit.sticks.lock().unwrap() = sticks;
}

// ---- the HUD ----

fn hud(mut commands: Commands) {
    let font = |s: f32| TextFont { font_size: FontSize::Px(s), ..default() };
    let panel = BackgroundColor(Color::srgba(0.03, 0.05, 0.08, 0.6));
    let text = Color::srgb(0.85, 0.95, 0.85);
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(10.0),
            left: px(10.0),
            padding: px(8.0).all(),
            ..default()
        },
        panel,
        children![(Readout, Text::default(), font(18.0), TextColor(text))],
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(10.0),
            right: px(10.0),
            max_width: px(440.0),
            padding: px(8.0).all(),
            ..default()
        },
        panel,
        children![(MissionText, Text::default(), font(16.0), TextColor(text))],
    ));
    commands.spawn((
        Notice,
        Text::new("W/S throttle   arrows roll and pitch   A/D yaw   space centre sticks"),
        font(14.0),
        TextColor(Color::srgb(0.7, 0.75, 0.8)),
        Node { position_type: PositionType::Absolute, bottom: px(12.0), left: px(12.0), ..default() },
    ));
}

/// The words of an order, as the mission says them.
fn order_text(m: &Mission, order: Order) -> String {
    let probe = Mission { order: Some(order), ..m.clone() };
    target_text(&probe).unwrap_or_else(|| "no order".into())
}

/// The readout: the drone's speed, height and heading, battery, and whether it is in contact.
pub fn readout(who: &str, m: &Mission) -> String {
    let speed = m.vel.length();
    let heading = m.vel.y.atan2(m.vel.x).to_degrees().rem_euclid(360.0);
    let battery = m.obs.battery.map_or("-".into(), |b| format!("{b:.0} s"));
    let contact = if m.in_contact { "IN CONTACT".to_string() } else { format!("OUT OF CONTACT {:.0} s", m.obs.silent) };
    format!(
        "{who}\nSPEED {:>3.0} m/s   HEIGHT {:>3.0} m   HEADING {heading:>3.0}\nBATTERY {battery}\n{contact}\nENEMIES IN SIGHT {}",
        speed,
        m.obs.alt,
        m.obs.enemies.len()
    )
}

/// The mission: the order with the tower's name, bearing and distance, then the queue.
pub fn mission_text(m: &Mission) -> String {
    let mut s = String::from("MISSION\n");
    match m.order {
        Some(order) => s += &format!("{}{}\n", order_text(m, order), if m.low { " (low)" } else { "" }),
        None => s += "no order\n",
    }
    for q in &m.queue {
        let when = match q.when {
            recreation_towers::When::Silent(after) => format!("if out of contact {after:.0} s"),
            recreation_towers::When::Done => "when done".to_string(),
            recreation_towers::When::Battery(left) => format!("when the battery is down to {left:.0} s"),
        };
        s += &format!("THEN {when}: {}\n", order_text(m, q.order));
    }
    if !m.in_contact {
        s += "(the radio does not reach: this is the last order that did)\n";
    }
    if let Some(o) = m.outcome {
        s += &format!("GAME OVER: {o:?}\n");
    }
    s
}

fn show(
    cockpit: Res<Cockpit>,
    mut readout_text: Query<&mut Text, (With<Readout>, Without<MissionText>)>,
    mut mission_box: Query<&mut Text, (With<MissionText>, Without<Readout>)>,
) {
    let (status, mission) = {
        let s = cockpit.seat.state();
        (s.status.clone(), s.mission.clone())
    };
    let who = format!("{}{}", if cockpit.fly.team == Team::Blue { "BLUE" } else { "RED" }, cockpit.fly.drone + 1);
    let (left, right) = match (&status, &mission) {
        (Status::Refused(why), _) => (format!("{who}\nREFUSED: {why}"), String::new()),
        (_, Some(m)) => (readout(&who, m), mission_text(m)),
        (Status::Seated(_), None) => (format!("{who}\nWAITING FOR THE DRONE"), String::new()),
        _ => (format!("{who}\nCONNECTING TO THE GAME SERVER..."), String::new()),
    };
    if let Ok(mut t) = readout_text.single_mut() {
        *t = Text::new(left);
    }
    if let Ok(mut t) = mission_box.single_mut() {
        *t = Text::new(right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use recreation_towers::{EdgeObs, Queued};

    #[test]
    fn fly_takes_a_team_and_a_drone_from_one() {
        assert_eq!(parse("blue:2"), Ok(Fly { team: Team::Blue, drone: 1 }));
        assert_eq!(parse("Red:1"), Ok(Fly { team: Team::Red, drone: 0 }));
        for bad in ["blue", "blue:0", "green:1", "blue:x", ":2"] {
            assert!(parse(bad).unwrap_err().contains("blue:2"), "{bad}");
        }
    }

    #[test]
    fn a_key_moves_a_stick_by_degrees_and_a_stick_let_go_comes_back_and_the_throttle_stays() {
        let mut s = Sticks::CENTRED;
        for _ in 0..10 {
            s = step_keys(s, Keys { roll: 1.0, throttle: 1.0, ..Default::default() }, 0.1);
        }
        assert_eq!(s.roll, 1.0, "held a second: full");
        assert!((s.throttle - 1.0).abs() < 1e-6, "{}", s.throttle);
        let mid = step_keys(Sticks::CENTRED, Keys { roll: 1.0, ..Default::default() }, 0.1);
        assert!(mid.roll > 0.2 && mid.roll < 0.3, "a tenth of a second moves it a little: {}", mid.roll);
        for _ in 0..10 {
            s = step_keys(s, Keys::default(), 0.1);
        }
        assert_eq!(s.roll, 0.0, "let go, it comes back");
        assert!((s.throttle - 1.0).abs() < 1e-6, "the throttle stays");
        assert_eq!(step_keys(s, Keys { centre: true, ..Default::default() }, 0.1), Sticks::CENTRED);
    }

    #[test]
    fn a_radio_is_read_as_roll_pitch_throttle_and_rudder_and_a_gamepad_as_mode_two() {
        let a = Axes { left_x: 0.5, left_y: 0.25, right_x: 1.0, right_y: 0.0, other: -0.5 };
        assert_eq!(radio_sticks(a), Sticks { roll: 0.5, pitch: 0.25, throttle: 1.0, yaw: -0.5 });
        let pad = Axes { left_x: -1.0, left_y: 1.0, right_x: 0.5, right_y: 1.0, other: 0.0 };
        let s = gamepad_sticks(pad, 0.5, 0.1);
        assert_eq!((s.roll, s.pitch, s.yaw), (0.5, -1.0, -1.0), "right stick up pushes forward: nose down");
        assert!(s.throttle > 0.5, "the left stick up opens the throttle");
        assert!(is_radio("RadioMaster TX15 Joystick") && !is_radio("Xbox Wireless Controller"));
        assert!(!moved(Sticks::CENTRED) && moved(Sticks { roll: 0.3, ..Sticks::CENTRED }));
    }

    fn mission(order: Option<Order>) -> Mission {
        Mission {
            drone: 1,
            order,
            low: false,
            queue: vec![Queued::silent(10.0, Order::Home)],
            in_contact: true,
            obs: EdgeObs {
                at: Vec2::ZERO,
                alt: 58.4,
                enemies: vec![],
                towers: vec![],
                battery: Some(190.0),
                silent: 0.0,
            },
            vel: Vec2::new(0.0, 21.0),
            attitude: [0.0, 0.0, 0.0, 1.0],
            towers: vec![("west".into(), Vec2::new(0.0, -300.0))],
            time: 12.0,
            time_left: 588.0,
            outcome: None,
        }
    }

    #[test]
    fn the_hud_says_the_speed_the_height_the_contact_and_the_mission() {
        let m = mission(Some(Order::Take(0)));
        let r = readout("BLUE2", &m);
        assert!(r.contains("SPEED  21 m/s") && r.contains("HEIGHT  58 m") && r.contains("HEADING  90"), "{r}");
        assert!(r.contains("BATTERY 190 s") && r.contains("IN CONTACT"), "{r}");
        let t = mission_text(&m);
        assert!(t.contains("take the west tower, 300 m at 270 degrees"), "{t}");
        assert!(t.contains("THEN if out of contact 10 s: go home"), "{t}");
        let dark = Mission { in_contact: false, obs: EdgeObs { silent: 14.0, ..m.obs.clone() }, ..m };
        assert!(readout("BLUE2", &dark).contains("OUT OF CONTACT 14 s"));
        assert!(mission_text(&dark).contains("last order that did"));
    }
}
