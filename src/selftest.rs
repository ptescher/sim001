//! Headless flight-model checks: `cargo run --release --bin sim001 -- --selftest`.

use crate::sim::{FlightMode, SimCore};
use crate::terrain;
use bevy::math::{Quat, Vec3};
use simulation_physics::{self as physics, Phase};

const DT: f32 = 1.0 / 600.0;

fn core() -> SimCore {
    let mut c = SimCore::new((simulation_aircraft::find("t1s-ranger").unwrap().params)(), vec![], false, 9002);
    c.wind.speed = 0.0;
    c.wind.turbulence = 0.0;
    c
}

fn airborne(c: &mut SimCore, speed: f32, alt: f32) {
    c.state.phase = Phase::Flying;
    c.state.pos = Vec3::new(0.0, alt, 0.0);
    c.state.rot = Quat::IDENTITY;
    c.state.vel = Vec3::NEG_Z * speed;
    c.state.omega = Vec3::ZERO;
}

fn run(c: &mut SimCore, secs: f32) {
    for _ in 0..(secs / DT) as usize {
        c.step_standalone(DT);
    }
}

pub fn run_all() {
    let p = (simulation_aircraft::find("t1s-ranger").unwrap().params)();
    let wing_area: f32 = p.surfaces.iter().filter(|s| !s.is_tail).map(|s| s.area).sum();
    println!(
        "T1-S model: mass {:.2} kg, wing area {:.4} m^2, wing loading {:.1} g/dm^2",
        p.mass,
        wing_area,
        p.mass * 1000.0 / (wing_area * 100.0)
    );

    // 1. Trimmed level flight vs throttle (manual, sticks centred).
    println!("\n-- Hands-off flight, manual mode, sticks centred --");
    for thr in [0.0, 0.3, 0.45, 0.6, 0.8, 1.0] {
        let mut c = core();
        airborne(&mut c, 14.0, 300.0);
        c.sticks.throttle = thr;
        run(&mut c, 25.0);
        let y0 = c.state.pos.y;
        run(&mut c, 5.0);
        let (roll, pitch, _) = physics::euler(c.state.rot);
        println!(
            "thr {:.2}: airspeed {:5.1} m/s, climb {:+5.2} m/s, pitch {:+5.1}°, roll {:+5.1}°, AoA(wing) {:+5.1}°, current {:4.1} A",
            thr,
            c.state.airspeed,
            (c.state.pos.y - y0) / 5.0,
            pitch.to_degrees(),
            roll.to_degrees(),
            c.state.alpha[1].to_degrees(),
            c.state.current
        );
    }

    // 2. Stall: idle, slowly feed in up elevator.
    println!("\n-- Power-off stall --");
    {
        let mut c = core();
        airborne(&mut c, 13.0, 300.0);
        c.sticks.throttle = 0.0;
        let mut min_speed = f32::MAX;
        let mut stalled_at = None;
        for i in 0..(10.0 / DT) as usize {
            c.sticks.pitch = (i as f32 * DT / 6.0).min(1.0);
            c.step_standalone(DT);
            if c.state.time > 1.0 {
                min_speed = min_speed.min(c.state.airspeed);
            }
            if stalled_at.is_none() && c.state.stalled.iter().take(4).any(|&s| s) {
                stalled_at = Some((c.state.airspeed, c.state.time));
            }
        }
        if let Some((v, t)) = stalled_at {
            println!("first wing section stall at {:.1} m/s (t={:.1}s); min airspeed {:.1} m/s", v, t, min_speed);
        } else {
            println!("no stall; min airspeed {:.1} m/s", min_speed);
        }
    }

    // 3. Control response signs and rates.
    println!("\n-- Control response at 15 m/s, half throttle --");
    for (name, roll, pitch, yaw) in [
        ("full right aileron", 1.0, 0.0, 0.0),
        ("full up elevator", 0.0, 1.0, 0.0),
        ("full right rudder", 0.0, 0.0, 1.0),
    ] {
        let mut c = core();
        airborne(&mut c, 15.0, 300.0);
        c.sticks.throttle = 0.5;
        run(&mut c, 0.5);
        c.sticks.roll = roll;
        c.sticks.pitch = pitch;
        c.sticks.yaw = yaw;
        let mut peak = (0.0f32, 0.0f32, 0.0f32);
        for _ in 0..(0.6 / DT) as usize {
            c.step_standalone(DT);
            let (p, q, r) = physics::body_rates(c.state.omega);
            if p.abs() > peak.0.abs() {
                peak.0 = p;
            }
            if q.abs() > peak.1.abs() {
                peak.1 = q;
            }
            if r.abs() > peak.2.abs() {
                peak.2 = r;
            }
        }
        println!(
            "{name:20}: peak roll {:+6.0}°/s  pitch {:+6.0}°/s  yaw {:+6.0}°/s",
            peak.0.to_degrees(),
            peak.1.to_degrees(),
            peak.2.to_degrees()
        );
    }

    // 4. Hand launch at 80% throttle.
    println!("\n-- Hand launch, 80% throttle, hands off --");
    {
        let mut c = core();
        c.reset();
        c.sticks.throttle = 0.8;
        let g0 = terrain::ground_height(c.state.pos.x, c.state.pos.z);
        for i in 0..(8.0 / DT) as usize {
            c.step_standalone(DT);
            if i % (1.0 / DT) as usize == 0 {
                let (_, pitch, _) = physics::euler(c.state.rot);
                println!(
                    "t={:4.1}s phase {:?} alt {:5.1} m airspeed {:4.1} m/s pitch {:+5.1}°",
                    c.state.time,
                    c.state.phase,
                    c.state.pos.y - g0,
                    c.state.airspeed,
                    pitch.to_degrees()
                );
            }
        }
        if c.state.phase == Phase::Crashed {
            println!("CRASHED: {}", c.state.crash_reason);
        }
    }

    // 5. Stabilized modes recover from a 60° bank.
    for mode in [FlightMode::Angle, FlightMode::Acro] {
        let mut c = core();
        airborne(&mut c, 15.0, 300.0);
        c.mode = mode;
        c.sticks.throttle = 0.5;
        c.state.rot = Quat::from_rotation_z(-60f32.to_radians());
        run(&mut c, 3.0);
        let (roll, pitch, _) = physics::euler(c.state.rot);
        println!(
            "\n{:?} mode from 60° bank after 3 s: roll {:+.1}°, pitch {:+.1}°",
            mode,
            roll.to_degrees(),
            pitch.to_degrees()
        );
    }

    // 6. Glide to landing on the runway.
    println!("\n-- Power-off glide onto the runway --");
    {
        let mut c = core();
        airborne(&mut c, 12.0, 6.0);
        c.state.pos = Vec3::new(0.0, 6.0, 40.0);
        c.mode = FlightMode::Angle;
        c.sticks.pitch = 0.0;
        run(&mut c, 12.0);
        println!(
            "phase {:?} {} | on ground {} | speed {:.2} m/s | pos ({:.0}, {:.2}, {:.0})",
            c.state.phase,
            c.state.crash_reason,
            c.state.on_ground,
            c.state.vel.length(),
            c.state.pos.x,
            c.state.pos.y,
            c.state.pos.z
        );
    }

    // 7. Battery endurance at cruise throttle.
    {
        let mut c = core();
        airborne(&mut c, 15.0, 300.0);
        c.mode = FlightMode::Angle;
        c.sticks.throttle = 0.5;
        run(&mut c, 60.0);
        println!(
            "\n50% throttle: {:.1} A, {:.2} V, {:.0} mAh/min -> ~{:.0} min on 1800 mAh",
            c.state.current,
            c.state.voltage,
            c.state.used_mah,
            1800.0 * 0.8 / c.state.used_mah
        );
    }

    // 8. SITL JSON sample.
    let mut c = core();
    c.reset();
    c.step_ardupilot(&[1500, 1500, 1000, 1500, 1000, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 1.0 / 1200.0);
    print!("\nSample JSON reply:{}", c.sitl_json());
}
