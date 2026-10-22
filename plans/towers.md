# Plan: Towers, a team game flown from a ground station

Two teams of drones fight for radio towers on a field. Each drone is a T1-S VTOL with its own
ArduPlane SITL (the fork with the AGENT mode), and a team is commanded the way the real station
commands a plane: by plans (`cognition_plan`), not sticks. A team holds a tower by circling it.
Towers it holds score, and they let it see: a team only knows what its towers and its drones
can see, so the rest of the field is in fog.

Each team has a **seat**. A seat is filled by a person at the commander app or by a ground control
agent (a `Brain`), and both use the same seat protocol, so human against computer, human
against human and computer against computer are the same game.

## Rules (version 1)

Numbers to tune, not measurements; they live in `recreation_towers::Rules`.

| Rule | Value |
| --- | --- |
| Field | A valley with a forest, flat ground in a circle 600 m round the centre (the AGENT fence is 620 m). North is up |
| Teams | Blue (south base, 520 m south of the centre) and Red (north base, 520 m north) |
| Drones | 3 a team, taking off from their base |
| Towers | Five: each team's home tower (430 m from the centre, towards its base, held by that team at the start), the centre, and the west and east flanks (about 360 m out, not mirror images: the lake and the lanes are not), neutral at the start |
| In a tower's circle | Flying, within 60 m of the tower across, and at most 120 m up |
| Capture | A tower's control goes from −1 (Red) to +1 (Blue). Each second it moves towards the team with more drones in the circle, by 1/20 for one drone more and 1/20 more for each drone after that. With the same number of each team's drones in the circle (and at least one), it is contested and does not move. With none, it stays |
| Holding | A team holds a tower when its control reaches its end (±1). It loses it when the control crosses 0. A tower between is neutral |
| Score | One point a second for each tower a team holds |
| End | 500 points, or the clock (8 min): the most points wins; equal points is a draw |
| Vision | A held tower sees 200 m round it, a drone 150 m (on the ground too), a base 150 m. The trees hide a drone below the treetops (20 m): it is seen only along open ground (a lane, a clearing, the lake), or from above within 75 m |
| Fog | A team sees an enemy drone only while something of its own sees it. Out of sight, the drone stays on its map where it was last seen, with its age. A tower's holder and control are known only while the team sees the tower; otherwise the team has what it last saw |
| Crash | The drone is out, and it is back at its base after 15 s (phase 2) |

There is no fighting in version 1: a team keeps the other out of a tower by being there too.

## The valley (map)

`recreation_towers::map` is the one map that the referee, the server, the 3D view and the app share:

- A forest covers the field (trees 20 to 30 m tall, one each 10 m or so), cut open by three **lanes**
  60 m wide from home tower to home tower (west through the west tower and along the shore of the
  lake, mid through the centre, east through the east tower), a road from each base to its home
  tower, **clearings** round the towers (80 m) and bases (150 m: a drone leaves the ground, turns
  on the wing and climbs there) and the **lake**, between the Blue home tower and the west tower.
- A drone flies **high** (straight over the forest, 55 to 75 m) or **low** (16 to
  20 m, below the treetops). A low order goes along the lanes (`flight::plan` and `flight::leg`
  turn it into the plans that the server sends) and circles low in a clearing; to a point in the
  forest it goes to the nearest clearing, climbs there and circles high. A drone below the
  treetops that is sent high climbs in its clearing first.
- The flight model meets every tree as an obstacle (a plane against the trees round it, in a
  grid): a low drone that leaves its lane crashes. The 3D view draws the same trees.
- The seat protocol carries `low` with an order, and the drone's `low` in the map. LOW (or `L`) in
  the app toggles it for the selected drone.

## The drones' cameras

