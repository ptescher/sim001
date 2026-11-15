# Plan: commander, units and advisors, each a person or a computer

Towers has three kinds of actor: a **commander** for each team, the **units** (the drones), and
**advisors** that help a commander. The goal is that each of them can be a person or a computer,
in any mix: a team of people against a team of computers, a person commanding computer units, a
computer commanding drones that people fly. And the same actors play in the fast arena, on
ArduPlane SITL, and with real aircraft.

## The three roles

| Role | What it does | A computer | A person |
| --- | --- | --- | --- |
| Unit | Flies one drone. It always runs. It holds a queue of instructions from its commander and flies them, also out of contact | ArduPilot with the AGENT mode and the executor, on the flight controller | Flies by the sticks. The queue is the mission on the pilot's screen; the pilot decides how to fly it |
| Commander | Looks at the team's map and gives each unit instructions. A simple loop that always answers on time | `recreation_towers::Commander`, once a second | The commander app |
| Advisor | Looks at the team's map in its own time and proposes moves. It can be slow | The lookahead, a Claude advisor, later a learned model | (not planned) |

### The unit

- **The basic unit is ArduPilot.** Nothing runs on a drone that does not fit on a HEEWING flight
  controller: the executor, in AGENT mode, inside its envelope. There is no model on board for now.
- **A unit holds a queue of instructions**, not one order and one standing order. Each
  instruction has an order and the condition that starts it (a time out of contact, the order
  before it done, the battery low). In contact, the commander replaces the queue when it likes.
  Out of contact, the unit works down the queue. The standing order of today is a queue of two;
  the failsafe (home) is the end of every queue.
- **A person's unit gets the same queue.** It is shown as a mission (the tower to take, the
  point to hold, what to do if the radio goes quiet). The person flies; the game does not check
  that they obey. The commander still sees the drone: its telemetry comes down like any other
  unit's, so the map shows where it really is, and the commander can go on giving it
  instructions. In a radio shadow the mission on the screen stops changing, as it does for the
  executor.
- **A bigger computer on board stays possible.** The unit's interface is what crosses the
  drone's radio: the queue down, telemetry and sightings up. A single-board computer beside the
  flight controller would sit on the drone's side of that interface, read the drone's own
  sensors (`EdgeObs`), and edit the queue. The `Edge` policies in the arena stay as the
  experiment for it. Nothing in this plan builds it.

### The commander

One path for a person and a computer: a `Command` seat, which is the team's ground station and
builds the team's map from telemetry. Today the computer commander runs inside the server on
the referee's view, and its ground sends the true places of the enemy as traffic; that goes.

### The advisor

An advisor gets the team's map and gives **proposals**: a unit, an instruction, the gain it
expects, and the time until the proposal is stale. It runs beside the commander, not in its loop.
A person sees proposals in the ADVISOR panel and accepts one with a click. A computer commander
takes advice by itself: it takes a proposal whose gain is above a threshold (the lookahead's rule today: more than 2
points). In the fast arena an advisor's delay is counted in game time, so a game stays
reproducible.

## What exists, and what is in the way

| Piece | Where | In the way |
| --- | --- | --- |
| Orders, standing orders, the seat protocol | `recreation_towers::{view, instruction, seat}` | One standing order, not a queue. Seats are per team: no seat for a unit |
| Plan and standing plan on board | `cognition_plan` (ABI 3), `cognition_executor::set_standing`, `COMMAND_INT` 31010 and 31012 | Two slots |
| The computer commander | `recreation_towers::Commander`, in `towers_server`'s main loop | In the server's process, on the referee's view. `Join::Command` is refused for a computer's team. Fixed at start |
| The ground station of a team | `recreation_towers_ground::Ground`, used by the app and the server | The server's uses the true game (`World::situation`) |
| `Captain` in the arena | `cognition_towers` | One object is commander, standing orders and the edge of every drone |
| Advisor | `recreation_towers::Advisor` (rules, text, for a person) and `lookahead` (inside `Commander`) | Synchronous, once a second. Advice is text |
| A person flying | The sim, the TX15 and CRSF, into one SITL's `rc` channels | The sim cannot join Towers: the server owns every drone's physics and radio |
| The light quad | `World::hover_sticks` | Flown by the server, not over the wire |

## Steps

Each step starts with its expectations (`expectations/09-towers`), then the arena, then the
server.

