# Recreation

Games and trials to fly against computer pilots, and Towers, a team game flown by plan. Each computer pilot flies a T1-S on the same
flight model as yours (`simulation_physics`), in the same wind, over the same ground and trees.
It moves the same controls a pilot's hands move (aileron, elevator, rudder, throttle), so it can
stall, tip-stall and crash like you. Nothing here draws anything; the sim shows the games (see
`src/games.rs`, and `domains/visualization/engine/src/model.rs` for the planes).

In the sim, **G** starts a pylon race and **T** a game of tag. The same key again stops the game.

## Modules

| Crate | Path | Role |
| --- | --- | --- |
| `recreation_racing` | [racing/](racing) | The pylon race: the leagues' course and its referee (laps, cuts, places, times). Depends only on `glam` |
| `recreation_tag` | [tag/](tag) | Tag: the arena and its referee (who is it, touches, no tag-backs, leaving the arena, the clock). Depends only on `glam` |
| `recreation_pilot` | [pilot/](pilot) | The computer pilot: hands that hold a bank and a climb rate, and steering to a point, along a line or onto another plane. Its [`Skill`](pilot/src/lib.rs) sets how hard it turns and how fast it flies |
| `recreation_towers` | [towers/](towers) | Towers, a team game for drones flown by plan: the towers and their circles, capture, points, the end of the game, the valley (forest, lanes, lake) and what each team sees through the fog and the trees. Depends only on `glam`. See [plans/towers.md](../../plans/towers.md) |
| `recreation_towers_server` | [towers_server/](towers_server) | The game server of Towers (binary `towers_server`): an ArduPlane SITL in AGENT mode for each drone, the flight model (with the trees), the referee, and a seat for each team over TCP |
| `recreation_towers_view` | [towers_view/](towers_view) | The 3D view of a game of Towers (binary `towers_view`): the valley, the six drones, and the camera of each drone on the sensor feed of the sim (TCP 5601 to 5606) |
| `recreation_games` | [games/](games) | A game in play: the countdown, the computer pilots (Kestrel the ace, Merlin the club pilot, Swift the rookie) and what they decide to do, their planes on the flight model, and the referee. Also a stand-in that flies your plane, which the acceptance tests use |

`recreation_games` depends on racing, tag and pilot, and on the simulation domain; the others don't
depend on each other.

## Games with ArduPilot

When the station flies your plane (`gcs --station`), the computer pilots fly planes like yours,
each with an ArduPlane SITL of its own, and the computer pilot flies it through a radio, as you
do with the TX15. So the start-up time is the same for everyone.

- The station starts four SITL containers: `sim001-sitl` (yours, instance 0) and `sim001-sitl-1` to
  `sim001-sitl-3` for Kestrel, Merlin and Swift. SITL instance N sends its physics to UDP port
  9002 + 10·N, and the sim has a bridge for each (`src/ardupilot.rs`). Between games their planes
  stand on the field beside the pilot box with their autopilots running.
- A game starts from cold: every plane on the ground, disarmed, and the countdown starts at once
  (it waits while a plane is being set down at its place). Nobody arms before GO: the sim holds
  your arm switch off until then.
- At GO, each computer pilot puts its arm switch on with the throttle down, as ArduPilot needs. If
  the props do not turn, it takes the switch off and tries again. Once they turn, it climbs in
  QHOVER with the throttle stick above the middle. At 15 m it pushes the FBWA button and takes the
  wing over, and then it flies the game.
- In FBWA the roll stick asks for a bank (all the way over is `ROLL_LIMIT_DEG`, 50° on the T1-S)
  and the pitch stick for a pitch (up to `PTCH_LIM_MAX_DEG`). `Pilot::with_radio` turns the pilot's
  bank and climb rate into those sticks, and the racing lines are 1.5 times as wide, because the
  autopilot limits the bank. The limits come from `ardupilot/t1s.parm` and `t1s-vtol.parm`.
- `Session::radio(i)` says what computer pilot `i`'s radio sends. The sim calls it after each step
  of that pilot's plane, which its autopilot flies (`SimCore::step_opponent`).

Without ArduPilot (the keyboard sim, flying the plain T1-S), the computer pilots fly their planes
direct on the flight model, as before.

