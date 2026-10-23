# SIM001 Towers: the commander app

The seat of a person in a game of Towers (see [plans/towers.md](../../../plans/towers.md) and
the "Towers" section of the manual). It is a window of its own, in the look of the console: the
map of your team, the towers, your drones and their orders, the clock and the score.

```bash
cd domains/teleoperation/command
cargo run --release -- --serve                  # you command Blue; the computer commands Red
cargo run --release -- --serve --red nobody     # a game to practise in: nobody commands Red
cargo run --release -- --connect HOST:5955      # the Blue seat of a game someone else runs
cargo run --release -- --team red --watch       # Red's map, with no orders
cargo run --release -- --serve --no-cameras     # no 3D view and no pictures from the drones
```

`--serve` builds the game server (`towers_server`, in the workspace) and the 3D view of the game
(`towers_view`, which sends the drones' cameras), and starts them with six
ArduPlane SITL drones; it needs the AGENT build of ArduPlane (`SIM001_BIN`, else
`.ardupilot-agent/build/sitl/bin`). The server writes to `logs/towers/server.log`, and stops its
drones when the app quits. The game starts (GO) once your seat is taken and ArduPilot is ready,
about a minute and ten seconds of the drones' time; `--speedup N` runs it faster.

## The screen

- **The top bar:** your seat (BLUE COMMANDER), the score of each team, the time left and the
  state of the game (WAITING FOR GO, PLAYING, BLUE WINS).
- **FIELD:** north is up. The dark is the fog: you see 150 m round your drones and your base,
  and 200 m round each tower you hold. Each tower has its circle (60 m), its colour (the team
  that holds it, grey for neutral) and its control bar (right is Blue, left is Red). A tower in
  the fog is dim, with the time since you saw it; a contested circle flashes. Your drones are
  triangles, with a dashed line to where each is sent. Enemy drones show while you see them, and
  dim where you last saw them.
- **DRONES:** each drone, its height (or GROUND, DOWN) and the order the server has for it.
- **TOWERS:** each tower as your team last saw it.
- **CAMERAS:** the picture from the camera of each of your drones (the sensor feed of the sim, from
  the 3D view of the game, ports 5601 and up), small, and the selected drone's larger. Click a
  picture to select its drone. `NO SIGNAL` shows until the 3D view has loaded the valley.

The field shows the valley under the fog: the lake, the three lanes cut through the forest, and the
clearings round the towers and the bases.

## Orders

Select drones: click one in DRONES or on the map, drag a box round several on the map, or press 1,
2 or 3 (Shift adds, Ctrl/Cmd+A selects all, Esc lets go). Then right-click a tower to take it (the
drones circle it 40 m out), or anywhere else on the field to fly there and circle.
HOLD (H) circles where it is, HOME (B) over your base. LOW (L) makes the drone fly low, along the
lanes and below the treetops, where the trees hide it from the enemy (and a drone that leaves the
lane hits a tree); LOW again makes it fly high. The drone's orders after that keep the setting.
Esc lets the drone go. Until you give a
drone an order, it circles over your base (HOME).

## Tests

`cargo run --release -- --expect` runs the app's tests of the "Towers" section
(expectations/09-towers/app/ and its stories): each on a fresh practice game of its own
server at ten times real time, clicking the real buttons and the map.

## Code

- `src/seat.rs`: the seat on the server (TCP, `recreation_towers::seat`), on threads of its own.
- `src/server.rs`: `--serve`, the game server and the 3D view as children that stop with the app.
- `src/cameras.rs`: the pictures from the drones (the sensor feed, one connection for each drone).
- `src/ui.rs`: the window. `src/draw.rs` and `src/theme.rs` are the console's drawing helpers and
  look (copied: the console is not a library).
- `src/expect.rs`: the driver for the acceptance tests.
