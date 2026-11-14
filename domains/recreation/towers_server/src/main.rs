//! The game server of Towers (see plans/towers.md): `cargo run --release -p
//! recreation_towers_server -- [--speedup N] [--blue computer|person|nobody] [--red computer|person|nobody]
//! [--on-leave computer|wait]`.
//!
//! Each drone has an ArduPlane SITL of its own, flying the plane of its class (`recreation_towers::Class`: standard T1-S VTOLs, a heavy T2 Cruza and a light Seeker 3) (MAVLink system ids 1 to 10), with
//! the AGENT mode and `ardupilot/towers.parm`. The SITL instances are 30 to 39
//! (`--first-instance`): clear of the station's (0 to 3), the runner's (1 to 3), and of TCP 5900
//! (macOS screen sharing). The server is their host: it answers each SITL's servo packets with
//! the drone's physics, and its radio takes the drone off at GO. Once a drone flies on the wing, the ground switches it to AGENT and flies its order by
//! plan. The referee (`recreation_towers::Game`) steps with the physics.
//!
//! Each team has a seat on TCP (`--seat-port`, 5955), and the same seat for a person at the
//! commander app and for a computer commander: the team's ground station, which gets the umpire's
//! word, what its towers and base report and the way to its drones (the air), and makes its own map
//! from its drones' telemetry. The server decides and sends no plan for any team: for `computer` (the
//! default) it starts a computer commander (`recreation_towers_ground::commander`, the same as the
//! binary `towers_commander`) as a thread of its own that takes the seat over loopback TCP; for
//! `nobody` one that gives no orders. What the server does itself is the flight of the light quads
//! from the orders the seat gave, the take-off, the referee and the air.
//!
//! A seat changes hands during the game. A person who joins a team that a computer commands takes
//! the seat (the computer is relieved; the person's seat starts with the orders the drones have). When
//! a person leaves, the seat is free; a second later, with `--on-leave computer` (the default), a
//! computer commander takes it, and with `--on-leave wait` the drones fly their last orders until a
//! person comes back. `--blue person` still means that GO waits for a person. One person for each
//! team may play. A seat gets only its team's map. GO waits for the people, and for ArduPilot,
//! which needs about a minute of its own time before it arms (`--warmup`). Needs the AGENT build:
//! `SIM001_BIN`, else `.ardupilot-agent/build/sitl/bin`.

mod air;
mod seats;
mod terrain;
mod trees;
mod world;

use glam::Vec3;
use recreation_games::Radio;
use recreation_towers::{Advisor, Class, Outcome, Rules, Team};
use recreation_towers_ground::commander::{self, Brain};
use recreation_towers_ground::station::Tally;
use simulation_sitl::{Lockstep, launch, port_for, sensor_json};
use std::net::{TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use world::World;

const BASE_PORT: u16 = 9002;
/// The advisors look once a second of game time.
const DECIDE_EVERY: f32 = 1.0;
/// A seat that a person left is free this long (s) before a computer takes it.
const GRACE: Duration = Duration::from_secs(1);
/// The server stops if no drone has sent a packet after this long.
const NO_PACKETS_AFTER: Duration = Duration::from_secs(60);
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
    /// A computer takes the seat that a person leaves.
    on_leave_computer: bool,
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
        on_leave_computer: arg("--on-leave").is_none_or(|v| v != "wait"),
        stop_with_stdin: args.iter().any(|a| a == "--stop-with-stdin"),
    }
}

/// The last instance number a block may reach (the MAVLink ports go up 10 a time from 5760).
const MAX_INSTANCE: u16 = 500;