## How a game runs

The sim owns your plane and steps its physics. Every physics step it hands the `Session` your
plane and the world around it (`Surroundings`: the ground, the trees and the wind), and the
session steps the computer pilots' planes with `simulation_physics::step` and lets the referee
judge everyone. Player 0 is you; the computer pilots are players 1 to 3.

Without ArduPilot, a game starts with every plane in its pilot's hand and a 5 s countdown. Nobody can throw before
GO. At GO the computer pilots throw, one each second, as the leagues release their models; you
throw when you are ready, and the game clock runs from GO.

## The pylon race

The course is the triangle of FAI F3D and F5D pylon racing: three 5 m pylons, 400 m round,
with two pylons 40 m apart at one end and the third 180 m from each of them. It is laid out in
front of the pilot box, with the twin pylons on the left, so you see the whole course side on.
Laps go counterclockwise with left turns: pylon 1 (the single one), pylon 2 (the far twin),
pylon 3 (the near twin), then the start line, which crosses the near leg 30 m past pylon 3. A
plane must go round pylon 1 before the others count, so a throw from the pilot box starts the
first lap wherever it joins the course.

The referee judges each pylon the way the leagues' judges sight along it: crossing the line out
from the pylon (halfway between its two legs) rounds it; crossing it on the inside, within 50 m
of the pylon, is a cut. A cut counts as passing the pylon, and adds 10 % to the racer's time; a
second cut disqualifies. A crash, or picking your plane up after GO, takes you out.

The race is five laps, not the leagues' ten: at full throttle a T1-S on its 1800 mAh pack lasts
about three minutes. The computer pilots race at 10 m, above the pylons, each on its own line
round them: the ace 30 m out, the club pilot 34 m and the rookie 40 m, at full, 92 % and 85 %
throttle. They finish in about two minutes.

## Tag

You are it first. A plane that is it tags another by touching it: their centres within 1.5 m,
about two wingspans. The tagged plane is it, and it can't tag back the plane that tagged it for
5 s. The game is played in an arena 90 m round the middle of the field in front of the pilot
box, below 60 m; small cones mark its edge, and a plane that flies out of it is it. A plane on
the ground or in a hand can't be tagged. The game lasts two minutes, and the pilot who was it
for the least time wins.

A computer pilot that is it chases the plane it can catch soonest (near, slow and ahead of it),
heads for where it will be along the curve it is flying, and closes on it at a little more than
its speed. The others wander round the arena, break away when the chaser comes within the
distance their skill sees it, and turn back from the edge in time. A computer pilot that crashes
gets its plane back after 3 s and throws again.

## The computer pilot

`Pilot::hold` is the hands: a bank angle and a climb rate. The elevator is fed forward from the
lift the wing needs (`ELEVATOR_LEVEL + ELEVATOR_FOR_LIFT · n / v²`, measured on the flight model
by the `trim_by_bank` probe), and a slow trim learns the rest. The hands keep the plane flying:
less bank when slow, an eased pull and bank at the first buffet (where a tip would stall), more
throttle when the speed bleeds away in a turn, and wings level and up near the ground.

On top of that, `toward` steers for a point (L1 guidance), `follow` flies along a line (the
line's curvature fed forward, and its distance and drift from the line fed back), and
`intercept` closes on another plane. In a steep bank the elevator mostly tightens the turn, so
the pilot asks less of it for height; without that, height and turn swing against each other.

## Towers

Two teams of three drones fight for five radio towers. A drone counts for a tower while it flies
within 60 m of it, below 120 m. The tower's control (−1 Red to +1 Blue) moves 1/20 a second for
each drone one team has there more than the other; a team holds the tower at its end of the bar
and loses it at the middle. Each held tower scores a point a second; 500 points, or the most
after 8 minutes, wins. Each team sees 150 m round its drones and base and 200 m round its towers,
and its map (`Game::maps`) keeps what it last saw of the rest.

`recreation_towers::Game` is only the referee. `recreation_towers::Commander` is the computer
commander: once a second it sends one drone to each tower its team does not hold (nearest the
base first, each to the drone nearest it), keeps a drone on its tower until the team holds it, and
first defends a held tower when it sees an enemy drone near it. It knows only its team's map.

### The rules (v2), and a replay you can open

Three kinds of tower: forts (the homes) block the enemy's radio 160 m round them, the spire (the
centre) 200 m, and gives two points a second, and the depots (the flanks) 120 m and recharge a
drone. A drone has a battery (150, 240 and 300 s for light, standard and heavy); an empty one is
down for 100 s. An empty tower circle drifts back 1 % a second. A drone in a shadow is in contact
if a friend in contact is within 70 m. A link is lost after 45 s, or a drone flies home at 60 s
of battery, unless the commander gave it a standing order. The full rules are in
[plans/towers-rules-v2.md](../../plans/towers-rules-v2.md), what the arena says of them in
[plans/towers-rules-v2-results.md](../../plans/towers-rules-v2-results.md).

