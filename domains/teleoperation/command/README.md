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
cargo run --release -- --serve --no-cameras     # no 3D battlefield and no pictures from the drones
```

`--serve` builds the game server (`towers_server`, in the workspace) and the 3D view of the game
(`towers_view`, which renders the battlefield and sends the drones' cameras), and starts them with six
ArduPlane SITL drones; it needs the AGENT build of ArduPlane (`SIM001_BIN`, else
`.ardupilot-agent/build/sitl/bin`). The server writes to `logs/towers/server.log`, and stops its
drones when the app quits. The game starts (GO) once your seat is taken and ArduPilot is ready,
about a minute and ten seconds of the drones' time; `--speedup N` runs it faster.

## The screen

The window is laid out as a real-time strategy game.

- **The top bar:** your seat (BLUE COMMANDER), a chip for each tower in the colour of the team
  that holds it (it flashes amber while contested), the score of each team with the towers it
  holds, the time left and the state of the game (WAITING FOR GO, PLAYING, BLUE WINS).
- **The battlefield** fills the window: the valley in 3D, from your camera above the field. The
  picture comes from the 3D view of the game (`towers_view`, port 5621 for Blue and 5622 for Red),
  rendered with no drones in it so that it shows nothing your fog hides; the app draws your map on
  it with the same camera. The dark is the fog: you see 150 m round your drones and your base,
  and 200 m round each tower you hold. Each tower is a mast with a light in the colour of its team,
  its circle (60 m) on the ground and its control bar (right is Blue, left is Red). Each drone
  stands on a line from its shadow, so you see how high it flies, with a dashed line on the ground
  to where it is sent. Enemy drones show while you see them, and dim where you last saw them. The
  ADVISOR's words show at the top left. Without the 3D view (`--no-cameras`, or until it has
  loaded the valley) the battlefield is the plain map of the valley in the same perspective.
- **The bottom bar:** the minimap (the whole field, north up, with the area that the camera sees
  and a compass), a card for each drone (its camera, its height or GROUND, DOWN or NO LINK, its
  order, LOW and its battery), the PORTRAIT (the selected drone's camera, large) and UNIT (its
  class and numbers), and the command card.

## The camera

W, A, S, D or the arrow keys move the camera, and so does the mouse at an edge of the battlefield.
Q and E turn it 45°. The mouse wheel takes it nearer to the ground or further. Click or drag on
the minimap to look there. Space looks at the selected drones.

## Orders

Select drones: click one on the battlefield or its card, drag a box round several on the
battlefield, or press 1 to 5 (Shift adds, Ctrl/Cmd+A selects all, Esc lets go). Then right-click
a tower to take it (the drones circle it 40 m out), or anywhere else on the field to fly there and
circle; a green ring marks the place. The command card has HOLD (H), which circles where it is,
HOME (B), over your base, and LOW (L), which makes the drone fly low, along the lanes and below
the treetops, where the trees hide it from the enemy (and a drone that leaves the lane hits a
tree); LOW again makes it fly high. The drone's orders after that keep the setting. ALL, FOCUS
and NONE do what Ctrl/Cmd+A, Space and Esc do. Until you give a drone an order, it circles over
your base (HOME).

## Tests

`cargo run --release -- --expect` runs the app's tests of the "Towers" section
(expectations/09-towers/app/ and its stories): each on a fresh practice game of its own
server at ten times real time, clicking the real buttons and the map.

## Code

- `src/seat.rs`: the seat on the server (TCP, `recreation_towers::seat`), on threads of its own.
- `src/server.rs`: `--serve`, the game server and the 3D view as children that stop with the app.
- `src/cameras.rs`: the pictures from the drones (the sensor feed, one connection for each drone).
- `src/battlefield.rs`: the picture of the battlefield from the 3D view, and the camera the app asks for.
- `src/lens.rs`: where the field is on screen, through the camera and on the minimap.
- `src/field.rs`: what the app draws on the battlefield and the minimap.
- `src/ui.rs`: the window, its bars and the controls. `src/draw.rs` and `src/theme.rs` are the console's drawing helpers and
  look (copied: the console is not a library).
- `src/expect.rs`: the driver for the acceptance tests.