/// Binds the physics port of each of instances `first..first + n`, and checks that their MAVLink
/// ports are free too; all or none.
fn bind_physics(first: u16, n: usize) -> std::io::Result<Vec<UdpSocket>> {
    (0..n as u16)
        .map(|i| {
            drop(TcpListener::bind(("0.0.0.0", simulation_sitl::mavlink_port(first + i)))?);
            UdpSocket::bind(("0.0.0.0", port_for(BASE_PORT, first + i)))
        })
        .collect()
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
    let program = launch::agent_binary(root);
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

fn main() {
    let mut a = args();
    // The ArduPilot parameters are in the checkout, or in the app bundle's resources (SIM001_HOME).
    let root = std::env::var_os("SIM001_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().expect("repo root")
    });
    // The server cannot put a drone down when its battery is empty yet (plans/towers-rules-v2.md,
    // step 11): the drones fly with no battery.
    let rules = Rules::default().without_batteries();
    let world = Arc::new(Mutex::new(World::new(rules, a.warmup)));
    let n = 2 * rules.drones;

    // A sim or another game may hold the ports of the instances: take the next free block.
    let socks = loop {
        match bind_physics(a.first_instance, n) {
            Ok(socks) => break socks,
            Err(e) if a.first_instance + 2 * n as u16 <= MAX_INSTANCE => {
                eprintln!(
                    "instances {} to {}: {e}; trying the next block",
                    a.first_instance,
                    a.first_instance + n as u16 - 1
                );
                a.first_instance += n as u16;
            }
            Err(e) => {
                eprintln!("no free physics ports: {e}");
                std::process::exit(2);
            }
        }
    };
    for (i, sock) in socks.into_iter().enumerate() {
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
    // The ground reaches each drone through the air: a frame goes only while the radio reaches it.
    let air: Vec<air::Channel> = (0..n)
        .map(|i| {
            let sitl = launch::MAVLINK_PORT + 10 * (a.first_instance + i as u16);
            let w = world.clone();
            let w2 = world.clone();
            air::open(
                sitl,
                Arc::new(move || w.lock().unwrap().in_reach(i)),
                Arc::new(move || w2.lock().unwrap().sense(i)),
            )
            .expect("the air for a drone")
        })
        .collect();
    let air_ports: Arc<Vec<u16>> = Arc::new(air.iter().map(|c| c.port).collect());
    // Clients find the seats at the port printed below (the next one up if this one is taken).
    let port_cell = Arc::new(std::sync::atomic::AtomicU16::new(0));
    // The computer commanders are seats of their own, in this process, over loopback TCP.
    let tally = Arc::new(Tally::default());
    let spawn_commander: Arc<dyn Fn(Team, Brain) + Send + Sync> = {
        let (port_cell, tally) = (port_cell.clone(), tally.clone());
        Arc::new(move |team, brain| {
            let (addr, tally) = (format!("127.0.0.1:{}", port_cell.load(Ordering::SeqCst)), tally.clone());
            std::thread::Builder::new()
                .name(format!("commander-{team:?}"))
                .spawn(move || commander::run(&addr, team, brain, Some(tally), &AtomicBool::new(false)))
                .expect("a commander thread");
        })
    };
    let take: Arc<dyn Fn(Team) + Send + Sync> = {
        let (spawn, on_leave) = (spawn_commander.clone(), a.on_leave_computer);
        Arc::new(move |team| {
            if on_leave {
                spawn(team, Brain::Play);
            }
        })
    };
    let seats = (a.seat_port..=a.seat_port.saturating_add(20))
        .find_map(|port| {
            let free = seats::Free { grace: GRACE, take: take.clone() };
            let seats = seats::Seats::listen(port, world.clone(), a.people, air_ports.clone(), free).ok()?;
            port_cell.store(seats.port(), Ordering::SeqCst);
            a.seat_port = port;
            Some(seats)
        })
        .unwrap_or_else(|| {
            eprintln!("can't take a seat port from {} up", a.seat_port);
            std::process::exit(2);
        });
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
    println!(
        "{n} SITL drones (instances {} to {}) starting at {}x; seats on tcp {} (connect with --connect 127.0.0.1:{})",
        a.first_instance,
        a.first_instance + n as u16 - 1,
        a.speedup,
        a.seat_port,
        a.seat_port
    );
    // A person who commands a team has an advisor.
    let mut advisors: Vec<(Team, Advisor)> =
        Team::BOTH.into_iter().filter(|t| a.people[t.index()]).map(|t| (t, Advisor::new(rules))).collect();
    for t in Team::BOTH.into_iter().filter(|t| !a.people[t.index()]) {
        spawn_commander(t, if a.nobody[t.index()] { Brain::Idle } else { Brain::Play });
    }

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
            // A SITL that never answers (no `.ardupilot` in this checkout, a build that is not
            // there) leaves the game waiting for ever: stop with the reason.
            if start.elapsed() > NO_PACKETS_AFTER && w.packets.iter().all(|&p| p == 0) {
                eprintln!(
                    "error: no drone sent a physics packet in {} s: is `.ardupilot` here (a fresh git worktree has none: symlink the main checkout's), and is SIM001_BIN the AGENT build?",
                    NO_PACKETS_AFTER.as_secs()
                );
                std::process::exit(2);
            }
            // GO waits for the people.
            if !seats.people_seated() && !w.started() {
                w.go = w.go.max(w.now() + 1.0);
            }
            if w.started() && w.game.time >= next_decision {
                next_decision = w.game.time + DECIDE_EVERY;
                for (team, advisor) in &mut advisors {
                    let said = advisor.look(&w.game.view(*team));
                    if !said.is_empty() {
                        seats.advise(*team, &said);
                    }
                }
            }
        }
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
                "        air {:?}",
                air.iter().map(|c| (c.counts.carried(), c.counts.dropped())).collect::<Vec<_>>()
            );
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
                    tally.lines.lock().unwrap().get(i).cloned().unwrap_or_default(),
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
            let (in_agent, refused) = (tally.in_agent(), tally.refused());
            for line in tally.warnings() {
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
            let light: Vec<usize> = w.of_class(Class::Light).collect();
            let away = light.iter().filter_map(|&i| w.away[i]).reduce(f32::min).unwrap_or(f32::MAX);
            println!("RESULT light_away={away:.1}");
            println!("RESULT light_top_speed={:.1}", light.iter().map(|&i| w.top_speed[i]).fold(0.0, f32::max));
            println!("RESULT light_low={}", light.iter().filter(|&&i| w.lows[i]).count());
            println!("RESULT air_carried={}", air.iter().map(|c| c.counts.carried()).sum::<u64>());
            println!("RESULT air_dropped={}", air.iter().map(|c| c.counts.dropped()).sum::<u64>());
            println!("RESULT sensed={}", air.iter().map(|c| c.counts.sensed()).sum::<u64>());
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
            for line in tally.warnings() {
                eprintln!("{line}");
            }
            drop(w);
            drop(fleet);
            std::process::exit(1);
        }
    }
}
