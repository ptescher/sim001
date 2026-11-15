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

   **Why the lookahead lost (branch `actors-lookahead`; `cargo run --release -p cognition_towers
   --example lookahead -- calibrate|variants SEEDS ...`).** The cause is a defect in the model: it had
   no flight. A drone in the model counted in a tower's circle as soon as it was within 60 m; in the
   arena it flies at its orbit height (55 to 87 m) and comes down at 3 m/s to 16 m only within 60 m of
   the tower, and only a drone below the treetops (20 m) counts (`cognition_towers` `Flyer::step`,
   `Game::in_circle`): 12 to 22 s of nothing at every arrival. A move that leaves a drone that is
   already low and capturing, to a tower where it must come down again, was therefore always a
   gain in the model and a loss in the game. Measured on the old model (40 seeds, both sides, the
   real game replayed from the seed with and without each taken proposal, nothing else proposed for
   45 s): 45 proposals made and 25.5 taken per game (a move every 8 s); promised +17.6 points on
   average, realised -1.9 (points of margin after 45 s).

   | Gain promised | Taken | Promised | Realised points |
   | --- | --- | --- | --- |
   | 2 to 5 | 359 | +3.3 | -3.0 ± 0.6 |
   | 5 to 10 | 473 | +7.2 | -2.1 ± 0.5 |
   | 10 to 20 | 533 | +14.5 | -2.4 ± 0.5 |
   | 20 to 40 | 487 | +27.7 | -1.2 ± 0.4 |
   | over 40 | 185 | +53.9 | +0.9 ± 0.3 |

   By what the drone was doing: flying to a tower (1574 taken) promised +18.4, realised +0.1; capturing a
   tower (204) promised +10.9, realised **-16.5 ± 1.3**; guarding a held tower (259) +17.9, -2.0. The
   harm is the moves of drones that sit low in a circle. With the flight in the model (the
   default now) the same table: 44.5 made and 21.5 taken per game, promised +9.9, realised -0.3 ± 0.2;
   capturing -6.4 ± 1.5 (86 taken), flying +0.3 (1401), guarding -1.6 (229). Still too many moves,
   and still a loss for the drones that are capturing: not understood to the end (see below).

   Where the old model differs from the arena, rule by rule (old `lookahead.rs` lines):
   - Flight, climb, descent (:104, :110-119; arena `lib.rs:229-257`, `Game::in_circle` `lib.rs:678`):
     no height. 12 to 22 s per arrival. The biggest by far. **Fixed.**
   - Take-off (arena `lib.rs:40-42, 230-239`): a drone on the ground at the start counted in a circle
     as soon as it was near. 5 s plus 2 s a drone. **Fixed** (`grounded`).
   - Charging (`commander.rs:370-371`: the hand's orders are those before the charge orders): a drone on
     its way home counted at its tower. **Fixed**; worth +0.03 ± 0.04 (not measurable alone).
   - Drift of an empty circle (`lib.rs:697-700`, 100 s a bar): the model kept every bar where it was, so
     leaving a half-won tower cost nothing. Modelled (`Model::drift`), **but measured worse** (see below),
     so it is off.
   - What a drone does after it has won its tower (`commander.rs:288-361`, it goes on to the next):
     the model left it there. Modelled (`Model::redeploy`), **measured worse**, so it is off.
   - Not modelled, size unknown: the speed (16 m/s in the model; 0.95-1.05 of it plus a wind of up to
     5 m/s in the arena, `lib.rs:45, 335, 349`) and the orbit of 40 m round the tower; the order delay
     of 0.3 to 2 s (`lib.rs:51, 389`); radio shadows (`lib.rs:564`: a drone sent into the shadow of a
     tower is out of contact, and its orders do not reach it, `lib.rs:395-403`); batteries and the
     contest drain (`lib.rs:614`, only the `lasts` check); the give-up rule (`commander.rs:226-238`: the
     commander takes a drone off a tower it is behind at after 10 s, and the model keeps it there); the
     score cap at 600 (`lib.rs:725`); a tower the team has not seen for a while (its control is the
     old one); an enemy that goes down, charges or fights.
   - The enemy in the model flies straight to the tower of a guess and stays; its altitude is a guess
     (low at the tower, high elsewhere). With the enemy where it really is (an oracle, below) the model
     gains little: the belief is not what holds the lookahead back.

   Each variant against a commander with no advisor, 400 seeds, both sides (800 games), delay 0, the
   flight model (`variants 400 taking flight`; the rows with `first` are the model as it was):

   | Variant | Margin | Won | Lost |
   | --- | --- | --- | --- |
   | the model as it was (`first`) | -0.47 ± 0.04 | 268 | 532 |
   | flight | +0.02 ± 0.04 | 420 | 379 |
   | flight + drift | -0.08 ± 0.04 | 389 | 411 |
   | flight + charging (**the default now**) | +0.05 ± 0.04 | 430 | 370 |
   | flight + redeploy | -0.10 ± 0.04 | 380 | 419 |
   | flight + drift + charging | -0.04 ± 0.04 | 404 | 396 |
   | flight + drift + charging + redeploy | -0.18 ± 0.04 | 355 | 444 |
   | on the flight model: margin 5 | +0.18 ± 0.04 | 466 | 333 |
   | margin 10 | +0.12 ± 0.05 | 445 | 355 |
   | margin 20 | +0.12 ± 0.05 | 439 | 361 |
   | margin 40 | +0.02 ± 0.05 | 404 | 396 |
   | not a drone that is capturing | -0.03 ± 0.04 | 411 | 388 |
   | not capturing, margin 10 | +0.13 ± 0.05 | 446 | 354 |
   | a drone moved is left alone 10 s | +0.00 ± 0.04 | 416 | 383 |
   | 20 s | -0.17 ± 0.04 | 357 | 442 |
   | 40 s | -0.51 ± 0.04 | 265 | 534 |
   | blind (belief never updated) | -0.03 ± 0.04 | 410 | 389 |
   | oracle (enemy drones where they are) | +0.05 ± 0.04 | 433 | 366 |
   | only drones with no order | +0.00 ± 0.05 | 400 | 400 |

   ("Only drones with no order" is no advisor: every drone has an order.) On the old model the same rows
   (100 seeds, ±0.09): margin 5 -0.48, 10 -0.54, 20 -0.13, 40 +0.07; not capturing -0.12; blind
   -0.41; oracle -0.57. Horizon and futures on the model with all four rules on (100 seeds): 15 s
   +0.04, 30 s +0.04, 60 s -0.27, 90 s -0.29; 2 futures -0.25, 32 futures -0.18 (8: -0.18).

   What it comes to. The model's missing flight was the defect: with it the lookahead goes from -0.47
   to +0.05 ± 0.04 (430 won, 370 lost), which is level with no advisor, not a gain. A larger margin
   (5 points) wins by +0.18 ± 0.04; that is a number found by looking at the result, and is not in the
   code (`ADVICE_MARGIN` is 2 as before). Not fixed, and for the owner: whether the default commander
   keeps the lookahead. With the fix `judge --game captain --champion timid --challenger default
   --seeds 24` gives the default commander +0.384 (lower bound +0.048, promoted) with the lookahead,
   +0.268 (+0.031) with `SIM001_LOOKAHEAD=0`, and +0.175 (-0.132, kept out) with the lookahead as it was.
   Late advice now costs, as it should for a plan that is exact in time (`advice 400`):

   | Delay | Margin | Won | Lost |
   | --- | --- | --- | --- |
   | 0 s | +0.05 ± 0.04 | 430 | 370 |
   | 2 s | -0.16 ± 0.04 | 362 | 438 |
   | 5 s | -0.27 ± 0.04 | 331 | 469 |
   | 10 s | -0.55 ± 0.04 | 248 | 552 |
   | 30 s | -0.94 ± 0.04 | 143 | 657 |

   The old table was flat because the proposals were noise. Unexplained: why the moves of capturing
   drones still lose (-6.4 ± 1.5 a move with the flight model); why drift and redeploy, which are the
   referee's and the commander's own rules, measure worse than leaving them out (about 2 sigma each, not
   checked at more seeds); why 20 s and 40 s of cooldown per drone are so much worse than 10 s.
   **Results (the seat side: advisors beside the commander, a process advisor, a Claude advisor,
   and the app's accept).** Done in `recreation_towers_ground::{advisors, claude}`, the commander
   app and the server. An advisor at a seat runs on a thread of its own (`advisors::Runner`) behind
   a mailbox: the newest map and hand go in, an older one that was not looked at is dropped, never
   queued, and proposals come out when they are ready (`Advisors::take(now)` drops the ones that are
   stale on arrival). A computer commander on a seat (`commander::run_with`) has its built-in
   lookahead off and never calls an advisor in `decide`: before each decision it puts what has
   arrived in hand (`Commander::advise`), after it it shows the advisors the map and
   `Commander::hand()` (new: the orders after its rules, the drones it may change, the towers each
   keeps away from). `Advise` is now `Send`. Tests: an advisor that sleeps 3 s does not delay ten
   decisions (the slowest takes under 100 ms, the advisor looked at one map); a late proposal is
   dropped once stale; the mailbox keeps the newest of five maps (looks at the first and the last);
   an advisor that panics is dropped; and, through the commander on a fake server, a process that
   never answers does not slow its orders, while one that proposes gets its proposal taken.
   `ProcessAdvisor` (`sh -c COMMAND`, `serve` for the other end, binary `towers_advisor`): one JSON
   line in (`{"Start":{"team","rules"}}` once, answered `"ok"`; then `{"Look":{"view","hand"}}`),
   one out (a list of proposals). A process that dies, hangs (90 s), writes what is not a list or
   proposes for a drone or tower that is not in the game is dropped with a line, and the game goes
   on. The Claude advisor (`towers_advisor --claude`, `SIM001_CLAUDE` for the command) is tested only
   against stand-in commands (good, malformed, error, slow, no command): **it has not been run
   against the real `claude`**. Flags: `towers_commander --advisor`, `towers_server --blue-advisor`
   and `--red-advisor`, the app's `--advisor` (`none` by default under `--expect`). The app lists
   the live proposals in the ADVISOR panel (best first, seconds left, a click or Y takes it; the
   text warnings stay) and `expectations/09-towers/05-advice.expect` covers it.

   On SITL (the server's acceptance test, the lookahead as a station advisor on both sides, two
   games at 10x): both games play to the end and both teams take towers (Blue 1 and Red 2; Blue 2
   and Red 1). The server's log shows the proposals arriving and the commanders taking some: 91
   proposals and 61 taken in the first game, 86 and 44 in the second (`RESULT proposals`,
   `proposals_taken`; "taken" counts a proposal whose drone and order the commander's decision then
   had, so one that matched its own order anyway is counted too). Failing: "No drone crashes" (7 and
   3), the known one, and in the first game "the commands that the drones refused are 0" (1 refused;
   0 in the second game, and not seen again: not looked into). The app's expectations ran in the
   real window (`--expect --only proposal --no-cameras`): the four tests of 05-advice pass; the
   empty-panel check inside the story "Your first tower" was not run (it needs the cameras).
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
