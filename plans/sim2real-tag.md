# Plan: sim2real AI control of planes, through a game of tag

Planes that play tag against each other, each controlled by AI. A **ground control agent**
decides the plan. **Custom code in ArduPilot** flies the plan on board. Both are trained in
the sim by self-play, with rewards for winning. The same code then flies a real T1-S, with no
changes.

"Mostly complete" means the whole chain runs end to end and is tested at every step:

1. a fast training environment built on the sim's flight model;
2. multi-agent self-play training with game rewards;
3. a plan protocol between the ground and the plane;
4. an ArduPlane fork with an `AGENT` flight mode that runs the trained policy inside a safety envelope;
5. evaluation in real ArduPilot SITL, in lockstep with the sim;
6. system identification from real flight logs, back into the sim;
7. a ladder of real flights, ending with real planes playing tag (with a virtual touch, never a real one).

## What we already have

| Need | Already in the repo |
| --- | --- |
| Physics with no renderer | `simulation_physics::step`: strip theory, 6-DOF, wind, ground and trees. It depends only on `glam` and runs at 400–600 Hz |
| The game | `recreation_tag`: a pure referee (arena, touch, no tag-backs, out of bounds, clock, time as it) |
| Baseline opponents | `recreation_games`: Kestrel, Merlin and Swift, with `chase` and `wander` decisions, and `recreation_pilot` (L1 steering, intercept) |
| ArduPilot in the loop | `src/ardupilot.rs`: the JSON SITL bridge in lockstep, one SITL instance per plane (port 9002 + 10·N). The opponents already fly through RC channels in FBWA |
| What a plane knows about the others | `communication_*`: Remote ID, the MAVLink network and ADS-B over a narrow downlink, plus the tracker. Positions are realistically late, lost and stale |
| Ground station | `teleoperation_console`: MAVLink (the `mavlink` 0.19 crate), tactical map, missions, games over the feed |
| Link to the real plane | `aviation_*`: CRSF/ELRS with MAVLink carried in it, and the TX15 with the SIM001FC widget |
| Seeing a real plane | `visualization_telemetry` / `sim001_view`: a real plane drawn from its telemetry |
| Way of working | Expectations (`expectations/`) → acceptance tests → docs, with the `tdd` and `writing-specs` skills |

Gaps: the SITL bridge and `SimCore` live in the Bevy binary. ArduPilot is built from `master`,
not a pinned version. There is no learning code, no plan protocol and no custom flight mode.

## Architecture

Three layers. Each one runs at the rate its link and its compute allow.

```text
 GROUND (station)                       LINK (ELRS/CRSF, ~2.6 kB/s,       PLANE (flight controller, ArduPlane fork)
                                        drops, 3 s staleness)
 ┌───────────────────────────┐                                         ┌────────────────────────────────────────┐
 │ Ground control agent  1 Hz │ ── plan (COMMAND_INT, MAV_CMD_USER_n) ─▶│ MODE_AGENT                       50 Hz │
 │  sees: tracker's tracks,  │                                         │  plan + onboard tracks (AP_ADSB)       │
 │   game clock, roles,      │ ◀── AGENT_STATUS (NAMED_VALUE_*, ──────│  → tactical policy (small MLP)         │
 │   own telemetry           │      STATUSTEXT, policy hash)           │  → safety envelope (fence, floor,      │
 │  brain: scripted | RL |   │                                         │     separation, bank, plan timeout)    │
 │   Claude (optional)       │ ── opponent positions (ADSB_VEHICLE) ─▶│  → nav_roll / nav_pitch / throttle     │
 └───────────────────────────┘                                         │  → stock attitude controllers, TECS    │
                                                                       └────────────────────────────────────────┘
```

- **Ground control agent** (strategy, about 1 Hz). It chooses whom to chase, when to break
  away, where to go in the arena, which height band to use and how to spend the battery. It
  works from the tracker, so it sees what a real station sees. It sends a **plan**: one
  primitive with its parameters and an expiry time.
- **MODE_AGENT** (tactics, 25–50 Hz, on board). It flies the primitive: it closes on a
  target, breaks away from a threat, flies to a point or holds a band. A small trained network
  outputs a demanded bank, pitch and throttle. It runs on board because the link is too slow
  and too lossy for close manoeuvring, and because the plane has to fly safely when the link
  drops.
- **Stock ArduPilot** (stabilisation, 400 Hz). The policy acts on attitude demands, as FBWA
  does, and never moves the control surfaces directly. ArduPilot's tuned controllers absorb
  most of the difference between the sim's aerodynamics and the real plane's. This is the most
  important sim2real choice in the plan.

