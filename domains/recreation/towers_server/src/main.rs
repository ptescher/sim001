//! The game server of Towers (see plans/towers.md): `cargo run --release -p
//! recreation_towers_server -- [--speedup N] [--blue computer|person] [--red computer|person]`.
//!
//! Each drone has an ArduPlane SITL of its own, flying the plane of its class (`recreation_towers::Class`: standard T1-S VTOLs, a heavy T2 Cruza and a light Seeker 3) (MAVLink system ids 1 to 10), with
//! the AGENT mode and `ardupilot/towers.parm`. The SITL instances are 30 to 39
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
mod terrain;
mod trees;
mod world;

use glam::Vec3;
use recreation_games::Radio;
use recreation_towers::{Class, Commander, Outcome, Rules, Team};
use simulation_sitl::{Lockstep, launch, port_for, sensor_json};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Teams nobody commands.
    nobody: [bool; 2],
    stop_with_stdin: bool,
}

fn args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let person = |name: &str| arg(name).is_some_and(|v| v == "person");
    let nobody = |name: &str| arg(name).is_some_and(|v| v == "nobody");
    Args {
        speedup: arg("--speedup").and_then(|s| s.parse().ok()).unwrap_or(1.0),
        timeout: arg("--timeout").and_then(|s| s.parse().ok()).unwrap_or(3600),
        warmup: arg("--warmup").and_then(|s| s.parse().ok()).unwrap_or(70.0),
        seat_port: arg("--seat-port").and_then(|s| s.parse().ok()).unwrap_or(recreation_towers::seat::PORT),
        first_instance: arg("--first-instance").and_then(|s| s.parse().ok()).unwrap_or(30),
        people: [person("--blue"), person("--red")],
        nobody: [nobody("--blue"), nobody("--red")],
        stop_with_stdin: args.iter().any(|a| a == "--stop-with-stdin"),
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
///
/// ArduPilot takes the place where it starts as its home, and the AGENT fence is round its home:
/// each drone starts at its team's base, so its fence is round the base. A parameter file for each
/// drone moves the fence's centre (`AGENT_FENCE_N`, `_E`, metres from home) to the middle of the
/// field.
fn start_fleet(root: &Path, dir: &Path, rules: &Rules, first: u16, n: u16, speedup: f32) -> Result<Fleet, String> {
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
        // Drone `k - 1` flies the plane of its class, starts at its team's base, and the field's
        // middle is that far from its home.
        let (team, class) = if usize::from(k - 1) < rules.drones {
            (Team::Blue, rules.class(usize::from(k - 1)))
        } else {
            (Team::Red, rules.class(usize::from(k - 1) - rules.drones))
        };
        let mut args = launch::native_instance_args(root, class.aircraft(), first + k - 1, launch::HOME, speedup);
        let middle = -rules.base(team);
        let instance = dir.join(format!("instance{k}"));
        std::fs::create_dir_all(&instance).map_err(|e| format!("{}: {e}", instance.display()))?;
        let fence = instance.join("fence.parm");
        std::fs::write(&fence, format!("AGENT_FENCE_N {:.0}\nAGENT_FENCE_E {:.0}\n", middle.x, middle.y))
            .map_err(|e| format!("{}: {e}", fence.display()))?;
        if let Some(i) = args.iter().position(|a| a == "--defaults") {
            args[i + 1] = format!("{},{towers},{}", args[i + 1], fence.display());
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
    // The server cannot put a drone down when its battery is empty yet (plans/towers-rules-v2.md,
    // step 11): the drones fly with no battery.
    let rules = Rules::default().without_batteries();
    let world = Arc::new(Mutex::new(World::new(rules, a.warmup)));
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
    let fleet = match start_fleet(
        &root,
        &std::env::temp_dir().join("sim001-towers"),
        &rules,
        a.first_instance,
        n as u16,
        a.speedup,
    ) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    println!("{n} SITL drones starting at {}x; seats on tcp {}", a.speedup, a.seat_port);
    let names: Vec<String> = (0..n).map(|i| name(&rules, i)).collect();
    let ports: Vec<u16> = (0..n as u16).map(|i| launch::MAVLINK_PORT + 10 * (a.first_instance + i)).collect();
    // Light drones have no wing for AGENT to fly: they are flown by the radio, in QLOITER.
    let modes: Vec<u32> = (0..n)
        .map(|i| if rules.class(i % rules.drones) == Class::Light { ground::QLOITER_MODE } else { ground::AGENT_MODE })
        .collect();
    let home = cognition_agent::parse_home(launch::HOME).expect("the SITL home");
    let mut ground = ground::Ground::connect(&names, &modes, &ports, home);
    let mut commanders: Vec<(Team, Commander)> = Team::BOTH
        .into_iter()
        .filter(|t| !a.people[t.index()] && !a.nobody[t.index()])
        .map(|t| (t, Commander::new(rules)))
        .collect();

    let stop = Arc::new(AtomicBool::new(false));
    if a.stop_with_stdin {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            stop.store(true, Ordering::SeqCst);
        });
    }
    let start = Instant::now();
    // The report: every 2 s, or `TOWERS_REPORT` (s) if that is set.
    let report_every =
        Duration::from_secs_f32(std::env::var("TOWERS_REPORT").ok().and_then(|s| s.parse().ok()).unwrap_or(2.0f32));
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
                        w.command(i, order, false);
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
        if last_report.elapsed() >= report_every || w.game.outcome.is_some() {
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
                let d = &g.drones[i];
                println!(
                    "        {} {:?} {:?}{} | at {:.0}, {:.0} {:.0} m, {:.0}, {:.0} m/s, heading {:.0}, roll {:.0}, airspeed {:.1}",
                    ground.describe(i),
                    d.phase,
                    w.orders[i],
                    if w.lows[i] { " LOW" } else { "" },
                    d.at.x,
                    d.at.y,
                    d.alt,
                    d.vel.x,
                    d.vel.y,
                    simulation_physics::euler(w.planes[i].rot).2.to_degrees(),
                    simulation_physics::euler(w.planes[i].rot).0.to_degrees(),
                    w.planes[i].airspeed
                );
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
        if stop.load(Ordering::SeqCst) {
            println!("the input closed: stopping");
            drop(w);
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
