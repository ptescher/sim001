//! A game of tag between the three computer pilots, each flying a T1-S VTOL with an ArduPlane
//! SITL of its own, with no window: `cargo run --release -p cognition_runner -- [--speedup N]`.
//!
//! The planes stand on a flat field. The referee and the pilots are `recreation_games`, as in
//! the sim; this program is the host: it answers each SITL's servo packets with the plane's
//! physics and sends back what the pilot's radio says. The game starts after `--warmup` seconds
//! (70) of the planes' own time, which ArduPilot needs before it will arm. Needs
//! `ardupilot/build_native.sh`.
//!
//! `--agent`: once each plane has taken off and is on the wing, the ground control agent
//! (`cognition_agent`) switches it to the AGENT mode over MAVLink and flies it with plans, as the
//! real station will (see `ground.rs`). Needs the fork with the AGENT mode. With `--console` as
//! well, the first plane is yours after the game: send it plans by hand (see `console.rs`).

mod console;
mod ground;

use cognition_agent::link::Traffic;
use glam::Vec3;
use recreation_games::pilot::Fbwa;
use recreation_games::{Field, Radio, Session, Surroundings};
use simulation_aircraft::ActuatorCommands;
use simulation_physics::{Environment, Flat, FlightState, Phase, Start, step};
use simulation_sitl::{Lockstep, launch, port_for, sensor_json};
use std::net::UdpSocket;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BASE_PORT: u16 = 9002;
const WIND: Vec3 = Vec3::ZERO;

struct World {
    session: Session,
    /// Stands for player 0 (you), who is not playing: on the ground for good.
    bench: FlightState,
    params: simulation_aircraft::AircraftParams,
    packets: [u64; 3],
    /// The game waits until the autopilots have started up (their time, s).
    warmup: f64,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let speedup: f32 = arg("--speedup").and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let wall_limit: u64 = arg("--timeout").and_then(|s| s.parse().ok()).unwrap_or(600);
    let warmup: f64 = arg("--warmup").and_then(|s| s.parse().ok()).unwrap_or(70.0);
    let agent = args.iter().any(|a| a == "--agent");
    let console = agent && args.iter().any(|a| a == "--console");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().expect("repo root");

    let params = (simulation_aircraft::find("t1s-vtol").unwrap().params)();
    let fbwa = Fbwa::from_parms(&[
        &std::fs::read_to_string(root.join("ardupilot/t1s.parm")).unwrap(),
        &std::fs::read_to_string(root.join("ardupilot/t1s-vtol.parm")).unwrap(),
    ]);
    let field = Field { pilot: Vec3::ZERO, heading: 0.0 };
    let planes: Vec<FlightState> = (0..3).map(|i| FlightState::new(&params, &field.hand(i))).collect();
    let mut session = Session::tag(field).with_autopilots(params.clone(), fbwa, planes);
    // Player 0 is not playing, so the first chaser is the first computer pilot.
    session.tag_ref_mut().expect("tag").it = 1;
    let mut bench = FlightState::new(&params, &Start { pos: field.pilot, heading: 0.0 });
    bench.phase = Phase::Crashed;
    let world = Arc::new(Mutex::new(World { session, bench, params: params.clone(), packets: [0; 3], warmup }));

    for i in 0..3usize {
        let sock = UdpSocket::bind(("0.0.0.0", port_for(BASE_PORT, i as u16 + 1)))
            .expect("bind the physics port (is a sim running?)");
        let w = world.clone();
        std::thread::Builder::new()
            .name(format!("sitl-{}", i + 1))
            .spawn(move || {
                Lockstep::default().serve(
                    sock,
                    move |_, pkt, dt| {
                        let mut g = w.lock().unwrap();
                        g.packets[i] += 1;
                        step_plane(&mut g, i, &pkt.pwm, dt)
                    },
                    || {},
                )
            })
            .unwrap();
    }