Take-off and landing stay in stock modes. The ground agent sequences them: VTOL take-off
(QHOVER/QLOITER) → transition → AGENT → QRTL or QLAND. The pilot's TX15 mode switch always
wins: any mode change by the pilot leaves AGENT at once.

### The plan protocol

Version 1 adds no new MAVLink dialect. Neither ArduPilot's mavlink submodule nor the
console's `mavlink` crate has to change.

| Message | Use |
| --- | --- |
| `COMMAND_INT` with `MAV_CMD_USER_1` (31010) | Plan: `param1` primitive (PURSUE, EVADE, GOTO, ORBIT, BAND, LOITER), `param2` target system ID or threat ID, `param3` plan ID, `param4` expiry (s), `x/y/z` a point or band (lat, lon, alt) |
| `COMMAND_INT` with `MAV_CMD_USER_2` | Game state: role (it / not it), no-tag-back target, time left, arena centre and radius as a mission fence |
| `ADSB_VEHICLE` | Opponents' positions from the ground's tracker, for when the plane's own receivers can't hear them. AP_ADSB already takes these in |
| `NAMED_VALUE_FLOAT/INT` + `STATUSTEXT` | Agent status: active plan ID, primitive, policy hash, envelope interventions, the reason for falling back |
| `COMMAND_ACK` | Plan accepted or rejected (a reason in `result_param2`) |

The vocabulary and its encoding go in a new pure crate, `autonomy_plan`. The ground agent,
the console, the training environment and the plane all use the same definitions.

### MODE_AGENT, the custom ArduPilot code

- **A fork pinned to a release tag** (ArduPlane 4.6.x), kept as a patch series in
  `ardupilot/patches/` or a fork on GitHub as a submodule. `build_native.sh` and the
  Dockerfile build the pinned fork, not `master`.
- **A new Plane mode** (a free mode number, for example 50, `AGENT`), modelled on
  FBWA/GUIDED: `update()` sets `nav_roll_cd`, `nav_pitch_cd` and the throttle, and the stock
  `stabilize()` does the rest.
- **On-board tracks** come from the AP_ADSB vehicle list. On the real plane that list is fed
  by the receivers and the uplinked `ADSB_VEHICLE`. In SITL the sim feeds it, through the
  communication domain's `Air`, so the plane only knows what its radios would hear.
- **The policy** is a small MLP (for example 2×64, tanh, about 6 k weights; about 10 k
  multiply-adds at 50 Hz, nothing for an H7). Its weights are loaded from the SD card, with a
  hash that the plane reports to the ground.
- **The safety envelope** comes after the policy and can't be learned around: the geofence,
  a height floor, a bank limit, minimum separation from every track (both horizontal and
  vertical) and a stall margin on airspeed. An expired plan or a lost link → LOITER, then
  RTL/QRTL. A NaN or an out-of-range output → FBWA wings level. Every intervention is counted
  and reported.
- **One implementation, not two.** Recommended: write the executor (primitive logic,
  observation building, the MLP forward pass and the envelope) once, as a `no_std` Rust crate
  (`autonomy_executor`) with a C ABI, and link it into the mode. The training environment
  calls the same crate, so there is no drift between the C++ and Rust copies. The risk is
  linking a Rust static library into ArduPilot's waf/ChibiOS build. Phase 0 tries this out;
  if it fails, use a C++ port with shared golden test vectors.

## The training environments: a fidelity ladder

