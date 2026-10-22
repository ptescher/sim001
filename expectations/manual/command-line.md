# Command-line options

Use this page when you start the programs from a terminal. The launcher ([[The launcher]]) sets these for you.

## The sim (`cargo run --release --bin sim001 -- …`)

| Option | What it does |
| --- | --- |
| `--aircraft t1s-vtol` | The T1-S VTOL with its own ArduPilot. This is the default. |
| `--aircraft t1s-ranger` | The plain T1-S Ranger, with no flight controller |
| `--raytrace` | Ray-traced shadows, global illumination and reflections |
| `--map NAME` | A real place: `cypress` or `berkeley` |
| `--fullscreen` | Start in borderless full screen |
| `--ardupilot` | Use the sim as the physics of an ArduPlane SITL that you start yourself |
| `--direct` | With ArduPilot: your sticks fly the plane through the sim's own modes |
| `--flight-computer PORT` | Read the radio from the flight computer on a serial port. The sim ignores the USB controller. |
| `--feed [PORT]`, `--feed-size WxH` | Stream the sensor camera over TCP (default port 5600) |
| `--window-rect x,y,w,h`, `--title-bar N` | Place the window |
| `--exit-with-parent` | Quit when the program that started the sim quits |
| `--calibrate` | Open the radio wizard at the start |
| `--debug-input` | Print every stick and button change |
| `--selftest` | Print the flight-model numbers, with no window |

## The station (`cd domains/teleoperation/console && cargo run --release -- …`)

| Option | What it does |
| --- | --- |
| `--station` | Start everything: the sim, ArduPilot SITL, the sensor feed and the console |
| `--map NAME` | A real place |
| `--direct` | Start in Direct control |
| `--raytrace` | Run the sim with ray tracing |
| `--layout single` | Tile all windows on the main display |
| `--no-build` | Do not build the sim first |
| `--no-radio` | Connect to ArduPilot directly, with no radio stand-in |
| `--flight-computer` | Use the simulated TX15 on the serial line |
| `--sensor` | Open the sensor feed alone |

## Environment

| Variable | What it does |
| --- | --- |
| `SIM001_TREE_FRAC` | The share of the forest to draw, for example `0.5` |
| `SIM001_HOME`, `SIM001_BIN`, `SIM001_DATA` | Where the maps, the programs and your settings are. The app bundle sets them. |

## Files

| File | What it holds |
| --- | --- |
| `controls.cfg` | Your radio and gamepad mapping. See [[Controls]]. |
| `ardupilot/t1s.parm` | The ArduPilot parameters of the T1-S |
| `logs/sitl/` | The flight logs of ArduPilot (`.BIN`) |
