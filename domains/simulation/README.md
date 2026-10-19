# Simulation

The flight model and the planes it flies, with no renderer in sight. Anything that needs to
know how a plane moves (the sim's window, the self-tests, the ArduPilot bridge, tools to come)
depends on these crates; they depend on nothing but `glam`.

## Modules

| Crate | Path | Role |
| --- | --- | --- |
| `simulation_aircraft` | [aircraft/](aircraft) | What a plane is to the physics: lifting surfaces, motors (any number, forward or lift), servo outputs, ground-contact points, mass and battery. One module per plane, and a registry: `find("t1s-ranger")`, picked with `--aircraft` |
| `simulation_sitl` | [sitl/](sitl) | ArduPilot SITL with no renderer: the JSON physics packets, the lockstep exchange (`Lockstep`), and starting ArduPlane instances, native or Docker (`launch`, `Fleet`) |
| `simulation_physics` | [physics/](physics) | The 6-DOF model: strip-theory aerodynamics, propulsion, battery, ground contact, the hand launch and the carry back after a reset. The ground comes in through the `Terrain` trait, so it runs headless |

`simulation_physics` depends on `simulation_aircraft`, never the other way round. `simulation_sitl` depends on both: it encodes a flight state as the sensor packet.

`cargo run --release -p simulation_physics --example throughput` times the flight model: about 3 million steps a second (7 000 to 8 000 times real time at 400 Hz) on one core of an Apple Silicon Mac, over flat ground with no obstacles.

## Planes

| Id | What it is |
| --- | --- |
| `t1s-vtol` | The default. The T1-S airframe with its two wing motors on tilt servos and a third rotor in the tail, and a flight controller (ArduPilot flies it: `ardupilot/t1s-vtol.parm`). It hovers, transitions to forward flight and back, and lands. The hardware numbers (motors, props, tilt mechanism, weight) are estimates, because HEEWING doesn't publish them |
| `t1s-ranger` | The plain HEEWING T1-S Ranger: twin tractor motors, V-tail, and only a receiver and two ESCs, so no flight controller. Its numbers are fixed by `sim001 --selftest`, whose output must not change when the physics is moved or refactored |

A plane with a flight controller is flown by ArduPilot. The sim starts one for itself (`src/sitl.rs`: a native build from `ardupilot/build_native.sh`, else the Docker image), or flies beside the station's. A plane without one has the MANUAL mode only, so the sim's ANGLE and ACRO modes are for planes that have one.

## How a plane is driven

A plane has up to 16 servo outputs, as ArduPilot does. Each lifting surface says which output
moves it and by how many radians per unit; each motor says which output throttles it. The
physics takes one normalized value per output (`ActuatorCommands`). ArduPilot's PWMs become
those through the plane's `outputs` (a servo is centred on 1500 µs, a motor runs 1000 to 2000 µs;
an output can be inverted), and the sim's own flight modes use the plane's mixer
(`t1s_ranger::mix`).

A motor can also have a reaction torque (a lift rotor turning one way yaws the airframe the
other), which is what yaw control on a tri-rotor is made of. A motor can be on a tilt servo, which
turns it between pointing up (a hover) and pointing forward (cruise) at a limited speed; the
thrust, the reaction torque and the prop wash over the wing all follow the tilt.

## Adding a plane

Write a module in `simulation_aircraft` with a `params()` that returns an `AircraftParams`, and
add it to `AIRCRAFT`.

## Rotors

A lift rotor is more than a thrust along an axis: it turns a reaction torque on the airframe,
makes more thrust near the ground (ground effect), loses thrust when it descends into its own
wash at about the speed it blows the air down (vortex ring), and drags back along a sideways
flow (the H-force). These apply to motors marked `lift`, so the T1-S's numbers are untouched.
The values are textbook estimates, not fitted to a real rotor.

## Flying the VTOL

```bash
cd domains/teleoperation/console && cargo run --release -- --station
```

The station starts the SITL container with `AIRCRAFT=t1s-vtol` (so `start.sh` layers
`/t1s-vtol.parm` over the T1-S defaults), starts the sim, and shows the console with the Q modes
among its mode buttons. Arm the plane and press VTOL TKOF to lift it straight up, press CRUISE
for forward flight (the transition is ArduPilot's) and QLOITER to hover again, then QLAND.
With a radio, see the TX15 section of the main README: the arm switch, the mode channel (six
buttons on the radio: QHOVER, QLOITER, FBWA, CRUISE, AUTO, QRTL) and the throttle stick (in the hover modes, the climb rate).

ArduPilot's Tri frame mixes the two wing motors and the rear one, and swings the wing motors
apart to yaw (`Q_TILT_TYPE 2`, vectored yaw). The rear rotor sits twice as far behind the centre
of gravity as the wing motors are ahead of it, as that mixer assumes.

## Still to come for VTOL

- A model for the viewer: the sim window draws the T1-S, with two props, whatever the plane.
- Checking the rotor, tilt and airframe numbers against real flight logs and thrust-stand data.
- Sim-side stick modes for a plane with a flight controller (ANGLE and ACRO exist in the spec,
  but "not supported yet").