| Level | What runs | Speed | Use |
| --- | --- | --- | --- |
| **F0, fast** | `simulation_physics` + **ap-lite** (a Rust model of FBWA: ArduPilot's roll and pitch controllers and its throttle path, from the parameter file) + `recreation_tag` + `communication_air` for observations. Vectorised over many games with rayon | Target ≥1000× real time per core (to be measured in Phase 0) | Most of the RL training |
| **F1, SITL** | The same physics + **real ArduPlane SITL** (the fork with MODE_AGENT) per plane, in lockstep and faster than real time, headless | About 5–20× real time per plane | Validation, fine-tuning, regression tests. The exact code that will fly |
| **F2, bench** | The fork built for the real flight controller, on the bench with the props off: plan handling, CPU load, timing, failsafes | Real time | Before each real flight |
| **F3, field** | Real planes | Real time | The flight ladder (below) |

ap-lite is calibrated against F1 with step-response tests (bank and pitch steps, throttle
steps, a coordinated turn). Each of those is an expectation, so the gap between F0 and F1 is
measured, not assumed.

## Tag as a learning problem

**Agents.** Four players. Each player is a ground agent + MODE_AGENT pair. All players share
one set of weights, conditioned on their role (it / not it). The scripted computer pilots
stay in the opponent pool as fixed baselines.

**On-board policy (25–50 Hz).**
- Observation, relative to the plane and scaled: airspeed, roll and pitch, body rates, height
  above the ground and climb rate, the distance to the arena edge and the ceiling, the
  battery; the plan (a one-hot primitive and its target's relative position and velocity,
  with the track's age); the nearest threat; the role and no-tag-back flags. The previous
  action.
- Action: demanded bank and pitch (within `ROLL_LIMIT_DEG` / `PTCH_LIM_*`) and throttle. A
  penalty on rate of change keeps it from bang-bang control, which matters on a real plane.

**Ground policy (1 Hz).**
- Observation: every track (position, velocity, age, source), each player's role and time as
  it, the clock, its own plane's energy and battery, the arena.
- Action: a primitive, a target or threat, and parameters (a band or a point), plus an
  expiry.

**Rewards.**

| Event | Reward |
| --- | --- |
| Each second as it | −1/120 (so a full game as it is −1) |
| Tagging someone | +0.2 |
| Being tagged | −0.2 (on top of then being it) |
| Leaving the arena | −0.3, and you are it (the referee's rule) |
| Crashing | −1, and a 3 s respawn (respawning costs time as it) |
| Envelope intervention | −0.01 each (it learns to stay within the envelope instead of leaning on it) |
| End of the game | Placing: +1 for the winner (least time as it), then +0.3, −0.3, −1 |
| Shaping (faded out over training) | Chaser: closing rate on the target. Evader: separation from the chaser |

**Training order (hierarchical).**
1. **Skills.** Train the on-board policy to fly each primitive well against scripted targets,
   with dense tracking rewards: pursue a turning plane, break away, hold a band, fly to a point.
2. **Strategy.** Freeze the skills. Train the ground policy by self-play, on the game rewards
   alone.
3. **Together.** Fine-tune both on the game rewards, with a KL limit on the skills so they
   don't drift.

**Self-play.** MAPPO with a centralised critic (the critic sees the truth; the actors see
only what their radios tell them). A league of frozen snapshots and the scripted pilots,
sampled by prioritised fictitious self-play, so it doesn't collapse into one narrow
strategy. TrueSkill ratings for every snapshot. Every N updates, gate on F1 SITL win rates.

**Training stack.** Recommended: the environment and inference in Rust; PPO in Python
(PyTorch), through a PyO3 binding (`autonomy_gym`, built with maturin, with a
PettingZoo-style parallel API), in a separate `training/` folder managed by `uv`. The weights
are exported to the plane's file format, and a parity test checks the Rust forward pass
against PyTorch. A training stack in pure Rust (burn or candle) would stay all-Rust, but it
would make the RL research slower.

## Sim2real

1. **Act on attitude, not on surfaces** (above). This does the most to close the gap.
2. **Observations come through the radios.** Training sees opponents only through
   `communication_air` → tracker, with real rates, losses, latency and staleness, never their
   true positions. The real plane has the same view.
3. **Domain randomisation**, per game: mass and CG (±10 %), motor thrust and pack sag (±15 %),
   lift and drag scaling per surface (±15 %), servo rate and delay, wind (0–8 m/s) and
   turbulence, GPS and airspeed noise and delay, plan delay (0.2–2 s) and plan loss
   (0–30 %), and the ArduPilot gains in ap-lite (±20 %). The ranges are narrowed by system
   identification.
4. **System identification (real to sim).** A new crate, `simulation_sysid`, replays the
   servo outputs logged in a real DataFlash log through the physics, compares the
   trajectories and fits a chosen set of `AircraftParams` (CMA-ES). The fitted values and
   their spread update the aircraft module and the randomisation ranges. The README notes
   that today's VTOL numbers are estimates. This is how they become measurements.
5. **Same code in sim and air.** The executor crate and the weights file are bit-identical in
   F0, F1, F2 and F3. The policy hash is in the telemetry and the logs.
6. **Gates, each written as an expectation:** the ap-lite step responses within tolerance of
   SITL; the F1 win rate within 10 points of F0; the error on replayed real logs below a
   threshold; no envelope breaches in 1000 F1 games.

### Tag in the real world

Planes never touch. The real-world rules are a variant of the referee:

- **A virtual touch:** a horizontal distance under 15 m counts as a tag, whatever the height.
- **Height bands:** each plane has its own band, at least 15 m apart, enforced by the
  envelope. Training for the real variant uses the same rules.
- An arena fence, a ceiling under the legal limit, a visual observer and a safety pilot for
  each plane, with the TX15 mode switch as the kill switch.

`recreation_tag` gets these as a `Rules` variant, so the sim and the field use the same referee.

## Where the code goes

A new domain, `domains/autonomy/`:

| Crate | Role | Depends on |
| --- | --- | --- |
| `autonomy_plan` | The plan vocabulary, its MAVLink encoding, and the status | nothing |
| `autonomy_executor` | `no_std` + C ABI: observation, the MLP forward pass, the primitives, the safety envelope | `autonomy_plan` |
| `autonomy_aplite` | The Rust model of ArduPlane's FBWA path | `simulation_*` |
| `autonomy_env` | The vectorised multi-agent tag environment (F0), and the headless SITL environment (F1) | the above, `simulation_*`, `recreation_tag`, `communication_air`/`traffic` |
| `autonomy_gym` | The PyO3 binding | `autonomy_env` |
| `autonomy_agent` | The ground control agent: its brains (scripted, learned, Claude) and a MAVLink endpoint. The console embeds it, and it also runs on its own | `autonomy_plan`, `communication_traffic` |

Elsewhere:
- `simulation_sitl` (new): the JSON bridge and the multi-instance SITL runner, moved out of
  `src/ardupilot.rs` and `src/sitl.rs` so they run without Bevy.
- `simulation_sysid` (new): log replay and parameter fitting.
- `recreation_tag`: the real-world rules variant.
- `ardupilot/`: the pinned fork, the patches, the MODE_AGENT build, and `t1s-agent.parm`.
- `training/` (Python): MAPPO, the league, export, and reports.
- `expectations/07-autonomy/`: the specs, which generate the manual's chapter.

## Phases

Each phase starts with its expectations (the `tdd` skill) and ends with them passing and the
docs regenerated.

### Phase 0: Foundations (de-risking)

**Status: mostly done.** Done: `simulation_sitl` (the bridge, lockstep and launcher, with the sim rewired onto it); ArduPilot pinned to `Plane-4.7.2` with a patch series (`ardupilot/patches/`); the AGENT mode (number 50) builds and is entered in SITL; `autonomy_executor` links into it, and SITL logs `Agent: executor ABI 1`, so the Rust-in-ArduPilot approach works on SITL; the executor cross-compiles for Cortex-M7 (`thumbv7em-none-eabihf`); the physics runs at about 8 000× real time on one core. Also done: `autonomy_runner` plays a headless tag game with three real SITL planes at 9.5× real time (the fourth player, you, is not in it). Not done: linking into a ChibiOS board build (needs the Arm GNU toolchain, which is not installed here); the console entering and leaving AGENT; the existing `.ardupilot` build is still the old `master` one until rebuilt with `ardupilot/build_native.sh`.
- Pin ArduPilot to a release tag. Build the fork with a stub `AGENT` mode that flies wings
  level, in SITL.
- Spike: link a `no_std` Rust static library into the SITL build, and cross-compile it for one
  real board target (for example MatekH743). Decide between Rust and a C++ port.
- Move the JSON bridge and the SITL runner into `simulation_sitl`. Run 4 SITLs + physics
  headless, faster than real time.
- Measure the physics' throughput (steps per second per core) and set the F0 budget.
- **Done when:** a headless binary plays a scripted game of tag with 4 SITL planes at ≥5×
  real time, and the stub mode can be entered and left from the console.

### Phase 1: The plan protocol and a scripted loop, end to end

**Status: the loop runs; tuning is open.** Done: `autonomy_plan` (plan, `COMMAND_INT` 31010/31011, C ABI v2); `autonomy_executor` (six primitives, the envelope, its own maths, 34 tests with closed loops and soaks, builds for Cortex-M7); the AGENT mode takes plans, `ADSB_VEHICLE` tracks and `AGENT_*` limits, and reports `AG_*` status (patches 0001 and 0002); `autonomy_agent` (scripted brain, MAVLink link); `autonomy_runner --agent` flies three SITL planes by plan at 9.4× real time with every command accepted. Open: in the first full run the envelope intervened on about half the steps and the executor kept falling back to Recovery (the planes fly at 13–19 m, near the 12 m floor and 13 m airspeed floor), so the planes tag little (1 real tag in the run); the console's plan display and override; the envelope expectations (specs); tracks reach the executor but nothing reports their use.
- `autonomy_plan` and its encoding. Classical primitives in `autonomy_executor` (pursuit by
  the existing L1 and intercept logic, break-away, goto, band) and the full safety envelope.
- MODE_AGENT runs the executor. The sim feeds `ADSB_VEHICLE` into each SITL from `Air`.
- `autonomy_agent` with a scripted brain: the existing Kestrel/Merlin/Swift decisions, now
  sending plans.
- The console shows each plane's plan and status, and lets the operator override a plan.
- **Done when:** four ArduPilot planes play tag in F1, every one of them in AGENT mode under a
  scripted ground agent. Envelope tests (fence, floor, separation, plan expiry, link loss)
  pass as expectations.

### Phase 2: The fast environment
- `autonomy_aplite`, calibrated against SITL step responses (these are expectations).
- `autonomy_env`: vectorised, randomised, with observations through the radios. The F0 and F1
  backends behind one API. `autonomy_gym` bindings.
- **Done when:** a random policy and the scripted brain run in F0 at the budget, and the
  scripted brain's results in F0 and F1 match within tolerance.

### Phase 3: Learned skills
- `training/`: PPO for each primitive (skills). Export weights, and the parity test between
  Rust and PyTorch.
- Load the skills into MODE_AGENT and evaluate them in F1 against the classical primitives.
- **Done when:** the learned pursuit closes on Kestrel faster than the classical one in F1,
  with no more envelope interventions.

### Phase 4: Self-play tag
- Train the ground policy (MAPPO, league, TrueSkill), then both together.
- Evaluate against Kestrel, Merlin and Swift and against earlier snapshots, in F1.
- In the sim and the console: press T and play tag against the trained agents (your plane
  against three AI planes), with a per-player panel showing each agent's current plan.
- **Done when:** the trained agents beat the scripted pilots in more than 70 % of F1 games,
  and there are no envelope breaches in 1000 games.

### Phase 5: Real to sim, and hardening
- `simulation_sysid` on logs from real manual flights. Update the T1-S parameters and the
  randomisation ranges. Retrain or fine-tune.
- The F2 bench checklist, automated where it can be (a script over MAVLink).
- The real-world rules variant, trained and evaluated.
- **Done when:** the error on replayed real logs is under threshold, and the F2 bench suite
  passes on the real board.

### Phase 6: The flight ladder
Each step gates the next. Each one keeps a safety pilot on the TX15.
1. Manual and FBWA flights with high-rate logging (the data for sysid).
2. AGENT with the classical goto, orbit and band primitives.
3. AGENT pursuing a **virtual** target, injected as `ADSB_VEHICLE` from the ground.
4. **Mixed reality:** one real plane against three simulated opponents. The sim flies them
   from the real plane's telemetry, and they reach the real plane as traffic. The console
   (and `sim001_view`) shows them together.
5. Two real planes, in their bands, with virtual touches; then more.

### Phase 7 (optional): Claude as commander
A third brain for `autonomy_agent`. Claude gets the tracks and the game state as tools, and
issues plans as tool calls, at about 0.2–0.5 Hz. It keeps a short log of its reasons, which
the console shows, so the operator can read why each plane does what it does. It is good for
plans you can read and for strategies set in plain language ("pin the leader against the
west fence"). It is too slow for close manoeuvring, so the skills on board still do the
flying. It runs only in F1 and in the station, and never in the training loop.

## Risks

| Risk | What we do about it |
| --- | --- |
| The flight model is too far from the real T1-S | Act on attitude; randomise; system identification in Phase 5; mixed reality before two real planes |
| ap-lite doesn't match ArduPilot | Calibrate with step-response expectations; fine-tune in F1 |
| Rust won't link into the ChibiOS build | Spike in Phase 0; fall back to a C++ port with golden vectors |
| Self-play collapses or finds exploits (for example, camping at the ceiling) | A league with the scripted pilots in it; the arena rules; watch the F1 replays |
| The real link is too slow for 1 Hz plans plus the uplinked traffic | Plans are a few dozen bytes; the downlink rate is already modelled in the communication domain; the plane flies on its own receivers and the plan's expiry |
| Two real planes close together | Virtual touches, height bands, separation enforced by the envelope, a safety pilot for each plane |
| ArduPilot changes upstream | Pin the tag; keep the patch series small (one mode, one parameter group) |

## Decisions to make

1. **The ground agent's brain:** a learned policy first and Claude as an optional extra
   (recommended), or Claude as the main strategist from the start.
2. **The training stack:** Python/PyTorch over Rust bindings (recommended), or pure Rust.
3. **The executor:** one `no_std` Rust crate linked into ArduPilot (recommended, if the Phase 0
   spike works), or C++ in the fork with parity tests.
4. **The first real airframe and flight controller:** the T1-S VTOL as in the sim, or a
   Ranger with a flight controller added (hand-launched FBWA is simpler for the first
   flights); and which board.
5. **The real-world tag rules:** a 15 m virtual touch with 15 m height bands, or something
   else.
