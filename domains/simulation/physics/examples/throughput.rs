//! How fast the flight model runs, headless: `cargo run --release -p simulation_physics --example throughput`.
//! Phase 0 of plans/sim2real-tag.md uses it to size the training environment.

use glam::{Quat, Vec3};
use simulation_aircraft::ActuatorCommands;
use simulation_physics::{Environment, Flat, Phase, Start, step};
use std::time::Instant;

fn main() {
    let dt = 1.0 / 400.0;
    for id in ["t1s-ranger", "t1s-vtol"] {
        let p = (simulation_aircraft::find(id).unwrap().params)();
        let mut s = simulation_physics::FlightState::new(&p, &Start { pos: Vec3::ZERO, heading: 0.0 });
        s.phase = Phase::Flying;
        s.pos = Vec3::new(0.0, 300.0, 0.0);
        s.rot = Quat::IDENTITY;
        s.vel = Vec3::NEG_Z * 15.0;
        let mut cmd = ActuatorCommands::default();
        cmd.out[2] = 0.5;
        let env = Environment { wind: Vec3::new(2.0, 0.0, 1.0) };
        let steps = 200_000;
        let t = Instant::now();
        for i in 0..steps {
            // Cost only: whatever the plane does with these commands, keep it flying high up so
            // that every step runs the whole model (a crashed plane skips it).
            s.phase = Phase::Flying;
            s.pos.y = 300.0;
            if i % 400 == 0 {
                s.rot = Quat::IDENTITY;
                s.vel = Vec3::NEG_Z * 15.0;
                s.omega = Vec3::ZERO;
            }
            step(&p, &mut s, &cmd, &env, &Flat, &[], dt);
        }
        let wall = t.elapsed().as_secs_f64();
        std::hint::black_box(&s);
        println!("  end: phase {:?}, pos {:?}, airspeed {:.1}", s.phase, s.pos, s.airspeed);
        let sim = steps as f64 * dt as f64;
        println!(
            "{id}: {steps} steps at {:.0} Hz in {wall:.2} s: {:.0} steps/s, {:.0}x real time on one core",
            1.0 / dt as f64,
            steps as f64 / wall,
            sim / wall
        );
    }
}