1. **The instruction queue.** `Instruction` becomes a queue with start conditions; `Standing`
   is kept as the two-entry case so the specs of today still hold. Then the wire: more slots in
   `cognition_plan` (ABI 4) and the executor, and `ToServer::Order` carries a queue. The
   commander app can send one (today it always sends `standing: None`). Gate: the `denial`
   table of `plans/edge.md` is unchanged, and a new policy with a three-entry queue (take, then
   the next tower, then home) is measured against persist.

   **Results (the queue in the rules, the arena, the server and the seat protocol).** Not yet
   done: the on-board slots (`cognition_plan` ABI 4, the executor, `COMMAND_INT`) and a queue in
   the commander app. The flight controller still has one standing slot: the ground uploads the
   first `Silent` instruction of a queue as the standing plan, as before; the server flies the
   whole queue for a drone with no wing, and the arena flies it for every drone. A `Standing` is
   a queue of one (`From<Standing> for Queued`), a queue has at most `MAX_QUEUE` = 4
   instructions (a longer one is cut), and `ToServer::Order` has a `queue` beside the old
   `standing` (both default, so an old seat still works). "Done" is told from the drone's own
   camera and position (`order_done` in `edge.rs`): a `Take` is done when the camera sees the
   team's tower held by the team, a `Goto` when the drone is within 35 m, and `Hold`, `Circle`
   and `Home` never.

   The `denial` table (100 seeds, both sides, against the failsafe) is the same, row for row,
   before and after. The new policies are in the last rows, and the last block is against persist:

   | policy | margin | error | won | lost |
   | --- | --- | --- | --- | --- |
   | persist (unchanged) | +0.060 | 0.095 | 103 | 97 |
   | hold after 5 s (unchanged) | -0.441 | 0.095 | 69 | 131 |
   | home after 5 s (unchanged) | -0.807 | 0.083 | 48 | 152 |
   | home after 30 s (unchanged) | -0.272 | 0.095 | 81 | 119 |
   | chain (persist, next tower when done, home), against failsafe | +0.060 | 0.095 | 103 | 97 |
   | chain, against persist | -0.000 | 0.094 | 100 | 100 |
   | timed 20 s, 60 s, against persist | -0.730 | 0.082 | 50 | 150 |
   | timed 30 s, 90 s, against persist | -0.350 | 0.093 | 75 | 125 |

   A queue with a longer plan does not beat persisting in the valley. `chain` plays exactly as
   persist: a drone that takes a tower flips the tower, the shadow of that tower goes, the drone
   is in contact again, and a drone in contact does not take an instruction, so "done" never
   starts the next one (the probe counted no drone past its first instruction). `timed`, which
   waits for the silence instead, pulls drones off their towers and loses. The next question is
   for a plan that has a reason to leave a tower: a battery entry, or an order that goes to a
   point.
2. **Split `Captain`.** In `cognition_towers`: a commander for a team, a unit for each drone
   (the default flies the queue), advisors. `Match` takes a roster. The out-of-process seat
   carries the queue. Gate: the same results on the same seeds.
   **Results (commander, units and a roster).** Done in the arena; advisors are step 4's. The two
   roles are traits in `recreation_towers` (`actors.rs`, no I/O, for the arena and the server):
   `Command { decide(&mut self, &TeamView) -> Result<Vec<Directive>, String> }`, a `Directive` being
   `{ drone, order, queue }`, and `Unit { fly(&mut self, &OnBoard, &mut Instruction) -> Option<Order>;
   restart() }`. `OnBoard` has private fields and methods for what a drone has on board only: its team
   and number, whether it flies, its clock, `silent()` and `in_contact()`, battery, the link timeout
   and bingo level, `obs()` (its `EdgeObs`, worked out when asked) and `done(order)`. The default unit
   is `Autopilot`: `Instruction::tick_with`. `Match::with_units` takes a unit for each of the ten
   drones; `Match::step_with(&mut dyn Command, &mut dyn Command)` takes the commanders; `Roster`
   (`Roster::new(rules, blue, red, |team, k| unit)`, `Roster::of_captains`, `with_unit`) and
   `play_roster` put them together. A unit runs on every step of the arena (ten a second), for every
   drone, in contact or not: the queue has to be worked at that rate for the games to stay the same.
   The edge policies run once a second inside `EdgeUnit`, as before.

   `Captain` is kept: `Directing(captain)` is its `Command` (orders with `queue`), and
   `Captain::unit(team, k)` replaces `Captain::edge` and makes the unit of each drone (default
   `Autopilot`; `Edged` makes an `EdgeUnit` with its own copy of the policy). No `edge` read captain
   state, so nothing was shared: `Edged`'s policy is stateless and cloned (`compare_edge` takes an
   `Rc<dyn Edge>` for that). `Prepared` only changes the queue that goes with an order, so it is on the
   command side. The seat protocol: a reply is a list of `[drone, order]` or `[drone, order, queue]`
   (`Reply`); an old reply is read as before, and `serve_with` serves any `Command`.

   Same results on the same seeds, digit for digit: `denial`, `ruleset v2 200`, `edge 30` and
   `cognition_league judge --game captain --champion timid --challenger default --seeds 24` print the
   same before and after. New tests: a counting unit runs on every step of every drone, in contact
   or not; a roster with different units for the two teams, and for one drone, plays the same game
   twice and not the game of the default units; the old and the new reply; a served and a spawned seat
   with a queue.

