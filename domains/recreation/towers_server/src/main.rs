//! The game server of Towers (see plans/towers.md): `cargo run --release -p
//! recreation_towers_server -- [--speedup N] [--blue computer|person] [--red computer|person]`.
//!
//! Each drone is a T1-S VTOL with an ArduPlane SITL of its own (MAVLink system ids 1 to 6), with
//! the AGENT mode and `ardupilot/towers.parm`. The SITL instances are 30 to 35
//! (`--first-instance`): clear of the station's (0 to 3), the runner's (1 to 3), and of TCP 5900
//! (macOS screen sharing). The server is their host: it answers each SITL's servo packets with
//! the drone's physics, and its radio takes the drone off at GO. Once a drone flies on the wing, the ground switches it to AGENT and flies its order by
//! plan. The referee (`recreation_towers::Game`) steps with the physics.
//!
//! Each team has a seat on TCP (`--seat-port`, 5955): a computer commander, or a person at the
//! commander app. A seat gets only its team's map. GO waits for the people, and for ArduPilot,
//! which needs about a minute of its own time before it arms (`--warmup`). Needs the AGENT build:
//! `SIM001_BIN`, else `.ardupilot-agent/build/sitl/bin`.

mod ground;
mod seats;
mod world;

use glam::Vec3;
use recreation_games::Radio;
use recreation_towers::{Commander, Outcome, Rules, Team};
use simulation_sitl::{Lockstep, launch, port_for, sensor_json};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use world::World;

const BASE_PORT: u16 = 9002;
/// The computer commanders decide once a second of game time.
const DECIDE_EVERY: f32 = 1.0;
/// The seats get the game four times a second.
const SEND_EVERY: Duration = Duration::from_millis(250);

struct Args {
    speedup: f32,
    timeout: u64,
    warmup: f64,
    seat_port: u16,
    first_instance: u16,
    people: [bool; 2],
}

fn args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let person = |name: &str| arg(name).is_some_and(|v| v == "person");
    Args {
        speedup: arg("--speedup").and_then(|s| s.parse().ok()).unwrap_or(1.0),
        timeout: arg("--timeout").and_then(|s| s.parse().ok()).unwrap_or(3600),
        warmup: arg("--warmup").and_then(|s| s.parse().ok()).unwrap_or(70.0),
        seat_port: arg("--seat-port").and_then(|s| s.parse().ok()).unwrap_or(recreation_towers::seat::PORT),
        first_instance: arg("--first-instance").and_then(|s| s.parse().ok()).unwrap_or(30),
        people: [person("--blue"), person("--red")],
    }
}

/// The drones' SITL processes, stopped when dropped.
struct Fleet(Vec<Child>);