To watch a game by these rules, in the fast arena and in a minute:

```bash
cargo run --release -p cognition_towers --example replay 7 persist failsafe towers-replay.html
open towers-replay.html
```

`replay SEED BLUE RED OUT` plays one game (BLUE and RED are what a drone does in a radio shadow:
`failsafe`, `persist` or `surface`) and writes a page that replays it: the towers by kind with
their shadows, the control bars, each drone's battery, the drones out of contact and the ones that
are down. [docs/towers-v2-replay.html](../../docs/towers-v2-replay.html) is one such game. The
round robin of policies is `cargo run --release -p cognition_towers --example ruleset v2 200`.

The game server below runs the drones with no batteries for now (it cannot put a drone down yet);
the kinds, the shadows, the drift, the mesh and the new timings are in it, and the commander app
draws the shadows and the kinds.

### The game server

```bash
SIM001_BIN=.ardupilot-agent/build/sitl/bin cargo run --release -p recreation_towers_server -- --speedup 10
```

`towers_server` starts six ArduPlane SITL drones with the AGENT mode (instances 30 to 35, system
ids 1 to 6, `ardupilot/towers.parm` for the field's fence and ceiling) and is their physics host,
as the runner is. At GO each drone's radio takes it off (`recreation_games::Preflight`: arm,
QHOVER to 15 m, FBWA); once it flies on the wing, the ground switches it to AGENT and sends the
plan for its order about once a second (a tower: an orbit 40 m out, at a height of its own) and the
other drones as `ADSB_VEHICLE` traffic. A crashed drone is back at its base after 15 s.

Each team has a seat on TCP port 5955 (`recreation_towers::seat`, a JSON message a line): a
computer commander, or a person (`--blue person`, `--red person`; GO waits for them). A seat that
commands or watches a team gets only that team's map, four times a second; a spectator gets the
whole game. At the end the server prints `RESULT` lines (the outcome, drones in AGENT, refused
commands, crashes, towers each team took).

The rules and the commander are specified in [expectations/09-towers](../../expectations/09-towers)
and run with no window by `cargo test --release -p recreation_towers`; a whole game with
ArduPilot, with the fog checked from a seat against a spectator, by
`cargo test --release -p recreation_towers_server` (about two minutes; "not supported yet"
without the AGENT build). The commander app is still to come: see
[plans/towers.md](../../plans/towers.md).

## Tests

```bash
cargo test --release -p recreation_racing -p recreation_tag -p recreation_pilot -p recreation_games -p recreation_towers -p recreation_towers_server
```

Every test of the pilot and the games flies the real flight model: a steep turn without losing
height, a tight circle, a throw and climb-out, closing on a turning plane, five clean laps for
each computer pilot in the order of its skill (also in a gusty breeze), a stand-in lap and cut,
tags both ways, and two minutes of tag without a crash. The behaviour in the sim is specified in
[expectations/01-sim/06-recreation](../../expectations/01-sim/06-recreation) and run with
`cargo run --release -- --expect`.

## Still to come

- More games and trials: a time trial against your own best lap, spot landing, limbo under a
  ribbon, a figure-eight trial.
- Planes touching in a race (a mid-air) don't collide; they pass through each other.
- The course and the arena are laid out from the pilot box, whatever the wind. The leagues lay
  the course out so the single pylon is upwind.
- At a real place (`--map`), the course and arena can overlap trees or buildings, which the
  computer pilots don't avoid.