3. **One commander path.** The computer commander takes a `Command` seat like a person, with
   its own `Ground` and a map from telemetry. A seat can be given up and taken during a game,
   and the computer takes a seat that a person leaves. Two people can play (phase 4 of
   `plans/towers.md`). Gate: the map of a computer's seat is the map a person's seat gets.
4. **Advisors.** Proposals with a gain and an expiry, an `Advise` seat, a mailbox the commander
   reads. The lookahead moves out of `Commander` and is the first advisor. Then a Claude
   advisor, once every few seconds. The app gets "accept". Gate: a league contest, the commander
   with the advisor against the commander without, on paired seeds.

   **Results (advisors in the rules and the arena; the seat side is not built).** `recreation_towers`
   has `Advise` (`advise(&mut self, view: &TeamView, hand: &Hand) -> Vec<Proposal>`), `Proposal
   { drone, order, queue, gain, made, until, why }` and `Lookahead`, the old lookahead moved out of
   `Commander` (same model, cadence and numbers). `Hand` is what the commander has in hand and the
   map does not show: its orders after its own rules, the drones it may still change and the towers
   each must stay away from; a seat has it, for the seat is the commander. A `Commander` holds
   advisors, each with a delay in game time (`with_advisor(advisor, delay)`), and takes the best
   proposal each decision that is not stale (`until`), gains more than `ADVICE_MARGIN` (2 points, a
   constant: `CommanderTuning` is the league's candidate format and stays as it is) and still fits
   (the drone is free and the tower is not one it gave up, nor, for a proposal made on an earlier map,
   one the team holds now). `Commander::advise` puts proposals in hand from outside, for a person's
   click or a process, and `Roster::with_advisor(team, advisor, delay)` gives a roster's team an
   advisor in the arena. With `with_lookahead(false)` a commander has no advisor and does not think
   ahead; `Commander::new` has the lookahead at delay 0 unless `SIM001_LOOKAHEAD=0`, as before. The
   text `Advisor` is unchanged: its warnings are for a person and carry no order or gain, so it is not
   an `Advise`. Not done: the seat side (proposals over the wire, the app's accept), a queue in the
   lookahead's proposals (it proposes none), the Claude advisor.

   Gate, no drift: `denial`, `ruleset v2 200` and `judge --game captain --champion timid --challenger
   default --seeds 24` are byte-identical before and after, with `SIM001_LOOKAHEAD` unset and with it
   `0`. (`denial` and every policy of `ruleset` but the last line play with the lookahead off, so
   only `judge` and the line "default against default" test it: Blue wins 50 % with it, 68 % without.)

   Gate, what advice is worth (`cargo run --release -p cognition_towers --example advice 400`:
   commander + lookahead advisor at the delay, against a commander with no advisor, 400 seeds, both
   sides, 800 games each; the lookahead's proposals are good for its 45 s horizon):

   | Delay | Margin | Won | Lost |
   | --- | --- | --- | --- |
   | 0 s | -0.47 ± 0.04 | 268 | 532 |
   | 2 s | -0.46 ± 0.04 | 272 | 528 |
   | 5 s | -0.44 ± 0.04 | 276 | 524 |
   | 10 s | -0.28 ± 0.05 | 333 | 467 |
   | 30 s | -0.81 ± 0.04 | 182 | 618 |

   In the arena by the default rules the lookahead is worth less than nothing: the commander that
   thinks ahead loses to the one that does not, at every delay. Lateness up to 5 s costs nothing
   measurable; at 10 s the harm is smaller and at 30 s it is worst. Why was not measured (the
   proposals taken, and how many are stale or no longer fit, are not counted). So the
   lookahead is a plumbing test for advisors, not a gain: it is the model and its enemy guesses that
   need work, before advice from a slower and better advisor can be told from lateness.
5. **A person flies a unit.** A `Unit` seat. The server gives that drone's radio channels to the
   seat: the sim (or a TX15 on CRSF) joins a game as one drone, with the drone's camera and the
   mission on its HUD. The pilot sees only what the drone sees. Take-off is the pilot's.
6. **The quad on the wire**, so that every unit is ArduPilot behind the same radio.
7. **The field.** The open items of `plans/unified-link.md`: the bench, a serial radio per drone,
   the field umpire. Then real and simulated drones in one game.

Results of step 3 (branch `actors-seats`):

- `recreation_towers_ground` now holds the seat client (`seat`), the station (`station`: its own
  `Ground` over the air ports, the map from telemetry) and the computer commander (`commander::run`,
  binary `towers_commander --team blue|red [--host H] [--port P] [--idle]`). The commander app uses
  the same `seat` and `station`.
- The server no longer decides or sends a plan for any team: its in-process `commanders` and its
  truth-based `Ground` are gone. For a computer (or `nobody`) team it starts `commander::run` in a
  thread that connects over loopback as `Join::Compute`; it keeps the light quads' sticks, take-off,
  the referee and the air.
- Handover: a `Join::Command` (person) relieves a `Join::Compute` holder (`Refused("relieved...")`
  and the connection is closed); the server sends `ToSeat::Orders` once after every welcome of a
  seat that commands, and the new commander starts from them. When a person leaves, a computer takes
  the seat after 1 s (`--on-leave computer`, default; `--on-leave wait` does not).
- Gate A: tests in `towers_server/src/seats.rs` (same messages for both seats; a Red scout in the
  open is in the spectator's truth and not on the map built from a seat's messages; takeover; the
  computer takes a freed seat; two people) and `towers_ground/tests/commander.rs` (a commander
  against a fake server gives orders from the umpire's word and sites alone, leaves when relieved).
- Crashes on SITL (`towers_server --speedup 10 --timeout 200`, one game at a time; crashes per game,
  4 games each, batches interleaved with plain `actors-plan`; the server now prints a second line
  for each crash with the time, order, low or high, jammed, silent time and the nearest flying drone):

  | Batch | Path | Crashes per game | Total |
  | --- | --- | --- | --- |
  | 1 | new, first version | 1, 2, 3, 3 | 9 |
  | 1 | actors-plan | 0, 3, 0, 0 | 3 |
  | 2 | new + AGENT only after the transition, clock cap in wall time | 3, 0, 2, 4 | 9 |
  | 2 | actors-plan | 1, 1, 0, 5 | 7 |
  | 3 | new + dead reckoning of the drones' reports | 1, 1, 0, 0 | 2 |
  | 3 | actors-plan | 4, 7, 4, 2 | 17 |

  Plain `actors-plan` also crashes 0 to 7 drones per game here (12 games: 27 crashes), more in a batch
  that ran while the machine was busy, so the acceptance test's "No drone crashes" is not a gate
  that either path passes reliably. What was a defect of the new path: (1) the station asked for
  AGENT as soon as a drone was in FBWA, in the middle of the transition (the server waits for 15 m/s):
  two games lost a drone at its base at t = 10 s, "Hard impact" at 22 m/s, the other drone 10 m
  away; fixed, no such crash in the 8 games after. (2) The station's clock was held to one game
  second past the last map, so at 10x (a map each 2.5 game seconds) plans went out about 0.8 a
  second instead of 1; now the cap is a wall second. (3) Plans were made from the position of the
  last report; the station now carries it on by its velocity. In all 24 games the only crashes with another
  drone within 30 m were the two take-off ones above (10 to 11 m) and one in plain `actors-plan` (24 m), so the executors' loss of the enemy's true positions as traffic does not show in
  the crashes; they are tree strikes while taking a tower (6 to 25 m), in the open or in a shadow,
  in both paths.
- Not done: the Orders echo is the server's `told` (what the drones last heard), not read from the
  drones' telemetry; no GUI check of a takeover in the app; no full-SITL expectation for a takeover
  (the arduplane driver only plays a computer-against-computer game).

Steps 1 and 3 do not depend on each other. Step 2 needs 1; step 4 needs 3; step 5 needs 1 and 3.

## Not settled

- **How many entries a queue has on board**, and which start conditions the executor knows. The
  flight controller's memory and the `COMMAND_INT` encoding decide it. Four entries and three
  conditions (silence, done, battery) is the first guess.
- **What "done" means** for an order on board: a unit out of contact knows who holds a tower
  only by its own camera (`expectations/09-towers/rules/14-on-board.expect`), and the executor
  has no camera. A time on station may have to stand in for it.
- **The single-board computer.** Which board, how it joins the flight controller (a MAVLink
  serial port is the plain choice), and what sensors it has. Not before the queue shows what a
  drone still cannot do alone.
