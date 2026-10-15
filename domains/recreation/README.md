# Recreation

Games and trials to fly against computer pilots. Each computer pilot flies a T1-S on the same
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
| `recreation_games` | [games/](games) | A game in play: the countdown, the computer pilots (Kestrel the ace, Merlin the club pilot, Swift the rookie) and what they decide to do, their planes on the flight model, and the referee. Also a stand-in that flies your plane, which the acceptance tests use |

`recreation_games` depends on the other three and on the simulation domain; the others don't
depend on each other.

## How a game runs

The sim owns your plane and steps its physics. Every physics step it hands the `Session` your
plane and the world around it (`Surroundings`: the ground, the trees and the wind), and the
session steps the computer pilots' planes with `simulation_physics::step` and lets the referee
judge everyone. Player 0 is you; the computer pilots are players 1 to 3.

A game starts with every plane in its pilot's hand and a 5 s countdown. Nobody can throw before
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

## Tests

```bash
cargo test --release -p recreation_racing -p recreation_tag -p recreation_pilot -p recreation_games
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