impl Drop for Fleet {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Starts SITL instances `first..first + n`, with system ids 1 to `n` and the Towers parameters
/// on top of the T1-S VTOL's.
fn start_fleet(root: &Path, dir: &Path, first: u16, n: u16, speedup: f32) -> Result<Fleet, String> {
    let program = launch::native_binary(root);
    if !program.exists() {
        return Err(format!(
            "no ArduPlane at {} (build the AGENT mode with `AP_DIR=.ardupilot-agent AGENT_EXEC=1 ardupilot/build_native.sh` and set SIM001_BIN)",
            program.display()
        ));
    }
    let towers = root.join("ardupilot/towers.parm").display().to_string();
    let mut fleet = Fleet(vec![]);
    for k in 1..=n {
        let mut args = launch::native_instance_args(root, "t1s-vtol", first + k - 1, launch::HOME, speedup);
        if let Some(i) = args.iter().position(|a| a == "--defaults") {
            args[i + 1] = format!("{},{towers}", args[i + 1]);
        }
        args.extend(["--sysid".to_string(), k.to_string()]);
        let (child, _) =
            launch::spawn(&dir.join(format!("instance{k}")), launch::Plan::Native { program: program.clone(), args })?;
        fleet.0.push(child);
    }
    Ok(fleet)
}

/// The RC channels a radio sends (AETR, the arm switch on 5, the mode on 8).
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

fn name(rules: &Rules, i: usize) -> String {
    let team = if i < rules.drones { "Blue" } else { "Red" };
    format!("{team}{}", i % rules.drones + 1)
}

fn main() {
    let a = args();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().expect("repo root");
    let rules = Rules::default();
    let params = (simulation_aircraft::find("t1s-vtol").unwrap().params)();
    let world = Arc::new(Mutex::new(World::new(rules, params, a.warmup)));
    let n = 2 * rules.drones;

    for i in 0..n {
        let sock = UdpSocket::bind(("0.0.0.0", port_for(BASE_PORT, a.first_instance + i as u16)))
            .expect("bind a physics port (is a sim or another game running?)");
        let w = world.clone();
        std::thread::Builder::new()
            .name(format!("sitl-{}", i + 1))
            .spawn(move || {
                Lockstep::default().serve(
                    sock,
                    move |_, pkt, dt| {
                        let mut w = w.lock().unwrap();
                        let radio = w.step_drone(i, &pkt.pwm, dt);
                        sensor_json(&w.planes[i], Vec3::ZERO, &radio_channels(&radio), Vec3::ZERO)
                    },
                    || {},
                )
            })
            .unwrap();
    }
    let seats = match seats::Seats::listen(a.seat_port, world.clone(), a.people) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("can't take the seat port {}: {e}", a.seat_port);
            std::process::exit(2);
        }
    };
    let fleet =
        match start_fleet(&root, &std::env::temp_dir().join("sim001-towers"), a.first_instance, n as u16, a.speedup) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        };
    println!("six SITL drones starting at {}x; seats on tcp {}", a.speedup, a.seat_port);
    let names: Vec<String> = (0..n).map(|i| name(&rules, i)).collect();
    let ports: Vec<u16> = (0..n as u16).map(|i| launch::MAVLINK_PORT + 10 * (a.first_instance + i)).collect();
    let home = cognition_agent::parse_home(launch::HOME).expect("the SITL home");
    let mut ground = ground::Ground::connect(&names, &ports, home);
    let mut commanders: Vec<(Team, Commander)> =
        Team::BOTH.into_iter().filter(|t| !a.people[t.index()]).map(|t| (t, Commander::new(rules))).collect();

    let start = Instant::now();
    let (mut last_send, mut last_report) = (Instant::now(), Instant::now());
    let mut next_decision = 0.0;
    loop {
        std::thread::sleep(Duration::from_millis(20));
        {
            let mut w = world.lock().unwrap();
            // GO waits for the people.
            if !seats.people_seated() && !w.started() {
                w.go = w.go.max(w.now() + 1.0);
            }
            if w.started() && w.game.time >= next_decision {
                next_decision = w.game.time + DECIDE_EVERY;
                for (team, c) in &mut commanders {
                    let view = w.game.view(*team);
                    for (k, order) in c.decide(&view) {
                        let i = w.game.drone(*team, k);
                        w.orders[i] = Some(order);
                    }
                }
            }
        }
        ground.tick(&world);
        if last_send.elapsed() >= SEND_EVERY {
            last_send = Instant::now();
            let mut w = world.lock().unwrap();
            // Each drone's physics moves it on its own: look again, so that every map is of this
            // moment, the moment the spectator gets.
            w.game.look();
            seats.send(&w);
        }
        let mut w = world.lock().unwrap();
        if last_report.elapsed() >= Duration::from_secs(2) || w.game.outcome.is_some() {
            last_report = Instant::now();
            let g = &w.game;
            let towers: Vec<String> = g
                .towers
                .iter()
                .map(|t| format!("{} {:+.0}%{}", t.name, 100.0 * t.control, if t.contested { "!" } else { "" }))
                .collect();
            println!(
                "[{:>3}s] t {:.0}s ({}) | Blue {:.0} Red {:.0} | {} | packets {:?}",
                start.elapsed().as_secs(),
                g.time,
                if w.started() { "playing" } else { "waiting for GO" },
                g.score[0],
                g.score[1],
                towers.join(", "),
                w.packets
            );
            for i in 0..n {
                println!("        {} {:?} {:?}", ground.describe(i), g.drones[i].phase, w.orders[i]);
            }
        }
        if let Some(outcome) = w.game.outcome {
            w.game.look();
            seats.send(&w);
            let (in_agent, refused) = ground.results();
            for line in ground.verdict() {
                println!("{line}");
            }
            println!("game over: {outcome:?}, Blue {:.0} Red {:.0}", w.game.score[0], w.game.score[1]);
            let winner = match outcome {
                Outcome::Won(t) => format!("{t:?}"),
                Outcome::Draw => "draw".into(),
            };
            println!("RESULT outcome={winner}");
            println!("RESULT in_agent={in_agent}");
            println!("RESULT refused={refused}");
            println!("RESULT crashes={}", w.crashes);
            println!("RESULT taken_blue={}", w.taken[0]);
            println!("RESULT taken_red={}", w.taken[1]);
            drop(w);
            std::thread::sleep(Duration::from_millis(500));
            drop(fleet);
            return;
        }
        if start.elapsed().as_secs() > a.timeout {
            eprintln!("timed out after {} s", a.timeout);
            for line in ground.verdict() {
                eprintln!("{line}");
            }
            drop(w);
            drop(fleet);
            std::process::exit(1);
        }
    }
}
