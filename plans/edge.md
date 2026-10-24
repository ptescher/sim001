# Plan: decisions on the drone, for when the radio is blocked

In Towers a held tower blocks the enemy's radio within 140 m. A drone in that shadow gets no
orders, sends no telemetry and no picture. After 20 s its link is lost and it flies home. So the
ground cannot decide for the drone exactly when the fight is hottest. Two things follow: the
ground has to **prepare** the drone before it goes dark, and the drone has to **decide** some
things itself.

## What is done

| Piece | Where |
| --- | --- |
| The rule: a *standing order* goes with an order, and the drone flies it when it has been out of contact for `after` s. With none, the failsafe (home at 20 s) stays | `recreation_towers::Instruction`, spec in `expectations/09-towers/rules/09-standing-orders.expect` |
| The server flies it, and a seat may send one with an order | `towers_server` `World::command_standing`, `ToServer::Order { standing }` |
| The fast arena now plays the radio rules: an order to a cut drone is lost, the failsafe flies home, standing orders are flown, and `TowersResult::dark` counts drone-seconds out of contact | `cognition_towers::Match` |
| An `Autonomy` chooses the standing order for each order, `Prepared` puts one on any captain, and `compare` plays two policies on paired seeds, on both sides | `cognition_towers::autonomy`, `cargo run --release -p cognition_towers --example denial` |

### First result (100 seeds, 200 games, against the failsafe)

| Policy | Margin | Won / lost | Drone-seconds dark |
| --- | --- | --- | --- |
| persist (fly the order on) | **+0.74 ± 0.08** | 152 / 48 | 256 (failsafe: 334) |
| hold after 5 s | −0.87 ± 0.08 | 43 / 157 | 466 |
| home after 5 s | −1.17 ± 0.06 | 21 / 179 | 321 |
| home after 30 s | +0.54 ± 0.09 | 139 / 61 | 265 |

An attacker is always in the shadow of the tower it takes, because the shadow (140 m) is wider
than the circle (60 m). The 20 s failsafe is as long as a capture, so it pulls the drone off the
tower just before it wins. Persisting also shortens the dark time, because taking the tower
removes the shadow.

Two consequences:

1. **The baseline is strong.** "Fly on" is what a learned autonomy must beat, and today there is
   little to beat it with: the drone has one choice that matters. A model on the drone earns its
   place only if it knows something the ground does not.
2. **That something is the drone's own eyes.** The team's picture ignores a cut drone, but its
   own camera still sees. Enemy drones round the tower, who holds it, and the capture bar are
   all known on board and unknown on the ground. This is the information a model can use.

## The next steps, in order

1. **Local observations in the arena.** Give a cut drone what its own camera sees
   (`drone_vision`), the standing order it holds, the time since contact, and the last picture
   it was sent (frozen). This is the observation of the edge model, `EdgeObs`, a fixed-length
   vector, written once in Rust and used by the arena, the data export and the executor.
2. **The edge decision**, `trait Edge { fn act(&self, obs: &EdgeObs) -> Order }`, run each
   second on a cut drone, in place of the standing order. Baselines: persist; persist, but
   retreat if outnumbered at the tower; finish the capture, then return. Each is measured with
   `compare`. Do not build a model before a hand-written edge policy shows there is something
   to gain on the local observations.
3. **Plans that carry their successor.** On the real plane the plan is one primitive with a
   `ttl`; on expiry the executor loiters. `Standing` is a second slot: the *next* plan, armed by
   silence. That is a change to `cognition_plan` (the C ABI, version 3: `next` and `arm_after`),
   to the executor (arm, switch, report it as `AG_FALL`), and to the `COMMAND_INT` encoding.
   `cognition_agent`'s `Brain` then returns `Order { plan, then: Option<(Plan, after)> }`, built
   from the same rule: the ground prepares the answer to "what if I cannot reach you".
4. **Let the league tune it.** The captain contest tunes `CommanderTuning`. Give the commander
   `standing_after` and `standing_then` as tunable numbers, and send the standing order over the
   seat's JSON lines (the reply is now a list of `[drone, order]`; add an optional third
   element). Then the ES finds the standing order, not a hand.

## The model, and MLX

Only when step 2 shows headroom.

- **Teacher.** A privileged policy: the arena with full information, or a search over the
  candidate orders by playing the next 30 s from the saved state (`Match` is `Clone`-able with
  small changes). The teacher labels each cut-drone decision with the best order.
- **Student.** A small MLP (2×64, tanh) from `EdgeObs` to a distribution over the five order
  kinds and a tower choice. Same size as the plan's skill network, so the executor's forward
  pass runs it.
- **Data.** The arena writes `(EdgeObs, teacher order, outcome)` as `.npy` or JSON lines, from
  thousands of seeded games with the randomisation of the sim2real plan (delay, wind, shadow
  radius ±20 %, link timeout).
- **Training on the Mac with MLX.** `training/edge/` managed by `uv`, with `mlx` and `numpy`.
  Distillation is supervised, so the loss is cross-entropy to the teacher with a margin on
  value; the network is tiny and trains in seconds on the GPU of an M-series Mac. Larger
  experiments (a sequence model over the last minute, a language model as teacher or critic) use
  `mlx-lm`, with LoRA, on the same machine. Nothing here needs a cluster.
- **Export.** Weights go out as a flat `f32` file with a hash. A parity test checks that the Rust
  forward pass equals the MLX one to 1e-5 on 1000 observations.
- **Gate.** The student must match `compare` against persist on 1000 seeds with a positive
  margin, and keep the same dark time and failsafe count, before it is loaded anywhere.
- **Claude as teacher.** The league already runs `claude -p`. A run that reads the failure
  replays (cut drones that lost a tower) and writes the rules is a cheaper teacher than search,
  and its rules are readable. Keep both: the rules are the baseline the student must beat.

## What this does not settle

- The arena's drones are simpler than the SITL's. A policy that wins in the arena has to win in
  the server's games (six SITL drones) before it counts; the same `compare` over the server's
  headless mode is the gate.
- The radio rules are the game's own numbers (140 m, 20 s). If they change, the first result
  changes with them: the table above is a measurement, and `flying_the_order_on_beats…` is the
  test that says when it stops being true.
