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
2. **Split `Captain`.** In `cognition_towers`: a commander for a team, a unit for each drone
   (the default flies the queue), advisors. `Match` takes a roster. The out-of-process seat
   carries the queue. Gate: the same results on the same seeds.
3. **One commander path.** The computer commander takes a `Command` seat like a person, with
   its own `Ground` and a map from telemetry. A seat can be given up and taken during a game,
   and the computer takes a seat that a person leaves. Two people can play (phase 4 of
   `plans/towers.md`). Gate: the map of a computer's seat is the map a person's seat gets.
4. **Advisors.** Proposals with a gain and an expiry, an `Advise` seat, a mailbox the commander
   reads. The lookahead moves out of `Commander` and is the first advisor. Then a Claude
   advisor, once every few seconds. The app gets "accept". Gate: a league contest, the commander
   with the advisor against the commander without, on paired seeds.
5. **A person flies a unit.** A `Unit` seat. The server gives that drone's radio channels to the
   seat: the sim (or a TX15 on CRSF) joins a game as one drone, with the drone's camera and the
   mission on its HUD. The pilot sees only what the drone sees. Take-off is the pilot's.
6. **The quad on the wire**, so that every unit is ArduPilot behind the same radio.
7. **The field.** The open items of `plans/unified-link.md`: the bench, a serial radio per drone,
   the field umpire. Then real and simulated drones in one game.

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
