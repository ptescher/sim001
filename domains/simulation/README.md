# Simulation

The flight model and the planes it flies, with no renderer in sight. Anything that needs to
know how a plane moves (the sim's window, the self-tests, the ArduPilot bridge, tools to come)
depends on these crates; they depend on nothing but `glam`.

## Modules

| Crate | Path | Role |
| --- | --- | --- |
| `simulation_aircraft` | [aircraft/](aircraft) | What a plane is to the physics: lifting surfaces, motors, ground-contact points, mass and battery. One module per plane (today `t1s_ranger`, with the geometry its model is drawn from), and a registry: `find("t1s-ranger")`, picked with `--aircraft` |
| `simulation_physics` | [physics/](physics) | The 6-DOF model: strip-theory aerodynamics, propulsion, battery, ground contact, the hand launch and the carry back after a reset. The ground comes in through the `Terrain` trait, so it runs headless |

`simulation_physics` depends on `simulation_aircraft`, never the other way round.

## Adding a plane

Write a module in `simulation_aircraft` with a `params()` that returns an `AircraftParams`, and
add it to `AIRCRAFT`. `t1s-ranger` is the only plane so far; its numbers are fixed by a
self-test (`t1s_sim --selftest`), whose output must not change when the physics is moved or
refactored.

## Still tied to one plane

- `ActuatorCommands` is the T1-S's: an aileron, two V-tail panels and two throttles.
- The sim's model, camera mount and flight modes are the T1-S's. A second plane needs its own
  model, and a more general command type.
