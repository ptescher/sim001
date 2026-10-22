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
```

`--serve` builds the game server (`towers_server`, in the workspace) and starts it with six
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

## Orders

Select a drone: click it in DRONES or on the map, or press 1, 2 or 3. Then click a tower to take
it (the drone circles it 40 m out), or anywhere else on the field to fly there and circle.
HOLD (H) circles where it is, HOME (B) over your base. Esc lets the drone go. Until you give a
drone an order, it circles over your base (HOME).

## Tests

`cargo run --release -- --expect` runs the app's tests of the "Towers" section
(expectations/09-towers/app/ and its stories): each on a fresh practice game of its own
server at ten times real time, clicking the real buttons and the map.

## Code

- `src/seat.rs`: the seat on the server (TCP, `recreation_towers::seat`), on threads of its own.
- `src/server.rs`: `--serve`, the game server as a child that stops with the app.
- `src/ui.rs`: the window. `src/draw.rs` and `src/theme.rs` are the console's drawing helpers and
  look (copied: the console is not a library).
- `src/expect.rs`: the driver for the acceptance tests.