    let dir = std::env::temp_dir().join("sim001-runner");
    // With the agent, each plane's MAVLink system id is its player number (1 to 3): the id the
    // others know it by in plans and on the ADS-B uplink (an AGENT plane ignores a track with its
    // own id, and every SITL is 1 by default).
    let fleet = if agent {
        ground::Fleet::start(&root, &dir, speedup)
    } else {
        launch::Fleet::start(&root, &dir, "t1s-vtol", 1..4, launch::HOME, speedup).map(ground::Fleet::Stock)
    };
    let _fleet = match fleet {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    println!("three SITL planes starting at {speedup}x (they need about a minute of their own time before they arm)");
    let mut ground = agent.then(|| {
        let names: Vec<&'static str> = recreation_games::PILOTS.iter().map(|p| p.0).collect();
        let ports: Vec<u16> = (1..4).map(|n| launch::MAVLINK_PORT + 10 * n).collect();
        let home = cognition_agent::parse_home(launch::HOME).expect("the SITL home");
        println!(
            "--agent: the ground control agent flies the planes once they are on the wing (MAVLink tcp {ports:?})"
        );
        ground::Ground::connect(&names, &ports, home)
    });

    let start = Instant::now();
    let mut go_wall: Option<(Instant, f32)> = None;
    let mut last_report = Instant::now();
    // Who was it, and everyone's tags: to tell a tag from a plane leaving the arena.
    let mut was: Option<(usize, Vec<u32>)> = None;
    loop {
        if let Some(ground) = &mut ground {
            std::thread::sleep(Duration::from_millis(20));
            let (picture, wing, now) = {
                let g = world.lock().unwrap();
                let tag = g.session.tag_ref().unwrap();
                let tags: Vec<u32> = tag.players.iter().map(|p| p.tags).collect();
                if let Some((it, before)) = &was
                    && *it != tag.it
                {
                    let (from, to) = (g.session.name(*it), g.session.name(tag.it));
                    if tags[*it] > before[*it] {
                        println!("  [game {:>5.1}s] {from} tagged {to}", tag.time);
                    } else {
                        println!("  [game {:>5.1}s] {to} left the arena and is it (was {from})", tag.time);
                    }
                }
                was = Some((tag.it, tags));
                let wing: Vec<bool> = (0..3).map(|i| g.session.on_the_wing(i)).collect();
                let (lat, lon, _) = ground_home();
                (ground::picture(&g.session, (lat, lon)), wing, g.session.opponents[0].state.time)
            };
            ground.tick(&picture, &wing, now);
            if last_report.elapsed() < Duration::from_secs(2) {
                continue;
            }
        } else {
            std::thread::sleep(Duration::from_secs(2));
        }
        last_report = Instant::now();
        let g = world.lock().unwrap();
        let tag = g.session.tag_ref().unwrap();
        let clock = format!("{:?}", g.session.clock);
        if go_wall.is_none() && matches!(g.session.clock, recreation_games::Clock::Running { .. }) {
            go_wall = Some((Instant::now(), tag.time));
        }
        let planes: Vec<String> = g
            .session
            .opponents
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let it = if tag.it == i + 1 { "*" } else { "" };
                format!(
                    "{}{} {:?} {:.0} m {:.0} m/s it {:.0}s tags {}",
                    o.name,
                    it,
                    o.state.phase,
                    o.state.pos.y,
                    o.state.airspeed,
                    tag.players[i + 1].it_time,
                    tag.players[i + 1].tags
                )
            })
            .collect();
        println!("[{:>3}s] {clock} | {} | packets {:?}", start.elapsed().as_secs(), planes.join(" | "), g.packets);
        if let Some(ground) = &ground {
            println!("        it: {}", g.session.name(tag.it));
            for (i, o) in g.session.opponents.iter().enumerate() {
                println!("        {:<8} {}", o.name, ground.describe(i));
            }
        }
        if tag.over() {
            let (t0, g0) = go_wall.unwrap_or((start, 0.0));
            let secs = t0.elapsed().as_secs_f32();
            println!("game over. The least time as it wins:");
            for (i, o) in g.session.opponents.iter().enumerate() {
                println!("  {}: it for {:.1} s, {} tags", o.name, tag.players[i + 1].it_time, tag.players[i + 1].tags);
            }
            println!(
                "{:.0} s of game in {:.1} s: {:.1}x real time",
                tag.time - g0,
                secs,
                (tag.time - g0) / secs.max(0.001)
            );
            if let Some(ground) = &ground {
                for line in ground.verdict() {
                    println!("{line}");
                }
                for line in ground.results() {
                    println!("{line}");
                }
                println!("RESULT it={}", g.session.name(tag.it));
            }
            drop(g);
            if let (true, Some(link)) = (console, ground.as_ref().and_then(|gr| gr.link(0))) {
                let home = ground_home();
                let now = || world.lock().unwrap().session.opponents[0].state.time;
                // The first plane goes on hearing the others as ADSB_VEHICLE, twice a second of
                // their time, until `silent`.
                let uplink = AtomicBool::new(true);
                let stop = AtomicBool::new(false);
                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        let names: Vec<&str> = recreation_games::PILOTS.iter().map(|p| p.0).collect();
                        let mut next = 0.0;
                        while !stop.load(Ordering::Relaxed) {
                            let (picture, t) = {
                                let g = world.lock().unwrap();
                                (ground::picture(&g.session, (home.0, home.1)), g.session.opponents[0].state.time)
                            };
                            if uplink.load(Ordering::Relaxed) && t >= next {
                                next = t + 0.5;
                                for c in picture.planes.iter().filter(|c| c.flying && c.id != 1) {
                                    let name = names.get(c.id as usize - 1).copied().unwrap_or("PLANE");
                                    let _ = link.send_traffic(&Traffic::from_craft(c, home, name));
                                }
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    });
                    console::Console::new(link, (home.0, home.1), &uplink, now)
                        .run(std::io::stdin().lock(), std::io::stdout().lock());
                    stop.store(true, Ordering::Relaxed);
                });
            }
            return;
        }
        if start.elapsed().as_secs() > wall_limit {
            eprintln!("timed out after {wall_limit} s");
            for line in ground.iter().flat_map(|gr| gr.verdict()) {
                eprintln!("{line}");
            }
            std::process::exit(1);
        }
    }
}

