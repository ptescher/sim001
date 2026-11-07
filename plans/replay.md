# Plan: replay, rewind and domain reproduction

You should be able to rewind a game or a flight to any moment, watch it again, or take over
from there. The same thing makes a bug report something anyone can replay, and makes a test
start cheap: it is restored, not rebuilt.

## The model

A run is a **snapshot** to start from and a **tape** of the inputs, each at the tick it
happened at: the sticks, the keys, the ground station's commands, the orders of a game of
Towers. The state at any tick is the nearest snapshot before it, with the tape played on from
there. Snapshots are taken every few seconds, so a rewind replays a few seconds at most.

- **Rewind** restores a snapshot and replays the tape up to the moment you picked.
- **Branch** is the same, and then new inputs instead of the tape's.
- **Reproduce** is a tape and its first snapshot in a file: anyone replays it, and it becomes a
  scenario that fails until the bug is fixed.

All of it needs **determinism**: the same state and the same inputs give the same next state.

## Where things stand

| Part | Snapshot | Deterministic | Notes |
| --- | --- | --- | --- |
| Flight physics (`simulation_physics::FlightState`) | plain data, `Clone` | yes: steps by the `dt` it is given, no random numbers | wind and turbulence are functions of sim time |
| The sim's games (`recreation_games::Session`) | `Clone` | yes | the computer pilots fly from the same state |
| Towers rules (`recreation_towers::Game`) | `Clone`, `PartialEq` | yes: `step(dt)` | the server adds ArduPlane SITL drones |
| The sim (`SimCore`) | not `Clone`: it holds the ArduPilot links and the test hook | to be proved | its domain state is the parts above, the sticks, modes and timers |
| ArduPlane SITL | none: a separate process | mostly, in lockstep; the ground station's MAVLink arrives in wall time | see phase 3 |

## The link to expectation

A snapshot is a named state: what `Driver::checkpoint` and `restore` give the runner
(ptescher/expectation#67), so the tests that share a start restore it instead of building it.
A tape is events at times, which is what a scenario is.

## Phases

1. **The sim's core.** Tests first:
   - the same inputs from the same state give the same state, to the bit;
   - a snapshot taken mid-run, restored, and stepped with the same inputs ends where the run
     did;
   - a tape recorded during a run, replayed from its first snapshot, ends where the run did.

   Then `SimCore::snapshot` / `restore`, a tape of the inputs, snapshots every few seconds in
   a ring, and a rewind key in the sim. The sim's acceptance driver gets `checkpoint` and
   `restore` from it.
2. **Towers.** The game server records the orders and its ticks; the commander app gets a
   timeline to scrub and to take over from.
3. **ArduPilot.** The sim and SITL meet at one place, the lockstep bridge: each frame SITL sends
   the servo outputs and the sim answers with the sensors. Recorded, a flight replays without
   ArduPilot (rewind to watch). Taking over from a rewound moment starts a fresh ArduPilot
   there (it flies; its estimator's history is new). Exact re-simulation from boot needs the
   ground station's commands on the tape at their tick, not over TCP in wall time: a spike
   measures how deterministic SITL is in lockstep before anything builds on it.

## Not in scope

The 3D view, sound and the sensor feed are drawn from the state; they are not part of it.