`towers_view` (`domains/recreation/towers_view`) is the 3D view of a game: a spectator seat on the
server, the engine's world with the valley, the six drones, and a camera under each drone's nose.
Each camera is served on the sensor feed of the sim (`teleoperation_shared::feed`), drone `i` (Blue's
three, then Red's) on TCP port 5601 + `i`, while someone watches it. The commander app shows its
team's three in CAMERAS. The app starts the view with `--serve`, and it stops with the app.

## Architecture

```text
 SEAT (one per team)                 SERVER (recreation_towers_server)             DRONES
 ┌─────────────────────┐  JSON     ┌───────────────────────────────────┐ MAVLink ┌───────────────────┐
 │ commander app (GPUI)│◀─lines──▶│ seats: the team's view, 4 Hz       │◀──────▶│ ArduPlane SITL ×6 │
 │   or a Brain        │  on TCP  │   orders in → plans out            │  tcp    │  AGENT mode       │
 └─────────────────────┘           │ referee: recreation_towers::Game  │         │  executor         │
                                   │ physics: simulation_physics,      │◀──────▶│                   │
                                   │   lockstep with each SITL          │ UDP JSON│                   │
                                   └───────────────────────────────────┘         └───────────────────┘
```

- **The fog is the host's.** A seat gets only its team's view (`Game::view(team)`), so a seat
  cannot see more than its team does, whatever is in it. A seat never talks to the drones; it
  sends orders, and the host turns each order into a plan for that drone.
- **Orders** are what an operator wants, not what the executor flies: `Capture(tower)` (orbit
  it inside the circle), `Goto(point)`, `Orbit(point, radius)`, `Hold` (loiter where it is),
  `Home` (back over the base). The host keeps each drone's order and re-sends its plan before it
  expires.
- **The host** is the runner (`cognition_runner`) grown up: N SITL instances in lockstep, the
  take-off sequence of `recreation_games` (arm, QHOVER to 15 m, FBWA, then AGENT), the AGENT
  fence and ceiling set for the field, and the referee stepped with the physics.

## Crates

| Crate | Path | Role |
| --- | --- | --- |
| `recreation_towers` | `domains/recreation/towers` | The rules, the referee, vision and fog, the team view, orders, the seat protocol's messages and the scripted brain. Depends on `glam` and `serde`. No I/O |
| `recreation_towers_server` | `domains/recreation/towers_server` | The game server (`towers_server`): SITL fleet, physics, referee, seats over TCP, orders to plans |
| `teleoperation_command` | `domains/teleoperation/command` | The commander app (binary `sim001_command`): the field with its fog, the towers, the roster, select and order. GPUI, outside the workspace like the console, with a copy of its theme and drawing |

## Phases

Done: 1, 2 and 3. The commander app (`domains/teleoperation/command`, `sim001_command --serve`)
takes a person's seat; its tests click it on a fresh practice game of a real server each.
Done: 1 and 2. A whole game between two computer commanders plays on six real ArduPlane SITL
drones at about 6.5 times real time (a 4.5 minute game in about 45 s), and the server's
acceptance test checks the fog from a seat against a spectator.

Settled on the way:

- The SITL instances are 30 to 35 (`--first-instance`), clear of the station's 0 to 3 and of
  TCP 5900 (macOS screen sharing took instance 14's MAVLink port). Seats are on TCP 5955.
- The server looks again just before it sends the maps, so that every map is of the same moment
  as the spectator's game (each drone's physics thread moves it between the referee's steps).
- The commander's "near a held tower" is two circles (120 m), not the tower's sight (200 m): the
  towers are 220 m apart, so with 200 m an enemy circling the next tower pinned a defender.
- Two commanders deadlocked one against one at every tower. A drone that is behind at a
  contested tower for 10 s gives it up and stays away 30 s; who arrived first decides.
- The app keeps the server's input open (`--stop-with-stdin`): when the app quits, or dies, or a
  test starts a fresh game, the server stops its six ArduPlanes. Killing the server itself would
  leave them running.
- A drone's order echoes back in its team's map, so the app shows what the server has. Until its
  seat orders it, a drone circles over its base (HOME). `--red nobody` makes a practice game.
- The app copies the console's drawing helpers and look (`draw.rs`, `theme.rs`): the console is
  a binary, not a library. A shared GPUI crate would end the copy.
- Red won both full games so far (500 to 394, 500 to 409). Red circles 4 m higher; whether that
  or something else favours it is not known yet.

1. **Rules** (`recreation_towers`). The referee, vision and fog, the team view, orders and the
   scripted brain, specified in `expectations/09-towers` and run headless with drones that fly
   straight to their orders (the rules don't depend on how a drone flies).
2. **Host.** Six SITL drones and two brain seats play a whole game headless, faster than real
   time. An acceptance test plays the section's game tests on real ArduPlane (skipped without
   the AGENT build, as the runner's are).
3. **Commander app.** A person takes a seat: the field, fog, towers, score and clock, the
   roster, click a drone and click a tower or a point to order it. Its expect driver clicks as
   the console's does.
4. **Two people.** Seats by name on the host's command line (`--blue app --red brain`, or
   both `app`), with the host on another machine.
5. **Later:** terrain and line of sight for vision, a real place (`--map`), the 3D view of the
   sim showing the towers, tagging enemy drones out, battery as a resource, replays.