/// SITL's home (latitude, longitude, altitude AMSL): where the pilot box is.
fn ground_home() -> (f64, f64, f32) {
    cognition_agent::parse_home(launch::HOME).expect("the SITL home")
}

/// One frame of computer pilot `i`'s plane, as the sim's `step_opponent` does it; computer pilot 0
/// also advances the game, by the same time.
fn step_plane(g: &mut World, i: usize, pwm: &[u16; 16], dt: f32) -> String {
    let cmd = ActuatorCommands::from_pwm(&g.params, pwm);
    let env = Environment { wind: WIND };
    let radio = {
        let s = &mut g.session.opponents[i].state;
        step(&g.params, s, &cmd, &env, &Flat, &[], dt);
        g.session.radio(i, &Flat, dt)
    };
    if i == 0 && g.session.opponents[0].state.time >= g.warmup {
        let wind = |_: Vec3, _: f64| WIND;
        let surroundings = Surroundings { ground: &Flat, obstacles: &[], wind: &wind };
        let bench = g.bench.clone();
        g.session.step(&bench, &surroundings, dt);
    }
    sensor_json(&g.session.opponents[i].state, WIND, &radio_channels(&radio), Vec3::ZERO)
}

/// The RC channels a computer pilot's radio sends (AETR, the arm switch on 5, the mode on 8).
fn radio_channels(r: &Radio) -> [u16; 8] {
    let c = |x: f32| (1500.0 + 500.0 * x.clamp(-1.0, 1.0)).round() as u16;
    let s = r.sticks;
    [
        c(s.aileron),
        c(s.elevator),
        (1000.0 + 1000.0 * s.throttle.clamp(0.0, 1.0)).round() as u16,
        c(s.rudder),
        if r.arm { 1900 } else { 1100 },
        1500,
        1500,
        simulation_aircraft::rc_mode_pwm(r.mode),
    ]
}
